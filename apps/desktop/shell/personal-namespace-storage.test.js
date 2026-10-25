const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const fsp = require("node:fs/promises");
const path = require("node:path");
const { createPersonalNamespaceStorage } = require("./personal-namespace-storage");

const PROFILE = "11111111111141118111111111111111";
const OTHER_PROFILE = "22222222222242228222222222222222";

function fixture(t) {
  const parent = path.join(require("./personal-isolation").personalBuildRoot(), "namespace-storage-fixtures");
  fs.mkdirSync(parent, { recursive: true });
  const root = fs.mkdtempSync(path.join(fs.realpathSync(parent), "probe-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const userData = path.join(root, "cn.cilai.personal.workbench");
  const paths = { userData, logs: path.join(userData, "logs"), sessionData: path.join(userData, "browser") };
  const storagePath = path.join(paths.sessionData, "Partitions", `ai-browser-${PROFILE}`);
  fs.mkdirSync(storagePath, { recursive: true });
  const session = { storagePath, async getCacheSize() { cacheCalls++; return 4096; },
    get cookies() { throw new Error("cookies must not be read"); },
    clearCache() { throw new Error("cache must not be cleared"); },
    clearStorageData() { throw new Error("browser storage must not be cleared"); } };
  let cacheCalls = 0;
  let verifiedProfileId = PROFILE;
  const app = { getPath: name => paths[name] };
  const contents = { isDestroyed: () => false, session };
  const browser = { profileId: PROFILE, tabs: new Map([["existing", { contents }]]),
    isPartition: value => value === `persist:ai-browser-${browser.profileId}`,
    sessionOf: () => { throw new Error("inventory must not create a session"); } };
  const inventory = createPersonalNamespaceStorage({ app, browser, getVerifiedProfileId: () => verifiedProfileId });
  return { root, paths, session, app, browser, inventory, cacheCalls: () => cacheCalls,
    setProfile: value => { verifiedProfileId = value; } };
}

test("只统计固定日志元数据和当前资料HTTP缓存，不读正文或浏览器登录数据", async t => {
  const f = fixture(t);
  fs.mkdirSync(f.paths.logs);
  fs.mkdirSync(path.join(f.paths.userData, "test-diagnostics"));
  fs.writeFileSync(path.join(f.paths.logs, "host-service.log"), "abc");
  fs.writeFileSync(path.join(f.paths.userData, "test-diagnostics/events.log"), "12345");
  const credential = "synthetic-private-browser-sentinel";
  fs.writeFileSync(path.join(f.session.storagePath, "Cookies"), credential);
  fs.writeFileSync(path.join(f.session.storagePath, "Login Data"), credential);
  t.mock.method(fsp, "readFile", () => { throw new Error("content must not be read"); });
  const value = await f.inventory.inventory();
  assert.equal(value.profileId, PROFILE);
  assert.equal(value.observation, "live_metadata");
  assert.equal(value.knownLogicalBytes, 4104);
  assert.equal(value.sizeComplete, true);
  assert.deepEqual(value.items.map(item => [item.category, item.logicalBytes, item.fileCount]), [["logs", 3, 1], ["logs", 5, 1], ["cache", 4096, null]]);
  assert.ok(value.items.every(item => !item.cleanupAllowed && item.snapshotDigest === null && item.managedRelativePath === null));
  assert.equal(f.cacheCalls(), 1);
  assert.ok(!JSON.stringify(value).includes(credential));
  assert.ok(!JSON.stringify(value).includes(f.root));
  assert.equal(fs.readFileSync(path.join(f.session.storagePath, "Cookies"), "utf8"), credential);
});

test("未生成的日志目录显示零占用，统计不创建目录", async t => {
  const f = fixture(t);
  const value = await f.inventory.inventory();
  assert.deepEqual(value.items.slice(0, 2).map(item => [item.logicalBytes, item.fileCount]), [[0, 0], [0, 0]]);
  assert.equal(value.knownLogicalBytes, 4096);
  assert.equal(fs.existsSync(f.paths.logs), false);
  assert.equal(fs.existsSync(path.join(f.paths.userData, "test-diagnostics")), false);
});

test("不接受界面路径参数，未核资料或非个人命名空间在任何缓存调用前拒绝", async t => {
  const f = fixture(t);
  await assert.rejects(f.inventory.inventory({ path: f.paths.userData }), /personal_storage_request_invalid/);
  f.setProfile("legacy");
  await assert.rejects(f.inventory.inventory(), /personal_storage_profile_unavailable/);
  f.setProfile(PROFILE);
  f.paths.userData = path.join(f.root, "enterprise");
  await assert.rejects(f.inventory.inventory(), /personal_storage_namespace_invalid/);
  assert.equal(f.cacheCalls(), 0);
});

test("日志或浏览器根偏离当前个人命名空间时不扫描", async t => {
  const f = fixture(t);
  f.paths.logs = f.root;
  await assert.rejects(f.inventory.inventory(), /personal_storage_namespace_invalid/);
  assert.equal(f.cacheCalls(), 0);
});

test("命名空间链接与目录内链接都不跟随，后者保留未知大小", async t => {
  const f = fixture(t);
  const outside = path.join(f.root, "outside");
  fs.mkdirSync(outside);
  fs.mkdirSync(f.paths.logs);
  fs.writeFileSync(path.join(outside, "not-ours.log"), "outside");
  fs.symlinkSync(outside, path.join(f.paths.logs, "linked"), "dir");
  const realReaddir = fsp.readdir;
  t.mock.method(fsp, "readdir", async (...args) => {
    assert.ok(!args[0].startsWith(outside));
    return realReaddir(...args);
  });
  const value = await f.inventory.inventory();
  assert.equal(value.sizeComplete, false);
  assert.equal(value.items[0].logicalBytes, null);
  assert.equal(value.items[0].fileCount, null);
  assert.ok(value.issues.includes("personal_storage_link_not_scanned"));
  const linkedParent = path.join(f.root, "alias");
  fs.symlinkSync(f.root, linkedParent, "dir");
  f.paths.userData = path.join(linkedParent, "cn.cilai.personal.workbench");
  f.paths.logs = path.join(f.paths.userData, "logs");
  f.paths.sessionData = path.join(f.paths.userData, "browser");
  await assert.rejects(f.inventory.inventory(), /personal_storage_namespace_invalid/);
});

test("缓存调用失败或大小无效保持未知，不导出任意异常正文", async t => {
  const f = fixture(t);
  f.session.getCacheSize = async () => { throw new Error("private-host-message https://secret.invalid/token"); };
  const unavailable = await f.inventory.inventory();
  assert.equal(unavailable.items[2].logicalBytes, null);
  assert.equal(unavailable.sizeComplete, false);
  assert.deepEqual(unavailable.issues, ["personal_storage_cache_unavailable"]);
  assert.ok(!JSON.stringify(unavailable).includes("secret.invalid"));
  f.session.getCacheSize = async () => { throw new Error("personal_storage_private_api_credential"); };
  assert.deepEqual((await f.inventory.inventory()).issues, ["personal_storage_cache_unavailable"]);
  f.session.getCacheSize = async () => -1;
  const invalid = await f.inventory.inventory();
  assert.equal(invalid.items[2].logicalBytes, null);
  assert.deepEqual(invalid.issues, ["personal_storage_cache_size_invalid"]);
});

test("缓存句柄不能指向其他资料分区或默认浏览器根", async t => {
  const f = fixture(t);
  f.session.storagePath = path.join(f.paths.sessionData, "Partitions", `ai-browser-${OTHER_PROFILE}`);
  const value = await f.inventory.inventory();
  assert.equal(value.items[2].logicalBytes, null);
  assert.deepEqual(value.issues, ["personal_storage_browser_scope_invalid"]);
  assert.equal(f.cacheCalls(), 0);
});

test("未曾初始化的浏览器不创建Session，已观察的Session关闭标签后仍可只读统计", async t => {
  const f = fixture(t);
  f.browser.tabs.clear();
  const before = await f.inventory.inventory();
  assert.equal(before.items[2].logicalBytes, null);
  assert.equal(before.coverage[2].state, "not_scanned");
  assert.equal(before.coverage[2].reason, "personal_storage_browser_not_initialized");
  assert.equal(f.cacheCalls(), 0);
  f.browser.tabs.set("existing", { contents: { isDestroyed: () => false, session: f.session } });
  assert.equal((await f.inventory.inventory()).items[2].logicalBytes, 4096);
  f.browser.tabs.clear();
  assert.equal((await f.inventory.inventory()).items[2].logicalBytes, 4096);
  assert.equal(f.cacheCalls(), 2);
});

test("统计在途切换资料不返回跨资料混合结果", async t => {
  const f = fixture(t);
  f.session.getCacheSize = async () => { f.setProfile(OTHER_PROFILE); return 0; };
  await assert.rejects(f.inventory.inventory(), /personal_storage_profile_changed/);
});

test("已有标签出现不同Session时不混合统计", async t => {
  const f = fixture(t);
  f.browser.tabs.set("other", { contents: { isDestroyed: () => false, session: { storagePath: f.session.storagePath } } });
  const value = await f.inventory.inventory();
  assert.equal(value.items[2].logicalBytes, null);
  assert.deepEqual(value.issues, ["personal_storage_browser_scope_invalid"]);
  assert.equal(f.cacheCalls(), 0);
});

test("稳定32位资料标识不额外限定UUID版本", async t => {
  const f = fixture(t);
  const profile = "a".repeat(32);
  f.browser.tabs.clear();
  f.setProfile(profile);
  f.browser.profileId = profile;
  assert.equal((await f.inventory.inventory()).profileId, profile);
});

test("日志在统计中变化不能把部分大小记成完整", async t => {
  const f = fixture(t);
  fs.mkdirSync(f.paths.logs);
  const file = path.join(f.paths.logs, "host-service.log");
  fs.writeFileSync(file, "before");
  const lstat = fsp.lstat;
  let seen = false;
  t.mock.method(fsp, "lstat", async (...args) => {
    const value = await lstat(...args);
    if (args[0] === file && !seen) { seen = true; fs.appendFileSync(file, "changed"); }
    return value;
  });
  const value = await f.inventory.inventory();
  assert.equal(value.items[0].logicalBytes, null);
  assert.equal(value.sizeComplete, false);
  assert.deepEqual(value.issues, ["personal_storage_inventory_changed"]);
});

test("显式本机Electron探针核对真实分区路径和缓存API", { skip: process.env.CILAI_PERSONAL_NAMESPACE_NATIVE !== "1", timeout: 55_000 }, async t => {
  const f = fixture(t);
  const { spawnSync } = require("node:child_process");
  const executable = require("electron");
  const project = path.resolve(__dirname, "../../../../..");
  assert.ok(fs.realpathSync(executable).startsWith(`${project}${path.sep}`));
  fs.rmdirSync(f.session.storagePath);
  for (const name of ["home", "tmp"]) fs.mkdirSync(path.join(f.root, name));
  const script = path.join(f.root, "electron-probe.cjs");
  fs.writeFileSync(script, `const fs=require('node:fs');const path=require('node:path');const {app}=require('electron');let stage='initialization';
const root=${JSON.stringify(f.paths.userData)};app.setName('CilaiPersonalStorageProbe');app.disableHardwareAcceleration();
app.on('window-all-closed',()=>{});
for(const [name,where] of [['userData',root],['sessionData',path.join(root,'browser')],['logs',path.join(root,'logs')]]){fs.mkdirSync(where,{recursive:true});app.setPath(name,where)}
fs.mkdirSync(path.join(root,'test-diagnostics'));fs.writeFileSync(path.join(root,'logs/host-service.log'),'abc');fs.writeFileSync(path.join(root,'test-diagnostics/events.log'),'12345');
app.whenReady().then(async()=>{const {BrowserRuntime}=require(${JSON.stringify(path.join(__dirname, "browser.js"))});
const browser=new BrowserRuntime({});browser.setProfile(${JSON.stringify(PROFILE)});
const {createPersonalNamespaceStorage}=require(${JSON.stringify(path.join(__dirname, "personal-namespace-storage.js"))});
const probe=createPersonalNamespaceStorage({app,browser,getVerifiedProfileId:()=>${JSON.stringify(PROFILE)}});browser.sessionOf=()=>{throw Error('inventory_must_not_create_session')};
const target=path.join(root,'browser/Partitions/ai-browser-${PROFILE}');
stage='uninitialized-inventory';const existedBefore=fs.existsSync(target);const initial=await probe.inventory();const existsAfter=fs.existsSync(target);
if(existedBefore||existsAfter||initial.items[2].logicalBytes!==null||initial.coverage[2].state!=='not_scanned')throw Error('readonly_initialization_failed');
stage='explicit-webcontents';const {BrowserWindow}=require('electron');const window=new BrowserWindow({show:false,webPreferences:{partition:'persist:ai-browser-${PROFILE}',sandbox:true}});
browser.tabs.set('explicit-probe',{contents:window.webContents});const value=await probe.inventory();window.destroy();browser.tabs.clear();
stage='retained-session';const retained=await probe.inventory();
console.log('NAMESPACE_STORAGE_PROBE '+JSON.stringify({electron:process.versions.electron,platform:process.platform,uninitializedPartitionCreated:existsAfter,uninitializedCacheBytes:initial.items[2].logicalBytes,uninitializedState:initial.coverage[2].state,cacheBytes:value.items[2].logicalBytes,sizeComplete:value.sizeComplete,logicalBytes:value.knownLogicalBytes,retainedCacheBytes:retained.items[2].logicalBytes,issues:value.issues}));app.quit();
}).catch(()=>{console.error('personal_storage_native_probe_failed '+stage);app.exit(1)});`);
  const result = spawnSync(executable, [script, `--user-data-dir=${f.paths.userData}`], {
    cwd: f.root, timeout: 45_000, encoding: "utf8", maxBuffer: 1024 * 1024,
    env: { HOME: path.join(f.root, "home"), USERPROFILE: path.join(f.root, "home"), TMPDIR: path.join(f.root, "tmp"),
      TEMP: path.join(f.root, "tmp"), TMP: path.join(f.root, "tmp"), PATH: "/usr/bin:/bin:/usr/sbin:/sbin", LANG: "en_US.UTF-8" },
  });
  assert.equal(result.status, 0, result.stderr);
  const marker = result.stdout.split("\n").find(line => line.startsWith("NAMESPACE_STORAGE_PROBE "));
  assert.ok(marker, "真实Electron探针没有返回结构化结果");
  const value = JSON.parse(marker.slice("NAMESPACE_STORAGE_PROBE ".length));
  assert.equal(value.sizeComplete, true, JSON.stringify(value));
  assert.equal(value.uninitializedPartitionCreated, false);
  assert.equal(value.uninitializedCacheBytes, null);
  assert.equal(value.uninitializedState, "not_scanned");
  assert.ok(Number.isSafeInteger(value.cacheBytes) && value.cacheBytes >= 0);
  assert.equal(value.logicalBytes, value.cacheBytes + 8);
  assert.deepEqual(value.issues, []);
  assert.ok(Number.isSafeInteger(value.retainedCacheBytes) && value.retainedCacheBytes >= 0);
  console.log(JSON.stringify({ nativeNamespaceStorage: value }));
});
