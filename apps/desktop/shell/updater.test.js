const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const test = require("node:test");
const { WorkbenchUpdater, releaseNotesText, CHECK_INTERVAL_MS, STARTUP_DELAY_MS } = require("./updater");

class FakeAutoUpdater extends EventEmitter {
  constructor() {
    super();
    this.checks = 0;
    this.downloads = 0;
    this.installs = [];
  }

  async checkForUpdates() {
    this.checks += 1;
  }

  async downloadUpdate() {
    this.downloads += 1;
  }

  quitAndInstall(...args) {
    this.installs.push(args);
  }
}

function setup(enabled = true, options = {}) {
  const autoUpdater = new FakeAutoUpdater();
  const states = [];
  const errors = [];
  const controller = new WorkbenchUpdater({
    autoUpdater,
    currentVersion: "1.0.0",
    enabled,
    startupDelayMs: 0,
    downloadPreferences: { load: () => ({ autoDownload: false }), save() {} },
    broadcast: (state) => states.push(state),
    logger: { error: (message) => errors.push(message) },
    ...options,
  });
  return { autoUpdater, controller, errors, states };
}

test("开发运行不访问正式更新源", async () => {
  const { autoUpdater, controller } = setup(false);
  const state = await controller.start();
  assert.equal(autoUpdater.checks, 0);
  assert.equal(state.supported, false);
});

test("启动只安排一次检查，下载由控制器调度且退出不会自动安装", async () => {
  const { autoUpdater, controller } = setup();
  await controller.start();
  await controller.start();
  assert.equal(autoUpdater.checks, 1);
  assert.equal(autoUpdater.autoDownload, false);
  assert.equal(autoUpdater.autoInstallOnAppQuit, false);
  assert.equal(controller.snapshot().trigger, "startup");
});

test("窗口加载完成前收到检查信号时先建立事件监听", async () => {
  const { autoUpdater, controller } = setup();
  await controller.check("platform");
  autoUpdater.emit("update-available", { version: "1.1.0" });
  await controller.start();
  assert.equal(autoUpdater.checks, 1);
  assert.equal(controller.snapshot().phase, "available");
  assert.equal(autoUpdater.listenerCount("update-available"), 1);
});

test("发现更新后由员工下载，完成后才能重启安装", async () => {
  const { autoUpdater, controller } = setup();
  await controller.start();
  autoUpdater.emit("update-available", {
    version: "1.1.0",
    releaseName: "稳定版",
    releaseNotes: [{ note: "修复启动问题" }, { note: "优化更新提示" }],
  });
  assert.equal(controller.snapshot().phase, "available");
  assert.equal(controller.snapshot().releaseNotes, "修复启动问题\n优化更新提示");

  await controller.download();
  assert.equal(autoUpdater.downloads, 1);
  assert.equal(controller.snapshot().phase, "downloading");
  autoUpdater.emit("download-progress", { percent: 42.6, transferred: 426, total: 1_000, bytesPerSecond: 100 });
  assert.equal(controller.snapshot().percent, 42.6);
  autoUpdater.emit("update-downloaded", { version: "1.1.0" });
  assert.equal(controller.snapshot().phase, "downloaded");

  await controller.install();
  assert.deepEqual(autoUpdater.installs, [[false, true]]);
});

test("异步更新助手失败会保留明确错误并允许重新下载", async () => {
  const { autoUpdater, controller } = setup();
  await controller.start();
  autoUpdater.emit('update-available', { version: '1.1.0' });
  autoUpdater.emit("update-downloaded", { version: "1.1.0" });
  autoUpdater.quitAndInstall = async () => { throw new Error("助手启动失败"); };
  await assert.rejects(controller.install(), /助手启动失败/);
  assert.equal(controller.snapshot().phase, "error");
  assert.equal(controller.snapshot().availableVersion, "1.1.0");
});

test("连续点击只启动一次更新助手", async () => {
  const { autoUpdater, controller } = setup();
  await controller.start();
  autoUpdater.emit('update-available', { version: '1.1.0' });
  autoUpdater.emit("update-downloaded", { version: "1.1.0" });
  let finish;
  autoUpdater.quitAndInstall = () => new Promise((resolve) => { finish = resolve; });
  const first = controller.install();
  await assert.rejects(controller.install(), /正在重启更新/);
  finish();
  await first;
});

test("下载失败保留目标版本并允许明确重试", async () => {
  const { autoUpdater, controller, errors } = setup();
  await controller.start();
  autoUpdater.emit("update-available", { version: "1.1.0" });
  autoUpdater.downloadUpdate = async () => { throw new Error("连接中断"); };
  await controller.download();
  assert.equal(controller.snapshot().phase, "error");
  assert.equal(controller.snapshot().availableVersion, "1.1.0");
  assert.match(controller.snapshot().error, /连接中断/);
  assert.equal(errors.length, 1);
});

test("渲染进程只能从主窗口调用固定更新操作", async () => {
  const { controller } = setup();
  const handlers = new Map();
  controller.registerIpc({ handle: (name, handler) => handlers.set(name, handler) }, (sender) => sender === "main");
  assert.deepEqual(
    [...handlers.keys()],
    ["updater:get-state", "updater:check", "updater:check-selected-channel", "updater:select-channel", "updater:download", "updater:pause", "updater:set-auto-download", "updater:network-restored", "updater:install"],
  );
  assert.throws(() => handlers.get("updater:get-state")({ sender: "other" }), /来源不受信任/);
  assert.equal(handlers.get("updater:get-state")({ sender: "main" }).currentVersion, "1.0.0");
});

test('启动延迟 45 秒，运行每 4 小时检查；提前唤醒和恢复网络不重复检查', async t => {
  t.mock.timers.enable({ apis: ['setTimeout', 'Date'], now: 1000 });
  const { controller, autoUpdater } = setup(true, { startupDelayMs: STARTUP_DELAY_MS });
  t.after(() => controller.stop());
  autoUpdater.checkForUpdates = async () => { autoUpdater.checks++; autoUpdater.emit('update-not-available', { version: '1.0.0' }); };
  await controller.start(); await controller.start();
  assert.equal(autoUpdater.checks, 0);
  t.mock.timers.tick(STARTUP_DELAY_MS - 1);
  await controller.checkDue('network');
  assert.equal(autoUpdater.checks, 0);
  t.mock.timers.tick(1);
  await Promise.resolve();
  assert.equal(autoUpdater.checks, 1);
  t.mock.timers.tick(CHECK_INTERVAL_MS - 1);
  await controller.checkDue('resume');
  assert.equal(autoUpdater.checks, 1);
  t.mock.timers.tick(1);
  await Promise.resolve();
  assert.equal(autoUpdater.checks, 2);
  // Sleep advances wall time without executing timer callbacks.
  t.mock.timers.setTime(Date.now() + CHECK_INTERVAL_MS);
  await controller.checkDue('resume');
  assert.equal(autoUpdater.checks, 3);
  await controller.checkDue('network');
  assert.equal(autoUpdater.checks, 3);
});

test('默认后台下载同通道更新，关闭后手动下载，跨通道不自动下载', async () => {
  const { controller, autoUpdater } = setup(true, { downloadPreferences: null });
  autoUpdater.checkForUpdates = async () => autoUpdater.emit('update-available', { version: '1.1.0' });
  await controller.check();
  assert.equal(autoUpdater.downloads, 1);
  assert.equal(controller.snapshot().phase, 'downloading');
  assert.equal(autoUpdater.autoInstallOnAppQuit, false);
  controller.stop();
  const manual = setup();
  manual.autoUpdater.checkForUpdates = async () => manual.autoUpdater.emit('update-available', { version: '1.1.0' });
  await manual.controller.check();
  assert.equal(manual.autoUpdater.downloads, 0);
  manual.controller.state.channel = 'testing';
  manual.controller.setAutoDownload(true);
  assert.equal(manual.autoUpdater.downloads, 0);
  manual.controller.stop();
});

test('暂停等待下载退出，后台信号不覆盖暂停，继续下载才重新启动', async () => {
  const { controller, autoUpdater } = setup();
  await controller.start(); autoUpdater.emit('update-available', { version: '1.1.0' });
  autoUpdater.downloadUpdate = token => new Promise((resolve, reject) => { autoUpdater.downloads++; token.once('cancel', () => reject(new Error('cancelled'))); });
  const downloading = controller.download();
  await controller.pause(); await downloading;
  assert.equal(controller.snapshot().phase, 'paused');
  assert.equal(controller.snapshot().error, null);
  const checks = autoUpdater.checks;
  await controller.check('platform');
  assert.equal(autoUpdater.checks, checks);
  autoUpdater.downloadUpdate = async () => { autoUpdater.downloads++; autoUpdater.emit('update-downloaded', { version: '1.1.0' }); };
  await controller.download();
  assert.equal(autoUpdater.downloads, 2);
  assert.equal(controller.snapshot().phase, 'downloaded');
  controller.stop();
});

test('任务保护失败保留已下载状态，任务结束后可直接重启', async () => {
  let busy = true;
  const { controller, autoUpdater } = setup(true, { beforeInstall: async () => { if (busy) throw new Error('仍有 AI 任务'); } });
  await controller.start();
  autoUpdater.emit('update-available', { version: '1.1.0' }); autoUpdater.emit('update-downloaded', { version: '1.1.0' });
  await assert.rejects(controller.install(), /仍有 AI 任务/);
  assert.equal(controller.snapshot().phase, 'downloaded');
  assert.equal(autoUpdater.installs.length, 0);
  busy = false; await controller.install();
  assert.equal(autoUpdater.installs.length, 1);
  controller.stop();
});

test('原生更新器通过错误事件报告安装失败时解除任务保护', async () => {
  let released = false;
  const { controller, autoUpdater } = setup(true, { beforeInstall: async () => () => { released = true; } });
  await controller.start();
  autoUpdater.emit('update-available', { version: '1.1.0' }); autoUpdater.emit('update-downloaded', { version: '1.1.0' });
  autoUpdater.quitAndInstall = () => autoUpdater.emit('error', new Error('安装器启动失败'));
  await controller.install();
  assert.equal(released, true);
  assert.equal(controller.installing, false);
  assert.equal(controller.snapshot().phase, 'error');
  controller.stop();
});

test('员工暂停的版本跨启动保留，继续下载才清除暂停偏好', async () => {
  let saved = { autoDownload: true, pausedCandidate: 'stable:1.1.0' };
  const { controller, autoUpdater } = setup(true, { downloadPreferences: { load: () => saved, save: value => { saved = value; } } });
  autoUpdater.checkForUpdates = async () => autoUpdater.emit('update-available', { version: '1.1.0' });
  await controller.check();
  assert.equal(controller.snapshot().phase, 'paused');
  assert.equal(autoUpdater.downloads, 0);
  await controller.download();
  assert.equal(saved.pausedCandidate, null);
  assert.equal(autoUpdater.downloads, 1);
  controller.stop();
});

test("发布说明只保留可显示文本并限制长度", () => {
  assert.equal(releaseNotesText([{ note: "第一项" }, {}, { note: "第二项" }]), "第一项\n第二项");
  assert.equal(releaseNotesText(null), null);
  assert.equal(releaseNotesText("x".repeat(2_100)).length, 2_000);
});
