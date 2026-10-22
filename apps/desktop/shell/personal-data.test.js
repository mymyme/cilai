const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const { preparePersonalRestore, planPersonalWorkspaceRestore, runPersonalMaintenance, runPersonalApplicationChange, exportPersonalBackup, restartAfterPersonalRestore } = require("./personal-data");

test("恢复原文目标只取自原生目录选择，取消和伪造路径均不提交", async () => {
  const calls = [];
  const state = { args: { request: { operationId: "restore", previewDigest: "before", clientOperationId: "choose-once" } }, window: {},
    dialog: { showOpenDialog: async () => ({ canceled: true }) }, service: { call: async (...args) => { calls.push(args); return { previewDigest: "after" }; } } };
  assert.equal(await planPersonalWorkspaceRestore(state), null);
  assert.equal(calls.length, 0);
  state.dialog.showOpenDialog = async (_window, options) => {
    assert.deepEqual(options.properties, ["openDirectory", "createDirectory"]);
    return { canceled: false, filePaths: [path.resolve("new-projects")] };
  };
  assert.deepEqual(await planPersonalWorkspaceRestore(state), { previewDigest: "after" });
  assert.deepEqual(calls, [["personal_data_restore_workspace_plan", { request: {
    ...state.args.request, destinationParent: path.resolve("new-projects"),
  } }]]);
  state.args.request.destinationParent = "/injected";
  await assert.rejects(planPersonalWorkspaceRestore(state), /request_invalid/);
  assert.equal(calls.length, 1);
});

test("恢复位置选择失败和未知保存回执不会自动提交或重试", async () => {
  let calls = 0;
  const state = { args: { request: { operationId: "restore", previewDigest: "before", clientOperationId: "once" } },
    dialog: { showOpenDialog: async () => ({ canceled: false, filePaths: ["relative"] }) },
    service: { call: async () => { calls++; throw Error("unknown"); } } };
  await assert.rejects(planPersonalWorkspaceRestore(state), /destination_invalid/);
  assert.equal(calls, 0);
  state.dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [path.resolve("new-projects")] });
  await assert.rejects(planPersonalWorkspaceRestore(state), /unknown/);
  assert.equal(calls, 1);
});

test("恢复候选仅使用原生选择的备份路径，取消不发送请求", async () => {
  const calls = [];
  const state = { args: { request: { clientOperationId: "restore-once" } }, window: {},
    dialog: { showOpenDialog: async () => ({ canceled: true }) }, service: { call: async (...args) => calls.push(args) } };
  assert.equal(await preparePersonalRestore(state), null);
  assert.equal(calls.length, 0);
  state.dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [path.resolve("backup.zip")] });
  await preparePersonalRestore(state);
  assert.deepEqual(calls, [["personal_data_restore_prepare", { request: { clientOperationId: "restore-once", archivePath: path.resolve("backup.zip") } }]]);
  state.args.request.archivePath = "/not-from-picker.zip";
  await assert.rejects(preparePersonalRestore(state), /request_invalid/);
  assert.equal(calls.length, 1);
});

test("维护先关闭应用，拒绝前端伪造静止证明；关闭失败不执行维护", async () => {
  const calls = [];
  const state = { command: "personal_data_backup_capture", args: { request: { operationId: "op", previewDigest: "digest", clientOperationId: "capture" } },
    closeApplications: async () => calls.push("close"), service: { call: async command => calls.push(command) } };
  await runPersonalMaintenance(state);
  assert.deepEqual(calls, ["close", "personal_data_backup_capture"]);
  state.args.request.quiescence = {};
  await assert.rejects(runPersonalMaintenance(state), /request_invalid/);
  delete state.args.request.quiescence;
  state.closeApplications = async () => { throw new Error("close_unconfirmed"); };
  await assert.rejects(runPersonalMaintenance(state), /close_unconfirmed/);
  assert.equal(calls.length, 2);
});

test("维护未知回执不自动重试", async () => {
  let count = 0;
  await assert.rejects(runPersonalMaintenance({ command: "personal_data_restore_apply",
    args: { request: { operationId: "op", previewDigest: "digest", expectedCurrentProfileId: "profile", clientOperationId: "apply" } },
    closeApplications: async () => {}, service: { call: async () => { count++; throw new Error("unknown"); } } }), /unknown/);
  assert.equal(count, 1);
});

test("全部记忆删除只收预览标识，关闭应用后调用一次", async () => {
  const calls=[];
  const state={command:"personal_memory_reset_confirm",args:{request:{operationId:"memory",previewDigest:"digest"}},closeApplications:async()=>calls.push("close"),service:{call:async(command,args)=>{calls.push([command,args]);throw new Error("unknown");}}};
  await assert.rejects(runPersonalMaintenance(state),/unknown/);
  assert.equal(calls.length,2);assert.equal(calls[0],"close");assert.equal(calls[1][0],"personal_memory_reset_confirm");
  state.args.request.quiescence={};await assert.rejects(runPersonalMaintenance(state),/request_invalid/);assert.equal(calls.length,2);
});

test("应用升级关闭容器并拒界面注入资料证明，未知不再提交",async()=>{
  const calls=[];
  const state={command:"personal_capability_upgrade",args:{request:{creationProjectId:"app",expectedRevision:1,packageDigest:"digest",confirmedPermissions:[],clientOperationId:"once"}},closeApplications:async()=>calls.push("close"),service:{call:async(command)=>{calls.push(command);throw Error("unknown");}}};
  await assert.rejects(runPersonalApplicationChange(state),/unknown/);assert.deepEqual(calls,["close","personal_capability_upgrade"]);
  state.args.request.dataChange={};await assert.rejects(runPersonalApplicationChange(state),/request_invalid/);assert.equal(calls.length,2);
});

test("存储清理经过应用关闭与同一受控维护入口", async () => {
  const calls=[];
  await runPersonalMaintenance({command:"personal_storage_execute",args:{request:{operationId:"storage",previewDigest:"digest",clientOperationId:"once"}},
    closeApplications:async()=>calls.push("close"),service:{call:async(command,args)=>{calls.push([command,args]);return {state:"incomplete"};}}});
  assert.equal(calls[0],"close");
  assert.equal(calls[1][0],"personal_storage_execute");
  assert.equal(calls.length,2);
});

test("备份导出由原生保存框指定目标，取消不导出", async () => {
  const calls = [];
  const state = { args: { request: { operationId: "op" } }, dialog: { showSaveDialog: async () => ({ canceled: true }) },
    service: { call: async (...args) => { calls.push(args); return { saved: true }; } } };
  assert.deepEqual(await exportPersonalBackup(state), { saved: false });
  state.dialog.showSaveDialog = async () => ({ canceled: false, filePath: path.resolve("export.zip") });
  assert.deepEqual(await exportPersonalBackup(state), { saved: true });
  assert.deepEqual(calls, [["personal_data_backup_export", { operationId: "op", destinationPath: path.resolve("export.zip") }]]);
});

test("恢复仍在应用时不能重启，结束后只重启当前应用", async () => {
  const calls = [];
  const state = { service: { call: async () => ({ personalDataState: "applying" }) }, app: { relaunch: () => calls.push("relaunch"), quit: () => calls.push("quit") } };
  await assert.rejects(restartAfterPersonalRestore(state), /restart_not_ready/);
  state.service.call = async () => ({ personalDataState: "restart_required" });
  await restartAfterPersonalRestore(state);
  assert.deepEqual(calls, ["relaunch", "quit"]);
});
