const test = require("node:test");
const assert = require("node:assert/strict");
const { createPersonalDocuments } = require("./personal-documents");
const { createPersonalPageAi } = require("./personal-page-ai");

function fixture(t, options = {}) {
  const owner = { isDestroyed: () => false, mainFrame: { url: "app://localhost/", frames: [] } };
  const event = { sender: owner, senderFrame: owner.mainFrame };
  const documents = createPersonalDocuments(() => owner);
  const calls = [];
  const service = { call: async (command, args) => { calls.push({ command, args }); return options.call ? options.call(command, args) : { retired: true, processesExited: true, cleanupPending: false, auditPending: false }; } };
  const page = createPersonalPageAi({ getOwner: () => owner, documents, service, actionTimeoutMs: 20 });
  t.after(() => page.stop());
  const binding = { creationProjectId: "personal1", expectedPackageDigest: `sha256:${"a".repeat(64)}` };
  const url = documents.invoke(event, { command: "personal_document_create", args: { html: "<p>page</p>", pageBinding: binding } });
  const frame = { url, detached: false, frameTreeNodeId: 56, processId: 12, routingId: 34, executeJavaScript: async code => { frame.lastCode = code; return { applied: true }; } };
  owner.mainFrame.frames.push(frame);
  const dispatch = (method = "session.read", params = {}) => page.invoke(event, { command: "personal_page_ai_dispatch", args: { url, request: { method, requestId: "request_1", params } } });
  return { page, documents, owner, event, url, binding, frame, calls, dispatch };
}

test("页桥仅接受真实工作台主框架，页面不能自批或注入宿主身份", async t => {
  const f = fixture(t);
  await assert.rejects(f.page.invoke({ ...f.event, senderFrame: f.frame }, { command: "personal_page_ai_dispatch", args: {} }), /来源无效/);
  await assert.rejects(f.dispatch("model.write.resolve"), /method_not_allowed/);
  await assert.rejects(f.page.invoke(f.event, { command: "personal_page_ai_dispatch", args: { url: f.url, documentId: "forged", request: {} } }), /request_invalid/);
  assert.equal(f.calls.length, 0);
  await f.dispatch();
  assert.deepEqual(f.calls[0].args.binding, f.binding);
  assert.match(f.calls[0].args.documentId, /^document_[a-f0-9]{32}$/);
  assert.equal(f.calls[1].command, "personal_page_ai_dispatch_internal");
});

test("页面绑定必须来自不可变登记且精确匹配唯一原生子框架", async t => {
  const f = fixture(t);
  assert.throws(() => f.documents.invoke(f.event, { command: "personal_document_create", args: { html: "x", pageBinding: { ...f.binding, path: "/forged" } } }), /binding_invalid/);
  f.owner.mainFrame.frames.push({ ...f.frame });
  await assert.rejects(f.dispatch(), /frame_unconfirmed/);
  f.owner.mainFrame.frames.pop();
  await f.dispatch();
  f.owner.mainFrame.frames[0] = { ...f.frame, routingId: 35 };
  await assert.rejects(f.dispatch(), /frame_changed/);
  assert.equal(f.calls.filter(call => call.command === "personal_page_ai_register_internal").length, 1);
});

test("导航退役原范围，同URL重载不能自动重新登记", async t => {
  const f = fixture(t);
  await f.dispatch();
  f.page.navigation(f.owner, { frame: f.frame });
  await assert.rejects(f.dispatch(), /document_closed/);
  await f.page.release(f.url);
  assert.equal(f.calls.filter(call => call.command === "personal_page_ai_close_internal").length, 1);
});

test("通知只定位创建该运行的现存文档，重读核对后不执行动作或新建运行", async t => {
  const target = { kind: "application", runId: "run-notification", applicationId: "app-notification", sessionId: "session-notification" };
  let valid = true;
  const f = fixture(t, { call: (command, args) => {
    if (command === "personal_page_ai_dispatch_internal") {
      return { runId: target.runId, applicationId: valid ? target.applicationId : "changed" };
    }
    return { retired: true, processesExited: true, cleanupPending: false, auditPending: false };
  } });
  await f.dispatch("model.run.create", { modelSessionId: target.sessionId });
  const before = f.calls.length;
  assert.deepEqual(await f.page.notificationLocation(target), { creationProjectId: f.binding.creationProjectId, documentUrl: f.url });
  assert.equal(f.calls.length, before + 1);
  assert.equal(f.calls.at(-1).args.request.method, "model.run.get");
  assert.equal(f.frame.lastCode, undefined);
  valid = false;
  await assert.rejects(f.page.notificationLocation(target), /target_missing/);
  const prior = f.calls.length;
  await assert.rejects(f.page.notificationLocation({ ...target, sessionId: "other-session" }), /target_missing/);
  assert.equal(f.calls.length, prior);
  f.page.navigation(f.owner, { frame: f.frame });
  await assert.rejects(f.page.notificationLocation(target), /target_missing/);
  assert.equal(f.calls.filter(call => call.command === "personal_page_ai_dispatch_internal").length, 3);
  assert.equal(f.calls.filter(call => call.command === "personal_page_ai_close_internal").length, 1);
});

test("可信确认单独调用，页内方法表不能到达它", async t => {
  const f = fixture(t);
  await f.page.invoke(f.event, { command: "personal_page_ai_resolve", args: { url: f.url, runId: "run1", confirmationId: "confirm1", decision: "approved", arguments: { text: "已核对" } } });
  assert.deepEqual(f.calls[1], { command: "personal_page_ai_resolve_internal", args: { documentId: f.calls[0].args.documentId, runId: "run1", confirmationId: "confirm1", decision: "approved", arguments: { text: "已核对" } } });
  await assert.rejects(f.page.invoke(f.event, { command: "personal_page_ai_resolve", args: { url: f.url, runId: "run1", confirmationId: "confirm1", decision: "allow" } }), /request_invalid/);
});

test("动作只在原生绑定页面调用固定入口，超时后退役且不重复执行", async t => {
  const f = fixture(t);
  await f.dispatch();
  const action = { documentId: f.calls[0].args.documentId, actionId: "setTitle", arguments: { text: "');globalThis.injected=true;//" } };
  assert.deepEqual(await f.page.action(action), { applied: true });
  assert.equal(f.frame.lastCode, `window.__cilaiPersonalAiExecute(${JSON.stringify({ actionId: action.actionId, arguments: action.arguments })})`);
  let executions = 0;
  f.frame.executeJavaScript = () => { executions++; return new Promise(() => {}); };
  await assert.rejects(f.page.action(action), /outcome_unconfirmed/);
  await assert.rejects(f.page.action(action), /document_closed/);
  assert.equal(executions, 1);
});

test("撤销未确认时封住新请求，明确重新核对后才能恢复", async t => {
  let failed = true;
  const f = fixture(t, { call: command => { if (command === "personal_page_ai_revoke_all_internal" && failed) throw new Error("stop_unconfirmed"); return { retired: true, processesExited: true, cleanupPending: false, auditPending: false }; } });
  await f.dispatch();
  await assert.rejects(f.page.revokeAll(), /stop_unconfirmed/);
  await assert.rejects(f.dispatch(), /close_unconfirmed/);
  failed = false;
  await f.page.revokeAll();
  await assert.rejects(f.dispatch(), /document_closed/);
  assert.equal(f.calls.filter(call => call.command === "personal_page_ai_register_internal").length, 1);
});

test("锁定Electron真实iframe执行SDK动作，重载撤权且不能自批", { skip: process.env.CILAI_PERSONAL_PAGE_AI_NATIVE !== "1", timeout: 45_000 }, () => {
  const fs = require("node:fs"), path = require("node:path"), crypto = require("node:crypto"), { spawnSync } = require("node:child_process");
  const source = path.resolve(__dirname, "../../.."), project = path.resolve(source, "../..");
  const parent = path.join(project, ".personal-build/page-ai-fixtures"); fs.mkdirSync(parent, { recursive: true });
  const root = fs.mkdtempSync(path.join(parent, "native-"));
  for (const folder of ["home", "tmp", "userData", "session", "logs"]) fs.mkdirSync(path.join(root, folder));
  const executable = path.join(project, ".personal-build/apps/desktop/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron");
  assert.ok(fs.realpathSync(executable).startsWith(path.join(project, ".personal-build") + path.sep));
  const driver = path.join(project, "07测试/实施证据/T4-执行脚本/electron-personal-page-ai.cjs");
  const sdkSource = path.join(source, "apps/desktop-ui/src/personal-page-ai-sdk.ts");
  const typescript = require(path.join(source, "node_modules/typescript"));
  const compiled = typescript.transpileModule(fs.readFileSync(sdkSource, "utf8"), { compilerOptions: { module: typescript.ModuleKind.CommonJS, target: typescript.ScriptTarget.ES2022 } }).outputText;
  const module_ = { exports: {} }; new Function("exports", compiled)(module_.exports);
  const sdk = path.join(root, "sdk.js"); fs.writeFileSync(sdk, module_.exports.personalPageAiBootstrap("fixture_channel"));
  const preload = path.join(root, "preload.cjs");
  fs.writeFileSync(preload, "const{contextBridge,ipcRenderer}=require('electron');contextBridge.exposeInMainWorld('pageProbe',{invoke:(command,args)=>ipcRenderer.invoke('page-probe',{command,args})});");
  const config = { root, shell: __dirname, sdk, preload, result: path.join(root, "result.json") };
  const configPath = path.join(root, "config.json"); fs.writeFileSync(configPath, JSON.stringify(config));
  const result = spawnSync(executable, [driver, configPath], { cwd: root, timeout: 40_000, encoding: "utf8", maxBuffer: 2 * 1024 * 1024,
    env: { HOME: path.join(root, "home"), USERPROFILE: path.join(root, "home"), TMPDIR: path.join(root, "tmp"), TMP: path.join(root, "tmp"), TEMP: path.join(root, "tmp"), PATH: "/usr/bin:/bin:/usr/sbin:/sbin", LANG: "en_US.UTF-8" } });
  const hashes = Object.fromEntries(["personal-page-ai.js", "personal-documents.js", "personal-application-close.js", "main.js", "bridge.js"].map(name => [name, crypto.createHash("sha256").update(fs.readFileSync(path.join(__dirname, name))).digest("hex")]));
  const evidence = { capturedAt: new Date().toISOString(), exitCode: result.status, signal: result.signal, error: result.error?.message ?? null,
    fixture: path.relative(project, root), moduleSha256: hashes, sdkSha256: crypto.createHash("sha256").update(fs.readFileSync(sdkSource)).digest("hex"),
    electronSha256: crypto.createHash("sha256").update(fs.readFileSync(executable)).digest("hex"), result: fs.existsSync(config.result) ? JSON.parse(fs.readFileSync(config.result)) : null, stderr: result.stderr };
  fs.writeFileSync(path.join(project, `07测试/实施证据/T4-页面AI真实Electron-${path.basename(root)}-20261022.json`), JSON.stringify(evidence, null, 2));
  assert.equal(result.status, 0, result.stderr);
  assert.equal(evidence.result.electron, "44.4.4");
  console.log(result.stdout.trim());
});
