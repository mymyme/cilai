// Feedback reports contain failures only. Keep local lifecycle records for
// troubleshooting, but do not send polling, successful checks or raw log text.
const { safeId, safeRunId, failureTime } = require("./ai-error-diagnostics");
const { diagnosticCategories } = require("./diagnostic-categories");
const failureEvents = new Set(["command_error", "service_error", "stderr_error", "ai_run_failure"]);
const failureStatuses = new Set(["error", "failed", "repairRequired", "notConfigured", "updateRequired"]);
const healthyStatuses = new Set(["ready", "running", "starting", "stopped", "notApplicable"]);
const sandboxCodes = new Set(["sandbox_refresh_failed", "workspace_acl_access_denied", "runtime_acl_failed", "sandbox_deny_acl_failed", "sandbox_helper_unknown_error", "runtime_directory_owner_mismatch"]);
const startupScopes = { managedPath: "受管路径", applicationData: "应用数据目录", environment: "环境目录", profiles: "账户目录", profile: "目标账户目录", loginCredentials: "登录凭据目录", loginSession: "登录会话文件", instance: "实例文件", startupDirectory: "启动子目录", startupFile: "启动元数据文件", runtimeDirectory: "运行环境目录", runtimeDatabase: "运行记录文件权限" };
const startupCodes = new Set(["startup_path_validation_failed", "private_file_owner_mismatch", "private_file_validation_failed", "startup_repair_journal_unavailable"]);
const repairPhases = { prepared: "准备完成", compare: "修改前核对", write: "写入权限", verify: "修改后复查" };
const securityParts = { owner: "所有者", group: "所属组", daclFlags: "权限继承标志", daclEntries: "访问规则", representation: "描述格式" };
const daclFlags = { protected: "P", autoInherited: "AI", autoInheritRequested: "AR" };

function repairErrors(context) {
  const errors = [];
  for (const attempt of Array.isArray(context?.repairAttempts) ? context.repairAttempts.slice(0, 3) : []) {
    if (!attempt || !Object.hasOwn(repairPhases, attempt.phase)) continue;
    const error = { source: "startup_repair", phase: attempt.phase, codes: [
      attempt.osError === 10017 ? "startup_repair_permissions_changed" : "startup_repair_incomplete",
    ] };
    if (safeId(attempt.operationId)) error.requestId = safeId(attempt.operationId);
    if (Object.hasOwn(startupScopes, attempt.targetScope)) error.scope = attempt.targetScope;
    for (const key of ["targetIndex", "previousAppliedIndex"]) {
      if (Number.isInteger(attempt[key]) && attempt[key] >= 0 && attempt[key] <= 128) error[key] = attempt[key];
    }
    if (Array.isArray(attempt.changedParts)) error.changedParts = [...new Set(attempt.changedParts.filter(part => Object.hasOwn(securityParts, part)))];
    for (const key of ["daclFlagsBefore", "daclFlagsAfter"]) {
      if (Array.isArray(attempt[key])) error[key] = [...new Set(attempt[key].filter(flag => Object.hasOwn(daclFlags, flag)))];
    }
    if (Number.isInteger(attempt.osError)) error.osError = attempt.osError;
    if (typeof attempt.rollbackVerified === "boolean") error.rollbackVerified = attempt.rollbackVerified;
    if (/^[a-f0-9]{64}$/.test(attempt.componentSha256)) error.componentSha256 = attempt.componentSha256;
    if (Number.isSafeInteger(attempt.atEpochSeconds) && attempt.atEpochSeconds > 0 && attempt.atEpochSeconds < 253402300800) {
      error.at = new Date(attempt.atEpochSeconds * 1000).toISOString();
    }
    errors.push(error);
  }
  return errors;
}

function isError(event) {
  if (failureEvents.has(event.event)) return true;
  if (event.event === "service_exit") return Number.isInteger(event.exitCode) && event.exitCode !== 0;
  if (failureStatuses.has(event.status)) return true;
  // A running status may still carry the stderr tail from an earlier failure.
  if (healthyStatuses.has(event.status)) return false;
  return event.codes?.length > 0;
}

function timestamp(value) {
  return typeof value === "string" && value.length <= 40
    && /^\d{4}-\d{2}-\d{2}[T ][\d:.]+(?:Z|[+-]\d{2}:\d{2})?$/.test(value)
    && Number.isFinite(Date.parse(value)) ? value : null;
}

function aggregate(records) {
  const groups = new Map();
  for (const { at, durationMs, completedAt, ...fields } of records) {
    if (fields.codes) fields.codes = [...new Set(fields.codes)].sort();
    const key = JSON.stringify(fields);
    let group = groups.get(key);
    if (!group) {
      group = { ...fields, count: 0 };
      groups.set(key, group);
    }
    group.count += 1;
    const time = fields.event === "ai_run_failure" ? failureTime(completedAt) : timestamp(at);
    if (time) {
      if (!group.firstAt || Date.parse(time) < Date.parse(group.firstAt)) group.firstAt = time;
      if (!group.lastAt || Date.parse(time) > Date.parse(group.lastAt)) group.lastAt = time;
    }
    if (Number.isFinite(durationMs)) group.maxDurationMs = Math.max(group.maxDurationMs ?? 0, durationMs);
  }
  return [...groups.values()];
}

function runtimeErrors(runtime) {
  if (!runtime) return [];
  if (runtime.status !== "collected") {
    return [{ source: "runtime_diagnostics", codes: [runtime.status === "timedOut" ? "diagnostics_timed_out" : "diagnostics_unavailable"] }];
  }
  const errors = [];
  const startup = runtime.startup;
  const context = startup?.status === "collected" ? startup.context : null;
  const elevation = typeof context?.processElevated === "boolean" ? { processElevated: context.processElevated } : {};
  if (runtime.profileStatus === "unavailable") errors.push({ source: "runtime_diagnostics", codes: ["startup_profile_unavailable"], ...elevation });
  if (startup?.status === "unavailable") errors.push({ source: "runtime_diagnostics", codes: ["startup_diagnostics_unavailable"] });
  for (const entry of Array.isArray(context?.errors) ? context.errors : []) {
    if (!startupCodes.has(entry?.code)) continue;
    const error = { source: "startup_permissions", codes: [entry.code], ...elevation };
    if (Object.hasOwn(startupScopes, entry.scope)) error.scope = entry.scope;
    if (["administrators", "otherAccount"].includes(entry.ownerCategory)) error.ownerCategory = entry.ownerCategory;
    if (Number.isInteger(entry.osError)) error.osError = entry.osError;
    errors.push(error);
  }
  errors.push(...repairErrors(context));
  const instance = runtime.instance;
  if (["failed", "unreadable"].includes(instance?.privateFileValidation)) {
    const error = { source: "instance", codes: ["private_file_validation_failed"] };
    if (instance.ownerMismatch === true) error.codes.push("private_file_owner_mismatch");
    if (Number.isInteger(instance.osError)) error.osError = instance.osError;
    errors.push(error);
  }
  for (const log of Array.isArray(runtime.sandbox) ? runtime.sandbox : []) {
    // Native collection already uses fixed names; apply the same boundary here
    // so a future host change cannot export arbitrary paths or messages.
    const source = /^(?:windows-sandbox-(?:readiness|setup)\.stderr\.log|sandbox\.\d{4}-\d{2}-\d{2}\.log)$/.test(log?.source)
      ? log.source : "sandbox_log";
    if (log?.status === "unreadable") {
      const error = { source, codes: ["diagnostic_log_unreadable"] };
      if (Number.isInteger(log.osError)) error.osError = log.osError;
      errors.push(error);
    }
    if (log?.status !== "read") continue;
    for (const entry of Array.isArray(log.errors) ? log.errors : []) {
      const codes = Array.isArray(entry?.codes) ? entry.codes.filter(code => sandboxCodes.has(code)) : [];
      if (codes.length) errors.push({ source, at: entry.at, codes });
    }
  }
  return errors;
}

function errorsOnlyReport(report, runtime) {
  const { events = [], product, exportedAt, system, logging, privacy } = report;
  return {
    schemaVersion: 2,
    product, exportedAt, system, logging, privacy,
    exportMode: "errors-only",
    errors: aggregate([
      ...events.filter(isError).map(event => ({ source: "workbench", ...event })),
      ...runtimeErrors(runtime),
    ]),
  };
}

const eventLabels = {
  command_error: "宿主命令", service_error: "服务错误", stderr_error: "服务错误输出",
  service_start: "服务状态", service_exit: "服务退出", sandbox_status: "沙箱状态",
  permission_check: "项目权限检查", permission_repair: "项目权限修复",
  ai_run_failure: "对话执行失败",
};
const sourceLabels = { workbench: "工作台", instance: "实例文件校验", runtime_diagnostics: "诊断采集", startup_permissions: "启动数据权限", startup_repair: "启动权限修复", sandbox_log: "沙箱日志" };
function formatErrorReport(report) {
  const system = report.system ?? {};
  const errors = report.errors ?? [];
  const lines = [
    `词来工作台错误日志（格式 ${report.schemaVersion}）`,
    `导出时间：${report.exportedAt}`,
    `工作台：${system.version ?? "未知"}；系统：${system.platform ?? "未知"} ${system.osRelease ?? ""} ${system.arch ?? ""}；Electron：${system.electron ?? "未知"}`,
    `记录开关：${report.logging?.enabled ? "开启" : "关闭（仅导出已有记录与本次检查错误）"}`,
    "仅包含错误类型、来源、时间、运行/请求编号、记录次数及修复阶段摘要，不含会话正文、文件内容、请求参数或登录凭据。",
  ];
  if (report.logging?.error) lines.push(`日志功能错误：${report.logging.error}`);
  lines.push("");
  if (!errors.length) lines.push(report.logging?.error ? "未发现其他已记录的错误。" : "未发现已记录的错误。");
  else {
    lines.push(`共 ${errors.length} 项错误，合并前 ${errors.reduce((sum, error) => sum + error.count, 0)} 条记录（含历史记录；次数为日志条数）。`);
    for (const error of errors) {
      const fields = [sourceLabels[error.source] ?? error.source];
      if (error.event) fields.push(eventLabels[error.event] ?? error.event);
      if (error.command) fields.push(error.command);
      if (error.status) fields.push(`状态 ${error.status}`);
      if (Object.hasOwn(startupScopes, error.scope)) fields.push(startupScopes[error.scope]);
      if (typeof error.processElevated === "boolean") fields.push(`进程权限 ${error.processElevated ? "管理员" : "普通用户"}`);
      if (error.ownerCategory === "administrators") fields.push("所有者 管理员组");
      if (error.ownerCategory === "otherAccount") fields.push("所有者 其他账户");
      fields.push(error.codes?.length ? error.codes.join(", ") : "未分类错误");
      const categories = diagnosticCategories(error.codes);
      if (categories.length) fields.push(`分类 ${categories.join("、")}`);
      if (Number.isInteger(error.osError)) fields.push(`系统错误 ${error.osError}`);
      if (Number.isInteger(error.exitCode)) fields.push(`退出码 ${error.exitCode}`);
      fields.push(`${error.count} 条`);
      if (error.firstAt) fields.push(error.firstAt === error.lastAt ? `时间 ${error.firstAt}` : `首次 ${error.firstAt}；末次 ${error.lastAt}`);
      if (error.maxDurationMs > 0) fields.push(`最长耗时 ${error.maxDurationMs} ms`);
      if (safeId(error.requestId)) fields.push(`请求编号 ${safeId(error.requestId)}`);
      if (safeRunId(error.runId)) fields.push(`运行编号 ${safeRunId(error.runId)}`);
      if (Object.hasOwn(repairPhases, error.phase)) fields.push(`阶段 ${repairPhases[error.phase]}`);
      if (Number.isInteger(error.targetIndex)) fields.push(`检查对象 第 ${error.targetIndex + 1} 项`);
      if (Number.isInteger(error.previousAppliedIndex)) fields.push(`上次完成 第 ${error.previousAppliedIndex + 1} 项`);
      if (error.changedParts?.length) fields.push(`变化 ${error.changedParts.map(part => securityParts[part]).join("、")}`);
      if (error.changedParts?.includes("daclFlags") && error.daclFlagsBefore && error.daclFlagsAfter) {
        fields.push(`继承标志 ${error.daclFlagsBefore.map(flag => daclFlags[flag]).join("+") || "无"} → ${error.daclFlagsAfter.map(flag => daclFlags[flag]).join("+") || "无"}`);
      }
      if (typeof error.rollbackVerified === "boolean") fields.push(error.rollbackVerified ? "原权限回读一致" : "原权限回读未通过");
      if (error.componentSha256) fields.push(`宿主 SHA256 ${error.componentSha256}`);
      lines.push(`- ${fields.join(" | ")}`);
    }
  }
  return `${lines.join("\n")}\n`;
}

module.exports = { errorsOnlyReport, formatErrorReport };
