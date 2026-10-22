const UPDATE_EVENT = "updater:state";
const { validateChannel, configureChannel } = require('./update-channels');
const { compareVersions } = require('./portable-update');
const CHECK_INTERVAL_MS = 4 * 60 * 60 * 1000;
const STARTUP_DELAY_MS = 45 * 1000;

function messageOf(error) {
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  const text = String(error ?? "").trim();
  return text || "更新操作失败";
}

function releaseNotesText(value) {
  if (typeof value === "string") return value.trim().slice(0, 2_000) || null;
  if (!Array.isArray(value)) return null;
  const text = value
    .map((entry) => typeof entry?.note === "string" ? entry.note.trim() : "")
    .filter(Boolean)
    .join("\n");
  return text.slice(0, 2_000) || null;
}

/**
 * Electron 更新控制器。
 *
 * 更新源只能来自包内的固定通道配置。渲染进程能触发检查与下载，但不能传入
 * 地址、文件路径或校验值，避免把受信任更新器变成任意下载与执行入口。
 */
class WorkbenchUpdater {
  constructor({ autoUpdater, currentVersion, enabled, broadcast, logger = console,
    configuration = null, preferences = null, downloadPreferences = null, beforeInstall = async () => {},
    startupDelayMs = STARTUP_DELAY_MS, createCancellationToken = () => new (require('electron-updater').CancellationToken)() }) {
    this.autoUpdater = autoUpdater;
    this.enabled = Boolean(enabled) && configuration?.distribution !== 'local';
    this.broadcast = broadcast;
    this.logger = logger;
    this.started = false;
    this.installing = false;
    this.startupCheckStarted = false;
    this.configuration = configuration;
    this.preferences = preferences;
    this.beforeInstall = beforeInstall;
    this.downloadPreferences = downloadPreferences;
    this.startupDelayMs = startupDelayMs;
    this.createCancellationToken = createCancellationToken;
    this.timer = null;
    this.nextCheckAt = null;
    const downloadSettings = downloadPreferences?.load();
    this.pausedCandidate = downloadSettings?.pausedCandidate ?? null;
    this.operation = null;
    this.candidate = null;
    const currentChannel = configuration?.buildChannel ?? 'stable';
    this.state = {
      supported: this.enabled,
      channel: currentChannel,
      selectedChannel: preferences?.load(currentChannel) ?? currentChannel,
      currentChannel,
      phase: "idle",
      currentVersion,
      availableVersion: null,
      latestVersion: null,
      releaseName: null,
      releaseNotes: null,
      percent: null,
      transferred: null,
      total: null,
      bytesPerSecond: null,
      error: null,
      checkedAt: null,
      trigger: null,
      autoDownload: downloadSettings?.autoDownload ?? true,
    };
  }

  snapshot() {
    return { ...this.state };
  }

  setState(patch) {
    this.state = { ...this.state, ...patch };
    this.broadcast(this.snapshot());
  }

  bindEvents() {
    this.autoUpdater.on("checking-for-update", () => {
      this.setState({ phase: "checking", error: null });
    });
    this.autoUpdater.on("update-available", (info) => {
      if (!this.validChannelInfo(info)) return;
      try {
        if (compareVersions(info.version, this.state.currentVersion) <= 0
          && this.state.channel === this.state.currentChannel) {
          this.noUpdate(info);
          return;
        }
      } catch (error) { this.fail(error); return; }
      this.candidate = { channel: this.state.channel, version: info.version };
      this.setState({
        phase: "available",
        availableVersion: String(info?.version ?? "").trim() || null,
        latestVersion: info.version,
        releaseName: typeof info?.releaseName === "string" ? info.releaseName.trim() || null : null,
        releaseNotes: releaseNotesText(info?.releaseNotes),
        percent: null,
        transferred: null,
        total: null,
        bytesPerSecond: null,
        error: null,
        checkedAt: Date.now(),
      });
    });
    this.autoUpdater.on("update-not-available", (info) => this.noUpdate(info));
    this.autoUpdater.on("download-progress", (progress) => {
      if (this.state.phase !== 'downloading' || this.candidate?.channel !== this.state.channel) return;
      this.setState({
        phase: "downloading",
        percent: Number.isFinite(progress?.percent) ? Math.max(0, Math.min(100, progress.percent)) : null,
        transferred: Number.isFinite(progress?.transferred) ? progress.transferred : null,
        total: Number.isFinite(progress?.total) ? progress.total : null,
        bytesPerSecond: Number.isFinite(progress?.bytesPerSecond) ? progress.bytesPerSecond : null,
        error: null,
      });
    });
    this.autoUpdater.on("update-downloaded", (info) => {
      if (info?.cilaiChannel && info.cilaiChannel !== this.state.channel) return;
      if (this.configuration?.distribution === 'native' && this.state.channel === 'testing'
        && info?.cilaiChannel !== 'testing') return;
      if (this.candidate?.channel !== this.state.channel || this.candidate.version !== info?.version) return;
      this.setState({
        phase: "downloaded",
        availableVersion: String(info?.version ?? this.state.availableVersion ?? "").trim() || null,
        releaseName: typeof info?.releaseName === "string"
          ? info.releaseName.trim() || this.state.releaseName
          : this.state.releaseName,
        releaseNotes: releaseNotesText(info?.releaseNotes) ?? this.state.releaseNotes,
        percent: 100,
        error: null,
      });
    });
    this.autoUpdater.on("error", (error) => { if (!this.downloadToken?.cancelled) this.fail(error); });
  }

  noUpdate(info) {
    if (!this.validChannelInfo(info)) return;
    this.candidate = null;
    this.setState({ phase: 'idle', availableVersion: null, latestVersion: info?.version ?? null,
      releaseName: null, releaseNotes: null, percent: null, transferred: null, total: null,
      bytesPerSecond: null, error: null, checkedAt: Date.now() });
  }

  validChannelInfo(info) {
    if ((info?.cilaiChannel && info.cilaiChannel !== this.state.channel)
      || (this.configuration?.distribution === 'native' && this.state.channel === 'testing' && info?.cilaiChannel !== 'testing')) {
      this.fail(new Error('更新包与所选通道不匹配'));
      return false;
    }
    return true;
  }

  selectChannel(channel) {
    validateChannel(channel);
    if (!this.enabled || !this.configuration) throw new Error('当前运行方式不能选择更新通道');
    if (this.installing || this.operation || ['checking', 'downloading'].includes(this.state.phase)) throw new Error('请等待当前更新操作完成后再切换通道');
    if (channel === this.state.selectedChannel) return this.snapshot();
    const previous = this.state.channel;
    configureChannel(this.autoUpdater, this.configuration, channel);
    try { this.preferences?.save(channel); }
    catch (error) {
      configureChannel(this.autoUpdater, this.configuration, previous);
      this.candidate = null;
      this.setState({ phase: 'error', availableVersion: null, latestVersion: null, checkedAt: null, error: messageOf(error) });
      throw error;
    }
    this.candidate = null;
    this.setState({ channel, selectedChannel: channel, phase: 'idle', availableVersion: null, latestVersion: null,
      releaseName: null, releaseNotes: null, percent: null, transferred: null, total: null,
      bytesPerSecond: null, error: null, checkedAt: null, trigger: null });
    return this.snapshot();
  }

  prepare() {
    if (this.started || !this.enabled) return;
    this.started = true;
    this.autoUpdater.autoDownload = false;
    this.autoUpdater.autoInstallOnAppQuit = false;
    this.autoUpdater.allowPrerelease = false;
    if (this.configuration) configureChannel(this.autoUpdater, this.configuration, this.state.channel);
    this.autoUpdater.allowDowngrade = this.state.channel !== this.state.currentChannel;
    this.bindEvents();
  }

  fail(error) {
    if (this.installing) {
      this.releaseInstall?.();
      this.releaseInstall = null;
      this.installing = false;
    }
    const message = messageOf(error);
    this.logger.error?.(`[updater] ${message}`);
    this.setState({ phase: "error", error: message });
  }

  async start() {
    if (this.startupCheckStarted || !this.enabled) return this.snapshot();
    this.startupCheckStarted = true;
    this.prepare();
    if (this.nextCheckAt !== null) return this.snapshot();
    if (this.startupDelayMs === 0) return this.check('startup');
    this.schedule(this.startupDelayMs, 'startup');
    return this.snapshot();
  }

  schedule(delay = CHECK_INTERVAL_MS, trigger = 'interval') {
    clearTimeout(this.timer);
    this.nextCheckAt = Date.now() + delay;
    this.timer = setTimeout(() => {
      this.schedule();
      void this.check(trigger).catch(error => this.fail(error));
    }, delay);
    this.timer.unref?.();
  }

  stop() { clearTimeout(this.timer); this.timer = null; }

  async checkDue(trigger) {
    if (!this.enabled || this.nextCheckAt === null || Date.now() < this.nextCheckAt) return this.snapshot();
    return this.check(trigger);
  }

  setAutoDownload(enabled) {
    if (!this.enabled || typeof enabled !== 'boolean') throw new Error('自动下载设置无效');
    this.downloadPreferences?.save({ autoDownload: enabled, pausedCandidate: this.pausedCandidate });
    this.setState({ autoDownload: enabled });
    if (enabled) this.maybeDownload();
    return this.snapshot();
  }

  maybeDownload() {
    if (!this.operation && this.state.phase === 'available' && this.state.autoDownload
      && this.state.channel === this.state.currentChannel
      && this.pausedCandidate !== `${this.state.channel}:${this.state.availableVersion}`) {
      void this.download().catch(error => this.fail(error));
    }
  }

  pause() {
    if (this.state.phase !== 'downloading' || !this.downloadToken) return this.snapshot();
    const pausedCandidate = `${this.state.channel}:${this.state.availableVersion}`;
    this.downloadPreferences?.save({ autoDownload: this.state.autoDownload, pausedCandidate });
    this.pausedCandidate = pausedCandidate;
    this.downloadToken.cancel();
    // Only expose the resumable state after the active writer has closed.
    return this.downloadPromise.then(() => this.snapshot());
  }

  async check(trigger = "platform") {
    if (!this.enabled) return this.snapshot();
    this.prepare();
    if (this.installing || this.operation || ["checking", "downloading", "downloaded", "paused"].includes(this.state.phase)) return this.snapshot();
    const manualSelection = trigger === 'settings';
    // Background notifications never change a cross-channel candidate which the
    // employee is already reviewing. They never query that candidate's channel.
    if (!manualSelection && this.candidate && this.state.channel !== this.state.currentChannel) return this.snapshot();
    const channel = manualSelection ? this.state.selectedChannel : this.state.currentChannel;
    this.schedule();
    this.operation = 'check';
    this.candidate = null;
    this.setState({ channel, phase: "checking", availableVersion: null, latestVersion: null,
      releaseName: null, releaseNotes: null, percent: null, transferred: null, total: null,
      bytesPerSecond: null, error: null, trigger });
    try {
      if (this.configuration) configureChannel(this.autoUpdater, this.configuration, channel);
      await this.autoUpdater.checkForUpdates();
      if (this.state.phase === 'available') await this.autoUpdater.restoreDownloadedUpdate?.();
      if (this.state.phase === 'available' && this.pausedCandidate === `${channel}:${this.state.availableVersion}`) this.setState({ phase: 'paused' });
    } catch (error) {
      this.fail(error);
    } finally { this.operation = null; }
    this.maybeDownload();
    return this.snapshot();
  }

  async download() {
    if (this.operation) throw new Error('更新操作正在执行，请稍候');
    const canDownload = ['available', 'paused'].includes(this.state.phase)
      || (this.state.phase === "error" && this.state.availableVersion !== null);
    if (!canDownload) throw new Error("当前没有可下载的更新");
    if (this.candidate?.channel !== this.state.channel) throw new Error('更新通道已变化，请重新检查');
    this.downloadPreferences?.save({ autoDownload: this.state.autoDownload, pausedCandidate: null });
    const token = this.createCancellationToken();
    this.operation = 'download';
    this.pausedCandidate = null;
    this.downloadToken = token;
    this.setState({
      phase: "downloading",
      percent: 0,
      transferred: 0,
      total: null,
      bytesPerSecond: null,
      error: null,
    });
    this.downloadPromise = (async () => {
      try {
        await this.autoUpdater.downloadUpdate(token);
      } catch (error) {
        if (!token.cancelled) this.fail(error);
      } finally {
        if (token.cancelled && this.state.phase !== 'downloaded') this.setState({ phase: 'paused', error: null, bytesPerSecond: null });
        this.operation = null;
        this.downloadToken = null;
      }
      return this.snapshot();
    })();
    return this.downloadPromise;
  }

  async install() {
    if (this.state.phase !== "downloaded") throw new Error("更新尚未下载完成");
    if (this.candidate?.channel !== this.state.channel || this.candidate.version !== this.state.availableVersion) throw new Error('更新通道已变化，请重新检查');
    if (this.installing) throw new Error("正在重启更新，请稍候");
    this.installing = true;
    let release;
    try {
      release = await this.beforeInstall();
    } catch (error) {
      this.installing = false;
      this.setState({ error: messageOf(error) });
      throw error;
    }
    try {
      this.releaseInstall = release;
      await this.autoUpdater.quitAndInstall(false, true);
    } catch (error) {
      this.fail(error);
      throw error;
    }
  }

  registerIpc(ipcMain, isAllowedSender) {
    const allowed = (event) => {
      if (!isAllowedSender(event.sender)) throw new Error("更新操作来源不受信任");
    };
    ipcMain.handle("updater:get-state", (event) => {
      allowed(event);
      return this.snapshot();
    });
    ipcMain.handle("updater:check", (event) => {
      allowed(event);
      return this.check("platform");
    });
    ipcMain.handle('updater:check-selected-channel', (event) => {
      allowed(event);
      return this.check('settings');
    });
    ipcMain.handle('updater:select-channel', (event, channel) => {
      allowed(event);
      return this.selectChannel(channel);
    });
    ipcMain.handle("updater:download", async (event) => {
      allowed(event);
      if (this.operation) throw new Error('更新操作正在执行，请稍候');
      const pending = this.download();
      if (this.operation !== 'download') return pending;
      void pending.catch(error => this.fail(error));
      return this.snapshot();
    });
    ipcMain.handle('updater:pause', event => { allowed(event); return this.pause(); });
    ipcMain.handle('updater:set-auto-download', (event, enabled) => { allowed(event); return this.setAutoDownload(enabled); });
    ipcMain.handle('updater:network-restored', event => { allowed(event); return this.checkDue('network'); });
    ipcMain.handle("updater:install", async (event) => {
      allowed(event);
      await this.install();
      return null;
    });
  }
}

function createWorkbenchUpdater(options) {
  return new WorkbenchUpdater(options);
}

module.exports = { UPDATE_EVENT, WorkbenchUpdater, createWorkbenchUpdater, releaseNotesText, CHECK_INTERVAL_MS, STARTUP_DELAY_MS };
