const test = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync, readdirSync } = require("node:fs");
const path = require("node:path");
const { pathToFileURL } = require('node:url');
const commandDeclaration = require('../host-commands.json');
const { isDelegatedCommand, isUpdateRead } = require('./host-commands.generated');

const projectRoot = path.resolve(__dirname, "..", "..", "..");

function sourceFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const target = path.join(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(target);
    if (!/\.(ts|tsx)$/.test(entry.name) || /\.test\./.test(entry.name)) return [];
    return [target];
  });
}

function bridgeCommands(source) {
  source = source.replace(/\r\n/g, "\n");
  assert.ok(source.includes('require("./host-commands.generated")'), 'Electron 转交必须使用生成契约');
  const delegated = Object.entries(commandDeclaration.commands).filter(([, flags]) => flags.includes('delegate')).map(([command]) => command);
  const implementedBlock = source.match(/const implemented = \{([\s\S]*?)\n\};\n\nfunction applyLocalProfile/);
  assert.ok(implementedBlock, "找不到 Electron 本地命令表");
  const implemented = [...implementedBlock[1].matchAll(
    /(?:^|\n)\s*(?:"([^"]+)"|([A-Za-z_][A-Za-z0-9_]*))\s*:/g,
  )].map((match) => match[1] || match[2]);
  return { delegated, known: new Set([...delegated, ...implemented]) };
}

test("界面使用的每个宿主命令都已进入 Electron 路由", () => {
  const bridge = readFileSync(path.join(__dirname, "bridge.js"), "utf8");
  const { known } = bridgeCommands(bridge);
  const documents = require("./personal-documents").createPersonalDocuments(() => null);
  const diskAccess = require("./macos-disk-access").createMacosDiskAccess(() => null, () => {});
  const pageAi = require("./personal-page-ai").createPersonalPageAi({ getOwner: () => null, documents, service: {} });
  const notificationNavigation = require("./personal-notification-navigation").createPersonalNotificationNavigation({ documents, service: {}, pageAi, getOwner: () => null, getProfile: () => null, emit: () => 0 });
  try {
  const main = readFileSync(path.join(__dirname, "main.js"), "utf8");
  assert.ok(main.includes("notificationNavigation.handles(message?.command)"));
  assert.ok(main.includes("notificationNavigation.ready(event, message.args)"));
  assert.ok(main.includes('if (isMainFrame) { notificationNavigation.invalidate();'));
  assert.ok(main.includes('on("render-process-gone", () => { notificationNavigation.invalidate();'));
  assert.ok(main.includes('on("closed", () => { notificationNavigation.invalidate();'));
  assert.ok(main.includes("personalDocuments.handles(message?.command)"));
  assert.ok(main.includes("personalDocuments.invoke(event, message)"));
  assert.ok(main.includes("macosDiskAccess.handles(message?.command)"));
  assert.ok(main.includes("macosDiskAccess.invoke(event, message)"));
  const missing = new Map();
  for (const file of sourceFiles(path.join(projectRoot, "apps", "desktop-ui", "src"))) {
    const source = readFileSync(file, "utf8");
    for (const match of source.matchAll(/\binvoke(?:<[^>]+>)?\(\s*["']([^"']+)["']/g)) {
      if (!known.has(match[1]) && !documents.handles(match[1]) && !diskAccess.handles(match[1]) && !pageAi.handles(match[1]) && !notificationNavigation.handles(match[1])) missing.set(match[1], path.relative(projectRoot, file));
    }
  }
  assert.deepEqual([...missing], []);
  } finally { pageAi.stop(); }
});

test('共享声明生成结果同步，业务处理器没有漏声明或多余授权', async () => {
  const { checkHostCommands } = await import(pathToFileURL(path.join(projectRoot, 'scripts/generate-host-commands.mjs')).href);
  const declaration = checkHostCommands(projectRoot);
  const source = readFileSync(path.join(projectRoot, 'apps/desktop/host-service/src/main.rs'), 'utf8');
  const commandHandlers = source.slice(source.indexOf('fn app_server_method('), source.indexOf('fn unified_search_params('));
  const handled = rustCommands(commandHandlers);
  for (const module of ['personal_page_ai', 'personal_notifications']) {
    const handler = readFileSync(path.join(projectRoot, `apps/desktop/host-service/src/${module}.rs`), 'utf8');
    for (const command of rustCommands(handler.slice(handler.indexOf('pub fn handle('), handler.indexOf('#[cfg(test)]')))) handled.add(command);
  }
  assert.deepEqual([...handled].sort(), Object.keys(declaration.commands).sort());
  for (const [command, flags] of Object.entries(declaration.commands)) {
    assert.equal(isDelegatedCommand(command), flags.includes('delegate'), command);
    assert.equal(isUpdateRead(command), flags.includes('update-read'), command);
    if (command.endsWith('_internal')) assert.equal(isDelegatedCommand(command), false, command);
  }
  for (const [method, flags] of Object.entries(declaration.appServerMethods)) {
    assert.equal(isUpdateRead('app_server_request', method), flags.includes('update-read'), method);
    assert.equal(isUpdateRead('other_command', method), false, method);
  }
  assert.equal(isDelegatedCommand('undeclared_command'), false);
  assert.equal(isUpdateRead('undeclared_command', 'undeclared/method'), false);
});

test('宿主声明拒绝未知标签和互相冲突的权限分类', async () => {
  const { validateHostCommands } = await import(pathToFileURL(path.join(projectRoot, 'scripts/generate-host-commands.mjs')).href);
  for (const flags of [['execute-anything'], ['delegate', 'delegate'], ['data-exclusive', 'data-observation'], ['parallel', 'context-change'], ['data-exclusive', 'update-read'], ['context-change', 'update-read']]) {
    const declaration = structuredClone(commandDeclaration);
    declaration.commands.host_platform = flags;
    assert.throws(() => validateHostCommands(declaration), /声明|策略/);
  }
  const declaration = structuredClone(commandDeclaration);
  declaration.commands.personal_page_ai_register_internal.push('delegate');
  assert.throws(() => validateHostCommands(declaration), /策略互斥/);
});

test("Electron 转交的每个命令都已进入 Rust 宿主服务", () => {
  const bridge = readFileSync(path.join(__dirname, "bridge.js"), "utf8");
  const { delegated } = bridgeCommands(bridge);
  const hostService = readFileSync(
    path.join(projectRoot, "apps", "desktop", "host-service", "src", "main.rs"),
    "utf8",
  );
  const handled = rustCommands(hostService);
  assert.deepEqual(delegated.filter((command) => !handled.has(command)), []);
});

function rustCommands(source) {
  return new Set([...source.matchAll(/^\s*((?:\|\s*)?"[^"]+"(?:\s*\|\s*"[^"]+")*)\s*=>/gm)]
    .flatMap((match) => [...match[1].matchAll(/"([^"]+)"/g)].map((name) => name[1])));
}

test("宿主命令解析覆盖合并分支且不接纳普通字符串", () => {
  assert.deepEqual([...rustCommands('"not-a-command";\n "first" | "second" => {}\n "third"\n | "fourth" => {}')], ["first", "second", "third", "fourth"]);
});

test("通用宿主入口仍拒绝浏览器写操作，不执行已移除的处理器", async () => {
  const source = readFileSync(path.join(__dirname, 'bridge.js'), 'utf8');
  const commands = ['browser_click', 'browser_type', 'browser_set_value'];
  const { known } = bridgeCommands(source);
  for (const command of commands) assert.equal(known.has(command), false);
  const dispatch = source.slice(source.indexOf('async function dispatch('), source.indexOf('function bindWindowEvents('));
  const browser = { takeover: false };
  const context = { module: { exports: {} }, BROWSER_WRITE_TOOLS: new Set(commands), browser };
  require('node:vm').runInNewContext(`${dispatch}\nmodule.exports=dispatch;`, context);
  for (const command of commands) {
    await assert.rejects(context.module.exports({ command, args: {} }, {}), /审批链路尚未接入/);
  }
  browser.takeover = true;
  for (const command of commands) {
    await assert.rejects(context.module.exports({ command, args: {} }, {}), /人工接管/);
  }
});


test("界面直接调用的应用服务方法均在宿主白名单中", () => {
  const runtime = readFileSync(path.join(projectRoot, "crates", "workbench-runtime", "src", "desktop_rpc.rs"), "utf8");
  const allowedBlock = runtime.match(/pub fn desktop_method_allowed\(method: &str\) -> bool \{([\s\S]*?)\n\}/);
  assert.ok(allowedBlock, "找不到桌面应用服务白名单");
  const allowed = new Set([...allowedBlock[1].matchAll(/"([^"\n]+)"/g)].map((match) => match[1]));
  for (const method of Object.keys(commandDeclaration.appServerMethods)) assert.ok(allowed.has(method), method);
  const missing = new Map();
  for (const file of sourceFiles(path.join(projectRoot, "apps", "desktop-ui", "src"))) {
    const source = readFileSync(file, "utf8");
    for (const match of source.matchAll(/\brpc(?:<[^>]+>)?\(\s*["']([^"']+)["']/g)) {
      if (!allowed.has(match[1])) missing.set(match[1], path.relative(projectRoot, file));
    }
  }
  assert.deepEqual([...missing], []);
});


test("磁盘导航专用主界面入口无法从普通dispatch或异步通道绕过", () => {
 const bridge = readFileSync(path.join(__dirname, "bridge.js"), "utf8");
 const host = readFileSync(path.join(projectRoot, "apps/desktop/host-service/src/main.rs"), "utf8");
 for (const command of ["macos_disk_access_open_settings", "macos_disk_access_reveal_application"]) {
  assert.ok(!bridgeCommands(bridge).known.has(command));
  assert.ok(rustCommands(host).has(command));
 }
});

test("旧企业容器命令不再注册，个人关闭握手与数据管理仍有正式入口", async () => {
  const source = readFileSync(path.join(__dirname,"bridge.js"),"utf8");
  const { known } = bridgeCommands(source);
  const old = ["application_container_open","application_container_restorable","application_container_detach","application_container_close","application_container_open_target","application_container_download_origins","application_container_download","application_container_clear_storage","application_theme_apply"];
  for(const name of old) assert.equal(known.has(name),false,name);
  for(const name of ["personal_applications_close_ack","personal_data_backup_capture","personal_application_data_rollback","personal_storage_inventory","mcp_connection_validate","model_connection_list"]) assert.equal(known.has(name),true,name);
  const implemented = source.slice(source.indexOf("const implemented = {"),source.indexOf("function applyLocalProfile"));
  const dispatch = source.slice(source.indexOf("async function dispatch("),source.indexOf("function bindWindowEvents("));
  let backendCalls=0;
  const vm=require("node:vm");const context={module:{exports:{}},console:{warn(){}},isDelegatedCommand,BROWSER_WRITE_TOOLS:new Set(),BrowserWindow:{fromWebContents(){return null;}},service:{call(){backendCalls++;throw Error("unexpected backend request");}}};
  vm.runInNewContext(`${implemented}\n${dispatch}\nmodule.exports=dispatch;`,context);
  for(const command of old) await assert.rejects(context.module.exports({command,args:{}},{}),/命令未迁移/);
  assert.equal(backendCalls,0);
  const main=readFileSync(path.join(__dirname,"main.js"),"utf8");
  assert.equal(/ipcMain\.handle\("container:bridge"/.test(main),false);
  assert.equal(/ipcMain\.on\("host:send"/.test(main),false);
  assert.ok(main.includes('require("./browser-webviews").guardBrowserWebviews'));
  assert.ok(main.includes('require("./personal-application-close").createPersonalApplicationClose'));
});
