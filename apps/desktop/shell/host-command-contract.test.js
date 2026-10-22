const test = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync, readdirSync } = require("node:fs");
const path = require("node:path");

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
  const delegatedBlock = source.match(/const delegated = new Set\(\[([\s\S]*?)\]\);/);
  assert.ok(delegatedBlock, "找不到 Electron 转交命令表");
  const delegated = [...delegatedBlock[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]);
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


test("界面直接调用的应用服务方法均在宿主白名单中", () => {
  const runtime = readFileSync(path.join(projectRoot, "crates", "workbench-runtime", "src", "desktop_rpc.rs"), "utf8");
  const allowedBlock = runtime.match(/pub fn desktop_method_allowed\(method: &str\) -> bool \{([\s\S]*?)\n\}/);
  assert.ok(allowedBlock, "找不到桌面应用服务白名单");
  const allowed = new Set([...allowedBlock[1].matchAll(/"([^"\n]+)"/g)].map((match) => match[1]));
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
  const delegated = source.match(/const delegated = new Set\(\[([\s\S]*?)\]\);/)[0];
  const dispatch = source.slice(source.indexOf("async function dispatch("),source.indexOf("function bindWindowEvents("));
  let backendCalls=0;
  const vm=require("node:vm");const context={module:{exports:{}},console:{warn(){}},BROWSER_WRITE_TOOLS:new Set(),BrowserWindow:{fromWebContents(){return null;}},service:{call(){backendCalls++;throw Error("unexpected backend request");}}};
  vm.runInNewContext(`${delegated}\n${implemented}\n${dispatch}\nmodule.exports=dispatch;`,context);
  for(const command of old) await assert.rejects(context.module.exports({command,args:{}},{}),/命令未迁移/);
  assert.equal(backendCalls,0);
  const main=readFileSync(path.join(__dirname,"main.js"),"utf8");
  assert.equal(/ipcMain\.handle\("container:bridge"/.test(main),false);
  assert.equal(/ipcMain\.on\("host:send"/.test(main),false);
  assert.ok(main.includes('require("./browser-webviews").guardBrowserWebviews'));
  assert.ok(main.includes('require("./personal-application-close").createPersonalApplicationClose'));
});
