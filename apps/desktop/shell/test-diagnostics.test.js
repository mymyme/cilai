const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { createTestDiagnostics } = require("./test-diagnostics");
const { formatErrorReport } = require("./diagnostic-report");
function fixture(t, maxBytes) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "cilai-test-logs-"));
  t.after(() => fs.rmSync(directory, {recursive:true,force:true}));
  const options = {directory,metadata:() => ({version:"test",platform:process.platform}), maxBytes};
  return {directory,options, log:createTestDiagnostics(options)};
}
test("sandbox execution preflight exports only the known code and run identity", t => {
  const {log} = fixture(t);
  const runId = "run_2442792a-b121-492f-9324-932293fca6b2";
  log.response("ai_message_send", {runId,status:"failed",completedAt:Date.now(),
    errorReason:"当前项目的沙箱执行检查失败，本轮尚未开始模型操作。 PRIVATE_CONTENT C:\\private\\project"});
  const report = log.exportText();
  assert.match(report, /sandbox_preflight_failed/);
  assert.match(report, new RegExp(runId));
  assert.doesNotMatch(report, /PRIVATE_CONTENT|private|当前项目/);
});
test("sandbox preflight exports specific safe causes across polling and restarts", t => {
  const {log,options} = fixture(t);
  for (const [index, suffix] of ["parameters_invalid", "protocol_invalid", "refresh_failed",
    "access_denied", "logon_denied", "credentials_invalid", "timeout", "exit_failed", "helper_failed"].entries()) {
    const code = `sandbox_preflight_${suffix}`;
    const run = {runId:`run_2442792a-b121-492f-9324-932293fca6b${index}`,status:"failed",
      completedAt:Date.parse("2026-10-17T12:54:37.317Z"),
      errorReason:`沙箱执行预检查未完成。错误类型：${code} PRIVATE_TOKEN C:\\private\\project`,
      output:"PRIVATE_REPLY"};
    for (let i=0; i<3; i++) log.response("ai_runtime_snapshot", {runs:[run]});
  }
  const report = JSON.parse(log.exportText());
  assert.equal(report.errors.length,9);
  for (const entry of report.errors) {
    assert.equal(entry.count,1);
    assert.equal(entry.firstAt,"2026-10-17T12:54:37.317Z");
    assert.equal(entry.codes.length,2);
    assert.ok(entry.codes.includes("sandbox_preflight_failed"));
  }
  const formatted = formatErrorReport(report);
  assert.match(formatted,/sandbox_preflight_parameters_invalid/);
  assert.doesNotMatch(formatted,/PRIVATE_|private\\project|沙箱执行预检查/);
  assert.deepEqual(JSON.parse(createTestDiagnostics(options).exportText()).errors,report.errors);
  log.response("ai_message_send", {runId:"run_2442792a-b121-492f-9324-932293fca6bf",status:"failed",
    errorReason:"PRIVATE_TOKEN sandbox_preflight_PRIVATE_CODE"});
  assert.deepEqual(JSON.parse(log.exportText()).errors.at(-1).codes,[]);
  assert.doesNotMatch(log.exportText(),/PRIVATE_/);
});
test("default on, disable is immediate and persists across restart", t => {
  const {options,log} = fixture(t);
  assert.equal(log.status().enabled,true);
  log.setEnabled(false);
  const before = log.exportText();
  log.record("service_error");
  log.stderr("helper_unknown_error\n");
  assert.deepEqual(JSON.parse(log.exportText()).errors, JSON.parse(before).errors);
  const restarted = createTestDiagnostics(options);
  assert.equal(restarted.status().enabled,false);
  restarted.setEnabled(true);
  assert.equal(createTestDiagnostics(options).status().enabled,true);
});
test("日志维护保留偏好，清理后不写回半行或丢弃同一错误的新观察", t => {
  const {directory, log} = fixture(t);
  log.setEnabled(true);
  const preferences = fs.readFileSync(path.join(directory, "preferences.json"));
  const run = {runId:"run_2442792a-b121-492f-9324-932293fca6b2",status:"failed",completedAt:Date.now(),errorReason:"sandbox_preflight_failed"};
  log.response("ai_message_send", run);
  log.stderr("helper_unknown_");
  const lease = log.suspendForStorage();
  assert.equal(log.status().storagePaused, true);
  assert.throws(() => log.setEnabled(false), /personal_namespace_log_paused/);
  assert.equal(log.record("service_error"), false);
  log.stderr("error\n");
  for (const name of ["events.log", "events.log.1"]) fs.rmSync(path.join(directory,name), {force:true});
  lease.rebase();
  lease.resume();
  assert.equal(log.status().storagePaused, false);
  assert.equal(log.status().enabled, true);
  assert.deepEqual(fs.readFileSync(path.join(directory,"preferences.json")), preferences);
  log.stderr("error\n");
  log.response("ai_message_send", run);
  const report = log.exportText();
  assert.doesNotMatch(report, /sandbox_helper_unknown_error/);
  assert.equal(JSON.parse(report).errors.filter(value=>value.runId===run.runId).length, 1);
});
test("export classifies failures and excludes all raw content", t => {
  const {log} = fixture(t);
  log.stderr("token=SUPER_SECRET conversation=PRIVATE_TEXT helper_unknown_error setup refresh ");
  log.stderr("had errors on C:\\Users\\PRIVATE_USER\\secret.txt\n");
  log.response("app_server_status", {phase:"failed", errorReason:"instance_discovery_failed", stderrTail:["private file is not owned by the current user"]}, null, 4);
  log.response("ai_message_send", {}, "ai_response_envelope_content_invalid PRIVATE_REPLY", 8);
  log.response("powershell_policy_apply", {}, "powershell_policy_failed PRIVATE_POLICY_PATH PRIVATE_POLICY_SID", 9);
  log.stderr("startup_data_initialization_failed PRIVATE_DIRECTORY\nstartup_permission_repair_failed PRIVATE_SID\n");
  const text = log.exportText();
  for (const value of ["SUPER_SECRET","PRIVATE_TEXT","PRIVATE_USER","secret.txt","PRIVATE_REPLY","PRIVATE_DIRECTORY","PRIVATE_SID","PRIVATE_POLICY_PATH","PRIVATE_POLICY_SID"]) assert.equal(text.includes(value),false);
  for (const code of ["sandbox_refresh_failed","private_file_owner_mismatch","response_envelope_invalid","startup_data_initialization_failed","startup_permission_repair_failed","powershell_policy_failed"]) assert.equal(text.includes(code),true);
});

test("sandbox status exports the returned error code even when its message has no technical details", t => {
  const {log,options} = fixture(t);
  for (const suffix of ["helper_missing", "private_owner_mismatch", "runtime_owner_mismatch",
    "runtime_access_denied", "runtime_prepare_failed", "log_failed", "spawn_failed",
    "workspace_unavailable", "control_failed", "setup_failed"]) {
    log.response("ai_windows_sandbox_status", {status:"error", errorCode:`windows_sandbox_${suffix}`,
      errorMessage:"无法确认状态 PRIVATE_MESSAGE C:\\private\\data"}, null, 10);
  }
  const report = JSON.parse(log.exportText());
  assert.equal(report.errors.length,10);
  for (const entry of report.errors) assert.ok(entry.codes.some(code => code.startsWith("windows_sandbox_")));
  assert.doesNotMatch(JSON.stringify(report), /PRIVATE_|private\\\\data|无法确认/);
  assert.deepEqual(JSON.parse(createTestDiagnostics(options).exportText()).errors,report.errors);
  log.response("ai_windows_sandbox_status", {status:"error",errorCode:"PRIVATE_UNKNOWN_CODE",errorMessage:"PRIVATE_SECRET"});
  const unknown = JSON.parse(log.exportText()).errors.find(entry => entry.codes.length === 0);
  assert.ok(unknown);
  assert.doesNotMatch(log.exportText(),/PRIVATE_/);
});

test("startup repair retains the exact failure stage without private helper output", t => {
  const {log} = fixture(t);
  for (const stage of ["invalid_request", "expired", "local_data_changed", "caller_changed",
    "directory_changed", "backup_changed", "permissions_changed", "parent_changed"]) {
    const code = `startup_repair_${stage}`;
    log.response("startup_permissions_repair", {}, `startup_permission_repair_failed ${code} PRIVATE_SID C:\\private\\data`);
  }
  const report = JSON.parse(log.exportText());
  assert.equal(report.errors.length, 8);
  assert.equal(report.errors.filter(entry => entry.codes.includes("startup_repair_permissions_changed")).length, 1);
  const text = formatErrorReport(report);
  assert.doesNotMatch(text, /PRIVATE_SID|private\\data/);
  assert.match(text, /startup_repair_parent_changed/);
});

test("history recovery errors survive running service status, restart and export", t => {
  const {log, options} = fixture(t);
  const error = "native_history_content_mismatch:message_PRIVATE_ID PRIVATE_CONTENT C:\\private\\data";
  log.response("app_server_status", {phase:"running", runtimeInitializationError:error, stderrTail:[]});
  log.response("app_server_restart", {}, error);
  log.response("startup_permissions_repair", {}, error);
  log.stderr(`AI 运行时初始化失败：${error}\n`);
  const report = JSON.parse(log.exportText());
  assert.equal(report.errors.length, 4);
  for (const entry of report.errors) assert.deepEqual(entry.codes, ["native_history_content_mismatch"]);
  assert.equal(report.errors.find(entry => entry.event === "service_start").status, "failed");
  assert.doesNotMatch(log.exportText(), /PRIVATE_|private|message_/);
  assert.deepEqual(JSON.parse(createTestDiagnostics(options).exportText()).errors, report.errors);
  log.response("app_server_status", {phase:"running", runtimeInitializationError:null, stderrTail:[error]});
  assert.deepEqual(JSON.parse(log.exportText()).errors, report.errors);
});
test("bounded rotation and disabled export retain previous diagnostic evidence", t => {
  const {directory,log} = fixture(t,400);
  for (let i=0;i<20;i++) log.record("service_exit", {exitCode:i});
  log.setEnabled(false);
  for (const file of ["events.log","events.log.1"]) assert.ok(fs.statSync(path.join(directory,file)).size <= 400);
  const result = JSON.parse(log.exportText());
  assert.ok(result.errors.some(event => event.exitCode === 19));
  assert.equal(result.logging.enabled,false);
});
test("corrupt preferences never silently re-enable recording", t => {
  const {directory,options} = fixture(t);
  fs.writeFileSync(path.join(directory,"preferences.json"),"invalid");
  const log = createTestDiagnostics(options);
  assert.equal(log.status().enabled,false);
  assert.ok(log.status().error);
  log.setEnabled(true);
  assert.equal(log.status().error,null);
});

const runId = "run_11111111-1111-4111-8111-111111111111";
const requestId = "f012d3e2-3857-452d-b9ca-1a19fc9ad81b";
const completedAt = Date.parse("2026-10-16T08:30:00.000Z");
const failedRun = { runId, status: "failed", completedAt,
  errorReason: `本次回复未完整完成\n错误代码：AI-EXEC-001\n请求编号：${requestId}\nPRIVATE_ERROR`,
  output: "PRIVATE_OUTPUT", events: [{ text: "PRIVATE_TOOL" }], conversationId: "PRIVATE_CONVERSATION" };

test("async failures are captured once after accepted send, using original time and request ID", t => {
  const {log, directory, options} = fixture(t);
  log.response("ai_message_send", {...failedRun, status:"running", errorReason:null}, null, 5);
  for (let i=0; i<5; i++) {
    log.response("ai_runtime_snapshot", {runs:[failedRun], messages:["PRIVATE_MESSAGE"]});
    log.response("ai_runtime_activity_snapshot", {runs:[failedRun]});
  }
  const result = JSON.parse(log.exportText());
  assert.equal(result.errors.length,1);
  assert.equal(result.errors[0].count,1);
  assert.equal(result.errors[0].firstAt, new Date(completedAt).toISOString());
  assert.equal(result.errors[0].requestId,requestId);
  assert.equal(result.errors[0].runId,runId);
  assert.deepEqual(result.errors[0].codes,["AI-EXEC-001"]);
  assert.match(formatErrorReport(result), /对话执行失败/);
  assert.ok(formatErrorReport(result).includes(requestId));
  assert.ok(formatErrorReport(result).includes(`运行编号 ${runId}`));
  assert.equal(fs.readFileSync(path.join(directory,"events.log"),"utf8").includes("PRIVATE_"),false);
  assert.equal(createTestDiagnostics(options).exportText().includes(requestId),true);
});

test("opening a historical failed run records it after logging is enabled without rerunning", t => {
  const {log} = fixture(t);
  log.setEnabled(false);
  log.response("ai_runtime_snapshot", {runs:[failedRun]});
  assert.equal(JSON.parse(log.exportText()).errors.length,0);
  log.setEnabled(true);
  log.response("ai_runtime_snapshot", {runs:[failedRun]});
  assert.equal(JSON.parse(log.exportText()).errors.length,1);
});

test("unknown failures stay visible; healthy runs and unvalidated fields are excluded", t => {
  const {log} = fixture(t);
  for (const status of ["queued","running","completed","cancelled"]) log.response("ai_message_send", {...failedRun,status});
  log.response("ai_runtime_snapshot", {runs:[{...failedRun,runId:"PRIVATE_RUN"}]});
  assert.equal(JSON.parse(log.exportText()).errors.length,0);
  log.response("ai_conversation_start", {run:{...failedRun,completedAt:"PRIVATE_TIME",errorReason:'PRIVATE_ERROR AI-EXEC-999 "request_id":"PRIVATE_REQUEST"'}});
  const result = JSON.parse(log.exportText());
  assert.equal(result.errors.length,1);
  assert.deepEqual(result.errors[0].codes,[]);
  assert.equal(result.errors[0].requestId,undefined);
  assert.equal(result.errors[0].firstAt,undefined);
  assert.equal(JSON.stringify(result).includes("PRIVATE_"),false);
  assert.match(formatErrorReport(result),/未分类错误/);
});

test("synchronous public gateway failures capture only allowlisted code and UUID", t => {
  const {log} = fixture(t);
  log.response("ai_message_send", null, `PRIVATE_REQUEST_BODY AI-AUTH-003 {"request_id":"${requestId}"}`, 10);
  const result = JSON.parse(log.exportText());
  assert.deepEqual(result.errors[0].codes,["AI-AUTH-003"]);
  assert.equal(result.errors[0].requestId,requestId);
  assert.equal(JSON.stringify(result).includes("PRIVATE_"),false);
});

test("only runtime UUID prefixes are accepted; gateway request IDs remain UUID-only", t => {
  const {log,options} = fixture(t);
  const {safeRunId,safeId,requestId:parseRequestId} = require("./ai-error-diagnostics");
  for (const id of [requestId, `run_${requestId}`, `app_run_${requestId}`]) {
    assert.equal(safeRunId(id),id);
    log.response("ai_runtime_activity_snapshot", {runs:[{...failedRun,runId:id}]});
  }
  for (const id of [`conversation_${requestId}`,`message_${requestId}`,`run_${requestId} PRIVATE_PROMPT`,
    `run_${requestId}/file`,"run_PRIVATE_TEXT","app_run_","PRIVATE_TOKEN"]) {
    assert.equal(safeRunId(id),null);
    log.response("ai_runtime_snapshot", {runs:[{...failedRun,runId:id}]});
  }
  assert.equal(safeId(`run_${requestId}`),null);
  assert.equal(parseRequestId(`请求编号：run_${requestId}`),null);
  const report=JSON.parse(log.exportText());
  assert.equal(report.errors.length,3);
  assert.deepEqual(JSON.parse(createTestDiagnostics(options).exportText()).errors,report.errors);
  assert.doesNotMatch(log.exportText(),/PRIVATE_|conversation_|message_/);
});
