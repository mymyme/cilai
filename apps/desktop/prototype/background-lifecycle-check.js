const { app, BrowserWindow, Tray, Menu, nativeImage, autoUpdater } = require("electron");
const assert = require("node:assert/strict");
const { mkdtempSync } = require("node:fs");
const { tmpdir } = require("node:os");
const path = require("node:path");
const { spawn } = require("node:child_process");
const { once } = require("node:events");
const { createBackgroundLifecycle } = require("../shell/background-lifecycle");

app.setName("词来后台驻留验证");
app.setPath("userData", mkdtempSync(path.join(tmpdir(), "cilai-background-check-")));
let win;
let child;
let ticks = 0;
let cleanupCount = 0;
let childExit;
const checks = [];
const lifecycle = createBackgroundLifecycle({
  app, Tray, Menu, nativeImage, nativeUpdater: autoUpdater,
  getWindow: () => win,
  cleanup: () => { cleanupCount++; child?.kill(); },
});
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const deadline = setTimeout(() => { console.error("后台驻留验证超时"); child?.kill(); app.exit(1); }, 30000);

app.whenReady().then(async () => {
  child = spawn(process.execPath, ["-e", "setInterval(() => process.stdout.write('tick\\n'), 100)"], {
    env: { ...process.env, ELECTRON_RUN_AS_NODE: "1" }, stdio: ["ignore", "pipe", "pipe"],
  });
  childExit = once(child, "exit");
  child.stdout.on("data", (bytes) => { ticks += bytes.toString().split("tick").length - 1; });
  win = new BrowserWindow({ width: 640, height: 440, show: false,
    webPreferences: { contextIsolation: true, nodeIntegration: false, backgroundThrottling: false } });
  lifecycle.bindWindow(win);
  lifecycle.createTray();
  await win.loadURL("data:text/html;charset=utf-8," + encodeURIComponent(
    '<title>后台驻留验证</title><input id="draft" value="未提交的草稿"><script>window.ticks=0;setInterval(()=>window.ticks++,100)</script>',
  ));
  lifecycle.showWindow();
  await delay(250);
  const id = win.id;
  const rendererPid = win.webContents.getOSProcessId();
  const childPid = child.pid;
  const initialTicks = ticks;
  const initialRendererTicks = await win.webContents.executeJavaScript("window.ticks");
  win.close();
  await delay(1200);
  assert.equal(win.isDestroyed(), false);
  assert.equal(win.isVisible(), false);
  assert.equal(cleanupCount, 0);
  assert.ok(ticks > initialTicks + 5);
  assert.ok(await win.webContents.executeJavaScript("window.ticks") > initialRendererTicks + 5);
  checks.push("关闭后原窗口隐藏，页面计时器和子进程持续运行");

  app.emit("activate");
  assert.equal(win.isVisible(), true);
  assert.equal(win.id, id);
  assert.equal(win.webContents.getOSProcessId(), rendererPid);
  assert.equal(child.pid, childPid);
  assert.equal(await win.webContents.executeJavaScript('document.getElementById("draft").value'), "未提交的草稿");
  checks.push("恢复原窗口、原渲染进程、原子进程及未提交草稿");

  await win.webContents.executeJavaScript('window.onbeforeunload = (event) => { event.returnValue = false; return false; }; undefined');
  win.close();
  const veto = once(win.webContents, "will-prevent-unload");
  app.quit();
  await veto;
  assert.equal(cleanupCount, 0);
  assert.equal(win.isVisible(), true);
  assert.equal(child.exitCode, null);
  await win.webContents.executeJavaScript("window.onbeforeunload = null");
  win.close();
  assert.equal(win.isDestroyed(), false);
  assert.equal(win.isVisible(), false);
  checks.push("草稿阻止退出后服务未清理，再次关闭仍隐藏");

  app.once("will-quit", (event) => {
    event.preventDefault();
    void (async () => {
      assert.equal(cleanupCount, 1);
      await childExit;
      checks.push("主动退出销毁窗口并清理子进程");
      console.log(JSON.stringify({ platform: process.platform, electron: process.versions.electron, checks }, null, 2));
      clearTimeout(deadline);
      app.exit(0);
    })().catch(fail);
  });
  app.quit();
}).catch(fail);

function fail(error) {
  console.error(error);
  clearTimeout(deadline);
  child?.kill();
  app.exit(1);
}
