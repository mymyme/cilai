const fs = require("node:fs");
const { createHmac, randomBytes } = require("node:crypto");
const path = require("node:path");
const { createServiceLog } = require("./service-log");
const { errorsOnlyReport } = require("./diagnostic-report");
const { publicAiCodes, publicCodes, safeId, safeRunId, requestId, failureTime } = require("./ai-error-diagnostics");
const { personalCodes, personalDiagnosticCodes } = require("./diagnostic-categories");

// An allowlist, not redaction of arbitrary text. Unknown messages never enter
// the feedback file, so prompts, model replies, URLs, tokens and paths cannot leak.
const startupRepairCodes = ["invalid_request", "expired", "local_data_changed", "caller_changed",
  "directory_changed", "backup_changed", "permissions_changed", "parent_changed"].map(suffix => `startup_repair_${suffix}`);
const sandboxStatusCodes = ["helper_missing", "private_owner_mismatch", "runtime_owner_mismatch",
  "runtime_access_denied", "runtime_prepare_failed", "log_failed", "spawn_failed",
  "workspace_unavailable", "control_failed", "setup_failed"].map(suffix => `windows_sandbox_${suffix}`);
const sandboxPreflightCodes = ["parameters_invalid", "protocol_invalid", "refresh_failed",
  "access_denied", "logon_denied", "credentials_invalid", "timeout", "exit_failed", "helper_failed"]
  .map(suffix => `sandbox_preflight_${suffix}`);
const nativeHistoryCodes = ["content_mismatch", "invalid", "state_invalid", "turn_missing",
  "notes_invalid", "notes_version_invalid", "binding_missing", "thread_missing", "title_missing",
  "archive_missing", "outbox_role_invalid", "message_item_missing", "message_role_invalid",
  "message_run_missing", "message_turn_missing", "message_link_missing", "user_item_missing",
  "run_missing", "reply_missing", "index_invalid", "handoff_message_missing", "handoff_turn_missing",
  "handoff_notes_missing", "handoff_notes_invalid"].map(suffix => `native_history_${suffix}`);
function errorCodes(value) {
  const text = typeof value === "string" ? value : "";
  const rules = [
    ["powershell_policy_failed", /powershell_policy_failed/],
    ["instance_discovery_failed", /instance_discovery_failed/],
    ["startup_permission_repair_failed", /startup_permission_repair_failed/],
    ["startup_data_initialization_failed", /startup_data_initialization_failed/],
    ["private_file_owner_mismatch", /private file is not owned by the current user/],
    ["private_file_acl_invalid", /exact restricted DACL|expected protected DACL/],
    ["sandbox_refresh_failed", /setup refresh had errors|setup refresh completed with errors/],
    ["sandbox_helper_unknown_error", /helper_unknown_error/],
    ["workspace_acl_access_denied", /SetNamedSecurityInfoW failed: 5|Windows 拒绝修改此目录/],
    ["runtime_directory_owner_mismatch", /runtime directory is not owned/],
    ["sandbox_preflight_failed", /windows_sandbox_preflight_failed|当前项目的沙箱执行检查失败/],
    ["sandbox_not_ready", /windows_sandbox_(?:not_configured|update_required|setup_failed|helper_missing)/],
    ["response_envelope_invalid", /ai[_ ]response[_ ]envelope[_ ]content[_ ]invalid/],
    ["administrator_authorization_cancelled", /已取消管理员授权|os error 1223/],
    ["permission_plan_expired", /确认已过期|确认已失效|os error 1237/],
    ["permission_verification_failed", /普通用户权限复查仍未通过/],
    ["permission_check_failed", /项目权限检查或修复未完成/],
    ["process_access_denied", /Access is denied|拒绝访问|os error 5\b/],
    ["connection_closed", /connection closed|broken pipe|os error 109\b/],
  ];
  const codes = [...rules.filter(([, pattern]) => pattern.test(text)).map(([code]) => code),
    ...[...startupRepairCodes, ...sandboxStatusCodes, ...sandboxPreflightCodes, ...nativeHistoryCodes].filter(code => new RegExp(`\\b${code}\\b`).test(text)), ...publicCodes(text), ...personalCodes(text)];
  if (codes.some(code => code.startsWith("sandbox_preflight_")) && !codes.includes("sandbox_preflight_failed")) codes.push("sandbox_preflight_failed");
  return codes;
}
const startupStages = new Set(["profile", "service", "history", "history_catalog"]);
const events = new Set(["startup_timing", "startup_lock", "response_timing", "app_start", "service_start", "service_exit", "service_error", "stderr_error", "command_error", "sandbox_status", "permission_check", "permission_repair", "logging_enabled", "ai_run_failure"]);
const statuses = new Set(["ready", "repairRequired", "error", "notApplicable", "notConfigured", "updateRequired", "failed", "starting", "running", "stopped"]);
const knownCodes = new Set(["powershell_policy_failed", "startup_data_initialization_failed", "startup_permission_repair_failed", "instance_discovery_failed", "private_file_owner_mismatch", "private_file_acl_invalid", "sandbox_refresh_failed", "sandbox_helper_unknown_error", "workspace_acl_access_denied", "runtime_directory_owner_mismatch", "sandbox_not_ready", "sandbox_preflight_failed", "response_envelope_invalid", "administrator_authorization_cancelled", "permission_plan_expired", "permission_verification_failed", "permission_check_failed", "process_access_denied", "connection_closed"]);
for (const code of [...startupRepairCodes, ...sandboxStatusCodes, ...sandboxPreflightCodes, ...nativeHistoryCodes, ...personalDiagnosticCodes]) knownCodes.add(code);
function safeRecord(event, fields = {}) {
  if (!events.has(event)) return null;
  const value = { event };
  if (typeof fields.command === "string" && /^[a-z_]{1,80}$/.test(fields.command)) value.command = fields.command;
  if (statuses.has(fields.status)) value.status = fields.status;
  if (Number.isInteger(fields.exitCode)) value.exitCode = fields.exitCode;
  if (Number.isFinite(fields.durationMs)) value.durationMs = Math.max(0, Math.round(fields.durationMs));
  if (Array.isArray(fields.codes)) value.codes = fields.codes.filter(code => knownCodes.has(code) || publicAiCodes.has(code)).slice(0, 16);
  if (event === "startup_timing" && startupStages.has(fields.stage)) value.stage = fields.stage;
  if (event === "startup_lock" && fields.stage === "runtime_publish") {
    value.stage = fields.stage;
    if (Number.isSafeInteger(fields.waitMs) && fields.waitMs >= 0) value.waitMs = fields.waitMs;
    if (Number.isSafeInteger(fields.holdMs) && fields.holdMs >= 0) value.holdMs = fields.holdMs;
  }
  if (event === "response_timing" && Number.isSafeInteger(fields.responseBytes) && fields.responseBytes >= 0) value.responseBytes = fields.responseBytes;
  if (["service_start", "sandbox_status", "command_error"].includes(event) && /^[a-f0-9]{32}$/.test(fields.fingerprint ?? "")) value.fingerprint = fields.fingerprint;
  if (safeId(fields.requestId)) value.requestId = safeId(fields.requestId);
  if (event === "ai_run_failure") {
    if (safeRunId(fields.runId)) value.runId = safeRunId(fields.runId);
    if (failureTime(fields.completedAt)) value.completedAt = fields.completedAt;
  }
  return value;
}
function createTestDiagnostics({ directory, metadata, maxBytes = 512 * 1024 }) {
  fs.mkdirSync(directory, { recursive: true });
  const preferences = path.join(directory, "preferences.json");
  const logPath = path.join(directory, "events.log");
  const log = createServiceLog(logPath, maxBytes);
  let enabled = true;
  let settingsError = null;
  let writeError = null;
  let stderrPending = "";
  let storagePaused = false;
  const observedFailures = new Set();
  const lastStatus = new Map();
  const fingerprintKey = randomBytes(32);
  const fingerprint = (detail, codes) => detail && !codes.length
    ? createHmac("sha256", fingerprintKey).update(detail).digest("hex").slice(0, 32)
    : undefined;
  function observe(command, event, fields) {
    const signature = JSON.stringify(safeRecord(event, fields));
    if (lastStatus.get(command) !== signature && api.record(event, fields)) lastStatus.set(command, signature);
  }
  try {
    const saved = JSON.parse(fs.readFileSync(preferences, "utf8"));
    if (typeof saved.enabled !== "boolean") throw new Error("invalid preferences");
    enabled = saved.enabled;
  } catch (error) {
    if (error.code !== "ENOENT") { enabled = false; settingsError = "测试日志设置读取失败，请重新设置开关。"; }
  }
  const api = {
    status() { return { enabled, error: settingsError ?? writeError, maxBytes: maxBytes * 2, storagePaused }; },
    suspendForStorage() {
      const lease = log.suspendForStorage();
      storagePaused = true;
      return {
        sealPending() { lease.sealPending(); },
        rebase() {
          lease.rebase();
          stderrPending = "";
          observedFailures.clear();
          lastStatus.clear();
        },
        resume() {
          lease.resume();
          storagePaused = false;
        },
      };
    },
    setEnabled(next) {
      if (storagePaused) throw new Error("personal_namespace_log_paused");
      if (typeof next !== "boolean") throw new Error("日志开关参数无效。");
      const temporary = `${preferences}.tmp`;
      fs.writeFileSync(temporary, JSON.stringify({ schemaVersion: 1, enabled: next }), { mode: 0o600 });
      fs.renameSync(temporary, preferences);
      enabled = next; settingsError = null; stderrPending = ""; lastStatus.clear();
      if (enabled) api.record("logging_enabled");
      return api.status();
    },
    record(event, fields) {
      if (!enabled) return;
      const safe = safeRecord(event, fields);
      if (!safe) return;
      try { const written = log.write(`${JSON.stringify(safe)}\n`); if (written) writeError = null; return written; }
      catch { writeError = "测试日志写入失败，请检查磁盘空间和日志目录权限。"; }
    },
    stderr(chunk) {
      if (!enabled || storagePaused) return;
      stderrPending += String(chunk);
      const lines = stderrPending.split("\n");
      stderrPending = (lines.pop() ?? "").slice(-8192);
      for (const line of lines) {
        const timing = /^personal_startup_timing stage=(profile|service|history|history_catalog) ms=(\d+) status=(ready|failed)$/.exec(line.trim());
        if (timing) {
          api.record("startup_timing", { stage: timing[1], durationMs: Number(timing[2]), status: timing[3] });
          continue;
        }
        const lock = /^personal_startup_lock stage=runtime_publish wait_ms=(\d+) hold_ms=(\d+)$/.exec(line.trim());
        if (lock) {
          api.record("startup_lock", { stage: "runtime_publish", waitMs: Number(lock[1]), holdMs: Number(lock[2]) });
          continue;
        }
        const codes = errorCodes(line);
        if (codes.length) api.record("stderr_error", { codes });
      }
    },
    response(command, result, error, durationMs, responseBytes) {
      if (!enabled) return;
      if (command === "app_server_status" || command === "ai_windows_sandbox_status") {
        const detail = [error, result?.runtimeInitializationError, result?.errorReason, result?.errorCode, result?.errorMessage]
          .filter(item => typeof item === "string").join("\n");
        const tail = Array.isArray(result?.stderrTail) ? result.stderrTail.slice(-12)
          .filter(line => typeof line === "string").map(line => line.slice(0, 4096)).join("\n") : "";
        const codes = errorCodes(`${detail}\n${tail}`);
        const status = error || result?.runtimeInitializationError ? "failed"
          : result?.runtimeInitializing ? "starting" : result?.phase ?? result?.status;
        observe(command, command === "app_server_status" ? "service_start" : "sandbox_status", {
          command, status, codes, fingerprint: fingerprint(detail, codes),
        });
      }
      else if (error) {
        const codes = errorCodes(error);
        api.record("command_error", { command, durationMs, codes, requestId: requestId(error), fingerprint: fingerprint(error, codes) });
      }
      if (!error && durationMs >= 100 && Number.isSafeInteger(responseBytes) && command !== "app_server_status") api.record("response_timing", { command, durationMs, responseBytes });
      if (!error && (command === "workspace_permissions_check" || command === "workspace_permissions_repair")) api.record(command.endsWith("repair") ? "permission_repair" : "permission_check", { status: result?.status, durationMs });
      else {
        const runs = command === "ai_runtime_snapshot" || command === "ai_runtime_activity_snapshot" ? result?.runs
          : command === "ai_conversation_start" || command === "ai_message_submit" ? [result?.run]
            : ["ai_message_send", "ai_run_control"].includes(command) ? [result] : [];
        for (const run of Array.isArray(runs) ? runs : []) {
          if (run?.status !== "failed" || !safeRunId(run.runId)) continue;
          const fields = { runId: safeRunId(run.runId), status: "failed", completedAt: run.completedAt,
            codes: errorCodes(run.errorReason), requestId: requestId(run.errorReason) };
          const key = JSON.stringify(safeRecord("ai_run_failure", fields));
          if (observedFailures.has(key)) continue;
          if (api.record("ai_run_failure", fields)) {
            observedFailures.add(key);
            if (observedFailures.size > 2048) observedFailures.delete(observedFailures.values().next().value);
          }
        }
      }
    },
    exportText(runtime) {
      const lines = [];
      for (const name of [`${logPath}.1`, logPath]) {
        try {
          // Our own bounded structured log only; no runtime/user/database files.
          const fd = fs.openSync(name, "r");
          let text;
          try {
            const size = fs.fstatSync(fd).size;
            const buffer = Buffer.alloc(Math.min(size, maxBytes));
            fs.readSync(fd, buffer, 0, buffer.length, Math.max(0, size - buffer.length));
            text = buffer.toString("utf8");
          } finally { fs.closeSync(fd); }
          for (const line of text.split("\n")) {
            const split = line.indexOf(" ");
            if (split < 0 || !/^\d{4}-\d{2}-\d{2}T[\d:.]+Z$/.test(line.slice(0, split))) continue;
            try {
              const value = JSON.parse(line.slice(split + 1));
              const safe = safeRecord(value.event, value);
              if (safe) lines.push({ at: line.slice(0, split), ...safe });
            } catch { /* Ignore incomplete lines; never export raw text. */ }
          }
        } catch (error) { if (error.code !== "ENOENT") throw new Error("无法读取测试日志，请检查日志目录权限。"); }
      }
      return JSON.stringify(errorsOnlyReport({ product: "词来工作台测试诊断", exportedAt: new Date().toISOString(),
        system: metadata(), logging: api.status(), privacy: "仅包含系统版本和结构化运行错误；不含会话正文、文件内容、账户凭据或请求参数。", events: lines }, runtime), null, 2);
    },
  };
  api.record("app_start");
  return api;
}
module.exports = { createTestDiagnostics, errorCodes };
