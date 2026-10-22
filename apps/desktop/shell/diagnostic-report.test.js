const test = require("node:test");
const assert = require("node:assert/strict");
const { errorsOnlyReport, formatErrorReport } = require("./diagnostic-report");

const header = { product: "词来工作台测试诊断", exportedAt: "2026-10-16T06:45:20.327Z", system: { version: "test", platform: "win32", osRelease: "10.0.26200", arch: "x64", electron: "test" }, logging: { enabled: true } };
function report(events = [], runtime) { return errorsOnlyReport({ ...header, events }, runtime); }

test("healthy startup, polling, shutdown and successful checks produce no exported errors", () => {
  const result = report([
    { event: "app_start" }, { event: "logging_enabled" }, { event: "service_start" },
    { event: "service_start", status: "starting" },
    { event: "service_start", status: "running", codes: ["process_access_denied"] },
    { event: "service_start", status: "stopped" },
    { event: "service_exit", exitCode: 0 }, { event: "service_exit" },
    { event: "sandbox_status", status: "ready" },
    { event: "permission_check", status: "ready" },
    { event: "permission_repair", status: "ready" },
  ], { status: "collected", instance: { privateFileValidation: "passed" }, sandbox: [{ status: "read", errors: [] }, { status: "absent" }] });
  assert.deepEqual(result.errors, []);
  assert.equal(result.exportMode, "errors-only");
  assert.match(formatErrorReport(result), /未发现已记录的错误/);
  for (const word of ["app_start", "running", "passed", "absent"]) assert.equal(formatErrorReport(result).includes(word), false);
});

test("repeated failure polling and retries merge without losing error types, time or counts", () => {
  const events = Array.from({ length: 30 }, (_, index) => ({
    event: "service_start", status: "failed", at: `2026-10-16T06:45:${String(index).padStart(2, "0")}.000Z`,
    codes: index % 2 ? ["process_access_denied", "instance_discovery_failed"] : ["instance_discovery_failed", "process_access_denied"],
  }));
  events.push(
    { event: "command_error", command: "enterprise_login", codes: [], at: "2026-10-16T06:44:56.021Z", durationMs: 10805 },
    { event: "command_error", command: "enterprise_login", codes: [], at: "2026-10-16T06:45:00.021Z", durationMs: 7 },
    { event: "command_error", command: "app_server_restart", codes: [], at: "2026-10-16T06:45:01.000Z" },
  );
  const result = report(events);
  assert.equal(result.errors.length, 3);
  assert.equal(result.errors[0].count, 30);
  assert.equal(result.errors[0].firstAt, "2026-10-16T06:45:00.000Z");
  assert.equal(result.errors[0].lastAt, "2026-10-16T06:45:29.000Z");
  assert.equal(result.errors[1].count, 2);
  assert.equal(result.errors[1].maxDurationMs, 10805);
  const text = formatErrorReport(result);
  assert.equal(text.split("\n").filter(line => line.startsWith("- ")).length, 3);
  assert.match(text, /共 3 项错误，合并前 33 条记录/);
  assert.match(text, /未分类错误/);
  assert.match(text, /enterprise_login/);
});

test("unknown failures, blocked sandbox states and nonzero exits remain visible", () => {
  const result = report([
    { event: "command_error", command: "app_server_request", codes: [] },
    { event: "service_error", codes: [] },
    { event: "service_exit", exitCode: 5 },
    { event: "sandbox_status", status: "error", codes: [] },
    { event: "sandbox_status", status: "notConfigured" },
    { event: "sandbox_status", status: "updateRequired" },
    { event: "permission_check", status: "repairRequired" },
  ]);
  assert.equal(result.errors.length, 7);
  assert.match(formatErrorReport(result), /退出码 5/);
});

test("runtime export keeps failure evidence, groups historical errors and omits private content", () => {
  const result = report([], {
    status: "collected", instance: { privateFileValidation: "failed", ownerMismatch: false, osError: 5 },
    sandbox: [
      { status: "read", source: "windows-sandbox-readiness.stderr.log", errors: [] },
      { status: "absent" },
      { status: "read", source: "sandbox.2026-10-13.log", errors: [
        { at: "2026-10-13T08:02:49.688504200+00:00", codes: ["sandbox_refresh_failed"], message: "PRIVATE_SECRET" },
        { at: null, codes: ["sandbox_refresh_failed"] },
        { at: "2026-10-13T08:03:20.343616500+00:00", codes: ["sandbox_refresh_failed"] },
      ] },
      { status: "read", source: "C:\\PRIVATE_PATH", errors: [{ at: "PRIVATE_TIME", codes: ["PRIVATE_CODE", "workspace_acl_access_denied"] }] },
      { status: "unreadable", osError: 5 },
    ],
  });
  assert.equal(result.errors.length, 4);
  assert.deepEqual(result.errors[0].codes, ["private_file_validation_failed"]);
  assert.equal(result.errors[0].osError, 5);
  assert.equal(result.errors[1].count, 3);
  assert.equal(result.errors[1].firstAt, "2026-10-13T08:02:49.688504200+00:00");
  assert.equal(result.errors[1].lastAt, "2026-10-13T08:03:20.343616500+00:00");
  const text = formatErrorReport(result);
  for (const value of ["PRIVATE_", "readiness", "ownerMismatch", "absent"]) assert.equal(text.includes(value), false);
  assert.match(text, /含历史记录/);
  assert.match(text, /diagnostic_log_unreadable/);
  assert.match(text, /workspace_acl_access_denied/);
});

test("failed diagnostic collection and logging remain explicit even without recorded events", () => {
  for (const status of ["unavailable", "timedOut"]) {
    const result = report([], { status });
    assert.equal(result.errors.length, 1);
    assert.equal(formatErrorReport(result).includes("未发现已记录的错误"), false);
  }
  const result = errorsOnlyReport({ ...header, logging: { enabled: false, error: "测试日志写入失败" }, events: [], runtimeDiagnostics: { secret: "PRIVATE_RAW_LOG" } });
  const text = formatErrorReport(result);
  assert.match(text, /记录开关：关闭/);
  assert.match(text, /日志功能错误：测试日志写入失败/);
  assert.match(text, /未发现其他已记录的错误/);
  assert.equal(JSON.stringify(result).includes("PRIVATE_RAW_LOG"), false);
});

test("startup failure before profile activation retains elevation and owner category without private metadata", () => {
  const result = report([], { status: "collected", profileStatus: "unavailable", sandbox: [], startup: {
    status: "collected", context: { processElevated: true, errors: [
      { scope: "profile", code: "private_file_owner_mismatch", ownerCategory: "administrators", path: "PRIVATE_PATH", sid: "PRIVATE_SID", securityDescriptor: "PRIVATE_DACL" },
      { scope: "PRIVATE_SCOPE", code: "private_file_validation_failed", ownerCategory: "PRIVATE_OWNER", osError: 5 },
      { scope: "profile", code: "PRIVATE_CODE" },
    ] },
  } });
  assert.equal(result.errors.length, 3);
  const text = formatErrorReport(result);
  assert.match(text, /startup_profile_unavailable/);
  assert.match(text, /目标账户目录 \| 进程权限 管理员 \| 所有者 管理员组/);
  assert.match(text, /系统错误 5/);
  assert.equal(JSON.stringify(result).includes("PRIVATE_"), false);
});

test("successful startup diagnostics stay out of errors-only export and collection failure stays visible", () => {
  assert.deepEqual(report([], { status: "collected", profileStatus: "active", startup: {status:"collected", context:{processElevated:false, errors:[]}} }).errors, []);
  assert.match(formatErrorReport(report([], { status: "collected", profileStatus: "unavailable", startup: {status:"unavailable"} })), /startup_diagnostics_unavailable/);
});

test("native repair failures retain exact phase, target, differences and rollback without private descriptors", () => {
  const result = report([], { status: "collected", startup: { status: "collected", context: { repairAttempts: [{
    operationId: "d3527d53-f550-4f1a-8d42-b19775824f26", atEpochSeconds: 1792226739,
    phase: "compare", targetIndex: 1, targetScope: "environment", previousAppliedIndex: 0,
    changedParts: ["daclFlags", "daclEntries", "PRIVATE_CHANGE"], osError: 10017,
    daclFlagsBefore: ["autoInherited", "PRIVATE_FLAG"], daclFlagsAfter: ["protected", "autoInherited"],
    rollbackVerified: true, componentSha256: "a".repeat(64), targetPath: "PRIVATE_PATH",
    expectedDescriptor: "PRIVATE_DACL_BEFORE", observedDescriptor: "PRIVATE_DACL_AFTER",
  }, { phase: "complete", targetPath: "PRIVATE_PATH" }, { phase: "PRIVATE_PHASE" }] } } });
  assert.equal(result.errors.length, 1);
  const text = formatErrorReport(result);
  assert.match(text, /阶段 修改前核对/);
  assert.match(text, /检查对象 第 2 项/);
  assert.match(text, /上次完成 第 1 项/);
  assert.match(text, /变化 权限继承标志、访问规则/);
  assert.match(text, /继承标志 AI → P\+AI/);
  assert.match(text, /原权限回读一致/);
  assert.match(text, /startup_repair_permissions_changed/);
  assert.match(text, /宿主 SHA256 a{64}/);
  assert.equal(JSON.stringify(result).includes("PRIVATE_"), false);
});

test("repair summaries reject untrusted values and preserve unverified rollback", () => {
  const result = report([], { status: "collected", startup: { status: "collected", context: { repairAttempts: [{
    phase: "write", targetIndex: -1, previousAppliedIndex: 900, targetScope: "PRIVATE_SCOPE",
    changedParts: ["PRIVATE_PART"], componentSha256: "PRIVATE_HASH", operationId: "PRIVATE_ID",
    atEpochSeconds: Number.MAX_SAFE_INTEGER, rollbackVerified: false, osError: 5,
  }] } } });
  const text = formatErrorReport(result);
  assert.match(text, /原权限回读未通过/);
  assert.match(text, /系统错误 5/);
  assert.equal(text.includes("PRIVATE_"), false);
  assert.equal(result.errors[0].targetIndex, undefined);
  assert.equal(result.errors[0].firstAt, undefined);
});
