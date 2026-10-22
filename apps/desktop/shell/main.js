const { app, BrowserWindow, ipcMain, clipboard, dialog, protocol, net, screen,
  Tray, Menu, nativeImage, powerMonitor, Notification, autoUpdater: nativeUpdater } = require("electron");
const { autoUpdater } = require("electron-updater");
const path = require("node:path");
const PERSONAL_SMOKE = process.argv.includes("--personal-smoke-check");
const HEADLESS = PERSONAL_SMOKE || process.argv.includes("--headless-check");
const { createHeadlessIsolation } = require('./headless-isolation');
const headlessIsolation = createHeadlessIsolation({ headless: HEADLESS });
app.setName(require("../package.json").productName);
const { personalDirectories } = require("./personal-isolation");
const requestedUserData = app.commandLine.getSwitchValue("user-data-dir");
if (headlessIsolation) {
  app.commandLine.removeSwitch('user-data-dir');
  app.commandLine.appendSwitch('user-data-dir', headlessIsolation.userData);
  app.setPath('userData', headlessIsolation.userData);
  app.setPath('sessionData', headlessIsolation.sessionData);
  app.setPath('logs', headlessIsolation.logs);
  console.log(`[check] 独立自检目录：${headlessIsolation.root}`);
} else {
  const directories = personalDirectories({ packaged: app.isPackaged, appData: app.getPath('appData'), requestedUserData });
  app.commandLine.removeSwitch('user-data-dir');
  app.commandLine.appendSwitch('user-data-dir', directories.userData);
  app.setPath('userData', directories.userData);
  app.setPath('sessionData', directories.sessionData);
  app.setPath('logs', directories.logs);
}
const personalHostArguments = headlessIsolation?.hostArguments ?? personalDirectories({ packaged: app.isPackaged, appData: app.getPath('appData') }).hostArguments;

const { existsSync } = require("node:fs");
const { pathToFileURL } = require("node:url");
const {
  dispatch, service, bindWindowEvents, emitToRenderer, browserMcp, browser,
  setPersonalApplicationCloser,
} = require("./bridge");
const { createWorkbenchUpdater, UPDATE_EVENT } = require("./updater");
const { loadPortableUpdater } = require("./portable-update");
const { createServiceLog } = require("./service-log");
const { createTestDiagnostics } = require("./test-diagnostics");
const { loadDataEnvironment } = require("./data-environment");
const { loadUpdateConfiguration, channelPreferences } = require('./update-channels');
const { updatePreferences } = require('./update-preferences');
const { holdServiceForUpdate, isUpdateRead } = require('./update-install-guard');
const {
  DEFAULT_WINDOW_STATE, currentWindowState, loadWindowState, saveWindowState, windowStateFile,
} = require("./window-state");
const { differentInstancePath } = require("./instance-identity");
const { pdfSelfCheckMode, runtimePdfJsAssets, runtimeStartupReady, localProfileReady,
  HEADLESS_SCOPE_MARKER, AUTHENTICATED_CHECK_MARKER } = require("./headless-check");
const { createPersonalDocuments, ORIGIN: PERSONAL_DOCUMENT_ORIGIN } = require("./personal-documents");
const { createMacosDiskAccess } = require("./macos-disk-access");
const { createBackgroundLifecycle } = require("./background-lifecycle");

// 开发期界面产物在源码树中，打包后随包置于资源目录
const DIST = app.isPackaged
  ? path.join(process.resourcesPath, "frontend")
  : path.resolve(__dirname, "../../desktop-ui/dist");

/** 与现有壳逐项一致的内容安全策略。 */
const CSP = [
  "default-src 'self'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob:",
  `frame-src 'self' data: ${PERSONAL_DOCUMENT_ORIGIN}`,
  "worker-src 'self' blob:",
].join("; ");

/**
 * 界面经自定义协议加载，不走 file 协议。
 *
 * 构建产物以绝对路径引用资源，在 file 协议下会解析到文件系统根目录而全部加载失败，
 * 表现为页面空白且无报错。现有壳用的也是自定义协议，这里保持一致。
 */
protocol.registerSchemesAsPrivileged([
  {
    scheme: "app",
    privileges: { standard: true, secure: true, supportFetchAPI: true, stream: true, corsEnabled: true },
  },
]);

// 应用名取自打包配置，避免两处各写一份。开发模式下运行时自带的应用包名为 Electron，
// 系统菜单栏的首项读的是应用包里的名字、运行时改不了；此处至少让程序坞与系统提示
// 显示产品名。打包产物没有这个问题——它有自己的应用包。


let main = null;
const activeHostOperations = new Set();
const updateConfiguration = app.isPackaged ? loadUpdateConfiguration({
  resourcesPath: process.resourcesPath, platform: process.platform, arch: process.arch,
}) : null;
const packagedDataEnvironment = updateConfiguration
  ? process.env.CILAI_DATA_ENVIRONMENT || (updateConfiguration.buildProfile === 'debug' ? 'dev' : 'prod') : null;
const personalDocuments = createPersonalDocuments(() => main && !main.isDestroyed() ? main.webContents : null);
const personalPageAi = require("./personal-page-ai").createPersonalPageAi({
  getOwner: () => main && !main.isDestroyed() ? main.webContents : null,
  documents: personalDocuments, service,
});
require("./bridge").setPersonalPageAi(personalPageAi);
const personalNotifications = require("./personal-notifications").createPersonalNotifications({
  Notification, service, getWindow: () => main, headless: HEADLESS,
  navigate: (request) => notificationNavigation.open(request),
});
const notificationNavigation = require("./personal-notification-navigation").createPersonalNotificationNavigation({
  documents: personalDocuments, service, pageAi: personalPageAi,
  getOwner: () => main && !main.isDestroyed() ? main.webContents : null,
  getProfile: () => personalNotifications.status().profileId, emit: emitToRenderer,
});
require("./bridge").setPersonalNotifications({ ...personalNotifications,
  setProfile: (id) => { if (personalNotifications.status().profileId !== id) notificationNavigation.invalidate(); personalNotifications.setProfile(id); },
});
app.on("ready", (_event, launchInfo) => personalNotifications.activation(launchInfo));
setPersonalApplicationCloser(require("./personal-application-close").createPersonalApplicationClose({
  getOwner: () => main && !main.isDestroyed() ? main.webContents : null,
  documents: personalDocuments, emit: emitToRenderer,
  revokePageAi: () => personalPageAi.revokeAll(),
}));
const macosDiskAccess = createMacosDiskAccess(() => main && !main.isDestroyed() ? main.webContents : null, (command, args) => service.call(command, args));
const backgroundLifecycle = createBackgroundLifecycle({
  app, Tray, Menu, nativeImage, nativeUpdater, headless: HEADLESS,
  getWindow: () => main,
  cleanup: () => {
    notificationNavigation.stop();
    void personalNotifications.stop();
    personalPageAi.invalidate(); personalPageAi.stop();
    personalDocuments.clear();
    browser.close();
    browserMcp.stop();
    service.stop();
  },
});
const portableUpdater = app.isPackaged && updateConfiguration?.distribution === "portable" ? loadPortableUpdater({
  resourcesPath: process.resourcesPath, platform: process.platform, currentVersion: app.getVersion(),
  executable: process.execPath, cacheRoot: path.join(app.getPath("userData"), "portable-updates"), app,
}) : null;
const workbenchUpdater = createWorkbenchUpdater({
  autoUpdater: portableUpdater ?? autoUpdater,
  currentVersion: app.getVersion(),
  // 本地测试、源码运行和无窗口自检不参与在线更新。
  enabled: app.isPackaged && !HEADLESS && Boolean(updateConfiguration) && updateConfiguration.distribution !== 'local',
  configuration: updateConfiguration,
  preferences: app.isPackaged && updateConfiguration?.distribution !== 'local' ? channelPreferences(app.getPath('userData')) : null,
  downloadPreferences: app.isPackaged && updateConfiguration?.distribution !== 'local' ? updatePreferences(app.getPath('userData')) : null,
  beforeInstall: async () => {
    if (activeHostOperations.size) throw new Error('仍有文件保存、上传或其他操作正在进行，请完成后再重启更新。');
    loadDataEnvironment({ userData: app.getPath('userData'), defaultEnvironment: packagedDataEnvironment });
    return holdServiceForUpdate(service);
  },
  broadcast: (state) => {
    if (!main || main.isDestroyed() || main.webContents.isDestroyed()) return;
    main.webContents.send(UPDATE_EVENT, state);
  },
});
workbenchUpdater.registerIpc(
  ipcMain,
  (sender) => Boolean(main && !main.isDestroyed() && sender === main.webContents),
);

function createWindow() {
  const stateFile = windowStateFile(app.getPath("userData"));
  const state = HEADLESS
    ? DEFAULT_WINDOW_STATE
    : loadWindowState(stateFile, screen.getPrimaryDisplay().workAreaSize);
  const win = new BrowserWindow({
    width: state.width,
    height: state.height,
    minWidth: 900,
    minHeight: 600,
    show: !HEADLESS,
    // 与现有壳一致：覆盖式标题栏并隐藏标题，macOS 保留系统窗口按钮
    titleBarStyle: process.platform === "darwin" ? "hiddenInset" : "hidden",
    title: app.getName(),
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
      // AI 浏览器标签使用独立内容进程。
      webviewTag: true,
      backgroundThrottling: false,
    },
  });
  if (!HEADLESS) {
    win.on("close", () => {
      try {
        saveWindowState(stateFile, currentWindowState(win));
      } catch (error) {
        console.warn(`[window] 本次窗口大小保存失败：${error.message}`);
      }
    });
    if (state.maximized) win.maximize();
  }
  backgroundLifecycle.bindWindow(win);
  guardWebviews(win);
  win.webContents.on("did-start-navigation", (_event, _url, _inPlace, isMainFrame) => {
    personalDocuments.trackNavigation(win.webContents, _event);
    if (_inPlace) return;
    if (isMainFrame) { notificationNavigation.invalidate(); personalPageAi.invalidate(); personalDocuments.clear(); }
    else personalPageAi.navigation(win.webContents, _event);
  });
  win.webContents.on("render-process-gone", () => { notificationNavigation.invalidate(); personalPageAi.invalidate(); personalDocuments.clear(); });
  win.on("closed", () => { notificationNavigation.invalidate(); personalPageAi.invalidate(); personalDocuments.clear(); });
  win.loadURL("app://localhost/");
  return win;
}

function guardWebviews(win) {
  require("./browser-webviews").guardBrowserWebviews(win,browser);
}

// 单实例：第二次启动唤起首个实例，行为与现有壳一致。
// 自检不参与单实例：它是独立的验证进程，不应因已有实例在跑而被挡下，也不应唤起对方。
if (!HEADLESS && !app.requestSingleInstanceLock({ executablePath: process.execPath })) {
  app.quit();
} else if (!HEADLESS) {
  app.on("second-instance", (_event, _argv, _workingDirectory, additionalData) => {
    if (!main || main.isDestroyed()) return;
    backgroundLifecycle.showWindow();
    const requestedPath = differentInstancePath(process.execPath, additionalData);
    if (requestedPath) {
      void dialog.showMessageBox(main, {
        type: "warning",
        title: "词来工作台测试包不一致",
        message: "当前运行的是另一份工作台测试包。请先退出当前应用，再打开要测试的版本。",
        detail: `当前：${process.execPath}\n准备打开：${requestedPath}`,
        buttons: ["知道了"],
      });
      return;
    }
    void workbenchUpdater.checkDue("second-instance");
  });
}

ipcMain.handle("host:invoke", async (event, message) => {
  const readOnly = isUpdateRead(message?.command, message?.args?.method);
  if (service.updateHold && !readOnly) throw new Error('正在准备重启更新，请稍后再操作。');
  const operation = Symbol();
  if (!readOnly) activeHostOperations.add(operation);
  try {
    if (message?.command === "personal_applications_close_ack"
        && (!main || main.isDestroyed() || event.sender !== main.webContents || event.senderFrame !== main.webContents.mainFrame)) {
      throw new Error("personal_application_close_ack_invalid");
    }
    if (notificationNavigation.handles(message?.command)) return notificationNavigation.ready(event, message.args);
    if (["personal_notification_preferences_get", "personal_notification_preferences_save", "personal_data_restore_workspace_plan"].includes(message?.command)) personalDocuments.verify(event);
    if (personalPageAi.handles(message?.command)) return await personalPageAi.invoke(event, message);
    if (personalDocuments.handles(message?.command)) {
      personalDocuments.verify(event);
      if (message.command === "personal_document_release") await personalPageAi.release(message.args?.url);
      const result = await personalDocuments.invoke(event, message);
      if (message.command === "personal_document_release") personalPageAi.forget(message.args?.url);
      return result;
    }
    if (macosDiskAccess.handles(message?.command)) return await macosDiskAccess.invoke(event, message);
    return await dispatch(message, event.sender);
  } catch (error) {
    // 保持与现有宿主一致的失败语义：命令失败以异常回到前端，不返回空值
    throw error;
  } finally {
    activeHostOperations.delete(operation);
  }
});

app.whenReady().then(async () => {
  app.setAppUserModelId("cn.cilai.personal.workbench");
  personalNotifications.start();
  powerMonitor.on('resume', () => { void workbenchUpdater.checkDue('resume'); });
  app.on('before-quit', () => workbenchUpdater.stop());
  let dataEnvironmentEnv;
  try {
    dataEnvironmentEnv = loadDataEnvironment({ userData: app.getPath("userData"), defaultEnvironment: packagedDataEnvironment });
  } catch (error) {
    console.error("[service] 数据环境无法确定：", error);
    if (!HEADLESS) dialog.showErrorBox("无法打开原有数据", error.message);
    app.exit(1);
    return;
  }
  // 开发模式下程序坞显示的是运行时自带的图标；打包后由应用包自带，无需设置。
  // 这里用位图而非图标集：图标集只有打包工具读得了，运行时的图像接口加载不了它。
  // 图标缺失不该影响启动，因此失败只记一条。
  if (!app.isPackaged && process.platform === "darwin") {
    const icon = path.join(__dirname, "..", "..", "..", "assets", "icons", "128x128@2x.png");
    try {
      app.dock?.setIcon(icon);
    } catch (error) {
      console.warn("[shell] 程序坞图标未设置：", error.message);
    }
  }

  // AI 浏览器的工具以本地 MCP 注册给对话运行时：先起服务拿到地址与令牌，再交给业务进程，
  // 由它在启动 Harness 时写进配置。服务起不来不阻断整体启动——不依赖浏览器的能力照常可用。
  let browserMcpEnv = {};
  try {
    const { endpoint, token } = await browserMcp.start();
    browserMcpEnv = { CILAI_BROWSER_MCP_URL: endpoint, CILAI_BROWSER_MCP_TOKEN: token };
  } catch (error) {
    console.warn("[browser-mcp] 未能启动，AI 将调用不到浏览器：", error.message);
  }
  try {
    service.testDiagnostics = createTestDiagnostics({
      directory: path.join(app.getPath("userData"), "test-diagnostics"),
      metadata: () => ({ version: app.getVersion(), platform: process.platform, arch: process.arch,
        osRelease: require("node:os").release(), electron: process.versions.electron }),
    });
  } catch (error) {
    console.error("[diagnostics] 测试日志无法创建：", error.message);
  }
  try {
    service.diagnosticsLog = createServiceLog(path.join(app.getPath("logs"), "host-service.log"));
  } catch (error) {
    console.error("[service] 诊断日志无法创建，耗时与错误只输出到控制台：", error);
  }
  service.start({ ...browserMcpEnv, ...dataEnvironmentEnv }, personalHostArguments);

  protocol.handle("app", (request) => {
    const { pathname, host } = new URL(request.url);
    if (host === "personal-capability") return personalDocuments.response(request);
    if (host !== "localhost") return new Response("forbidden", { status: 403 });
    const relative = pathname === "/" || pathname === "" ? "index.html" : decodeURIComponent(pathname).replace(/^\/+/, "");
    const target = path.join(DIST, relative);
    // 越界防护：自定义协议只服务构建产物目录内的文件
    if (!target.startsWith(DIST)) return new Response("forbidden", { status: 403 });
    return net.fetch(pathToFileURL(target).toString()).then((response) => {
      // 内容安全策略与现有壳保持一致，逐项对齐而非放宽
      const headers = new Headers(response.headers);
      headers.set("Content-Security-Policy", CSP);
      return new Response(response.body, { status: response.status, headers });
    });
  });

  main = createWindow();
  bindWindowEvents(main);
  try {
    backgroundLifecycle.createTray();
  } catch (error) {
    dialog.showErrorBox("后台运行无法启动", error.message);
    app.quit();
    return;
  }
  main.webContents.once("did-finish-load", () => {
    void workbenchUpdater.start().then(async () => {
      const failure = await portableUpdater?.previousFailure();
      if (failure) workbenchUpdater.fail(new Error(failure));
    }).catch((error) => workbenchUpdater.fail(error));
    if (portableUpdater && process.argv.some((arg) => arg.startsWith("--cilai-update="))) {
      void (async () => {
        const deadline = Date.now() + 60000;
        let status;
        do {
          status = await service.call("app_server_status", {});
          const rendered = await main.webContents.executeJavaScript('document.readyState === "complete" && document.getElementById("root")?.childElementCount > 0');
          if (runtimeStartupReady(status) && rendered) break;
          if (status.runtimeInitializationError) throw new Error(status.runtimeInitializationError);
          if (status.phase === "failed") throw new Error("新版业务服务启动失败");
          await new Promise((resolve) => setTimeout(resolve, 200));
        } while (Date.now() < deadline);
        if (!runtimeStartupReady(status) || Date.now() >= deadline) throw new Error("新版界面或业务服务未就绪");
        await portableUpdater.acknowledgeStartup();
      })().catch((error) => console.error("[updater] 新版启动核验失败：", error));
    }
  });

  if (PERSONAL_SMOKE) {
    try {
      await new Promise(resolve => main.webContents.once("did-finish-load", resolve));
      await require('./personal-smoke-check').checkPersonalStartup({ window: main, service, root: headlessIsolation.root });
      app.quit();
    } catch (error) {
      console.error('[personal-check] failed:', error.message);
      service.stop();
      app.exit(1);
    }
    return;
  }

  if (HEADLESS) {
    // 启动自检：确认前端能在新壳中完成加载并渲染出根节点
    const failures = [];
    let personalDocumentProbe = false;
    const checkFailures = [];
    const check = (name, passed) => {
      if (!passed) checkFailures.push(name);
    };
    main.webContents.on("console-message", (event) => {
      if (personalDocumentProbe && event.sourceId?.startsWith(`${PERSONAL_DOCUMENT_ORIGIN}/`)
        && event.message.includes("https://personal-document-probe.invalid/")
        && event.message.includes("Content Security Policy")) return;
      if (event.level === "error" || event.level >= 2) {
        failures.push(`${event.message} (${event.sourceId}:${event.lineNumber})`);
      }
    });
    await new Promise((resolve) => main.webContents.once("did-finish-load", resolve));
    await new Promise((resolve) => setTimeout(resolve, 2500));

    const probe = await main.webContents.executeJavaScript(`(() => ({
      title: document.title,
      hasRoot: Boolean(document.getElementById("root")),
      rootChildren: document.getElementById("root")?.childElementCount ?? 0,
      bodyText: document.body.innerText.slice(0, 120),
      hostChannel: typeof window.__CILAI_HOST__?.invoke,
      windowControls: [...document.querySelectorAll(".window-control")].map((button) => button.getAttribute("aria-label")),
    }))()`);

    console.log("标题:", probe.title);
    console.log("根节点存在:", probe.hasRoot);
    console.log("根节点子元素数:", probe.rootChildren);
    console.log("宿主通道可用:", probe.hostChannel === "function");
    console.log("界面文本:", JSON.stringify(probe.bodyText));
    personalDocumentProbe = true;
    try {
      const result = await require("./personal-document-check").checkPersonalDocument(main.webContents);
      console.log("应用隔离页面:", JSON.stringify(result));
      check("应用隔离页面", result.inlineRan && result.added && result.reset && result.networkBlocked && result.parentBlocked && result.hostAbsent && result.opaqueOrigin);
    } catch (error) {
      console.log("应用隔离页面:", error.message);
      check("应用隔离页面", false);
    } finally { personalDocumentProbe = false; }
    if (process.platform === "win32") {
      const expectedControls = ["最小化", "最大化", "关闭"];
      let windowControls = probe.windowControls;
      // 按钮要等异步 host_platform 返回后才渲染，启动探针必须等待这次状态更新。
      for (let attempt = 0; attempt < 40 && !expectedControls.every((label) => windowControls.includes(label)); attempt++) {
        await new Promise((resolve) => setTimeout(resolve, 250));
        windowControls = await main.webContents.executeJavaScript(`([...document.querySelectorAll(".window-control")].map((button) => button.getAttribute("aria-label")))`);
      }
      check("Windows 窗口控制", expectedControls.every((label) => windowControls.includes(label)));
      console.log("Windows 窗口控制:", windowControls.join("、"));
    }

    // 读取个人资料与本地服务状态。
    const summarize = (value) => {
      if (value === null || typeof value !== "object") return String(value);
      if (typeof value.phase === "string") {
        return `phase=${value.phase} mode=${value.connectionMode} pid=${value.pid}`;
      }
      return JSON.stringify(value).slice(0, 120);
    };
    const probes = ["host_platform", "app_server_status", "local_profile_status"];
    for (const command of probes) {
      try {
        const value = await service.call(command, {});
        if (command === "app_server_status") check("业务服务及历史会话初始化", runtimeStartupReady(value));
        if (command === "local_profile_status") check("本地资料初始化", localProfileReady(value));
        console.log(`业务服务 ${command}:`, summarize(value));
      } catch (error) {
        if (command === "app_server_status") check("业务服务及历史会话初始化", false);
        if (command === "local_profile_status") check("本地资料初始化", false);
        console.log(`业务服务 ${command}: 失败 ${error.message}`);
      }
    }
    // 应用服务的调用准入：方法名来自界面，表外方法必须被拒，否则这条通用通道
    // 就成了绕过命令边界的入口。表内方法同时验一次，确认这道门没有误伤在用功能。
    const rpcGate = async (method) => {
      try {
        await service.call("app_server_request", { method, params: {} });
        return "放行";
      } catch (error) {
        return error.message.includes("desktop_method_not_allowed") ? "已拒绝" : `失败(${error.message.slice(0, 40)})`;
      }
    };
    const outsideRpc = await rpcGate("system/executeArbitraryCommand");
    const insideRpc = await rpcGate('workspace/list');
    console.log(`应用服务准入: 表外方法：${outsideRpc}，表内方法：${insideRpc}`);
    check("应用服务调用准入", outsideRpc === "已拒绝" && insideRpc === "放行");

    if (failures.length > 0) {
      console.log(`\n控制台报错 ${failures.length} 条，前 5 条：`);
      failures.slice(0, 5).forEach((text) => console.log("  " + text.slice(0, 160)));
    } else {
      console.log("\n控制台无报错");
    }


    // AI 浏览器运行时：站点策略、页面快照、提取与截图
    let browserResult = "未执行";
    try {
      const { browser: runtime } = require("./bridge");
      const pieces = [];
      const allowedHttps = runtime.checkOrigin("https://example.com");
      pieces.push(`安全传输：${allowedHttps ? "未放行" : "已放行"}`);
      const insecure = runtime.checkOrigin("http://internal.example.com");
      pieces.push(`非安全传输：${insecure ? "已拒绝" : "未拒绝"}`);
      const localNonWeb = runtime.checkOrigin("ftp://localhost/file");
      check("浏览器传输策略", !allowedHttps && Boolean(insecure) && Boolean(localNonWeb));

      await require('./headless-browser-fixture').installBrowserFixture(main.webContents);
      await runtime.ensure();
      // 声明字符集，否则中文会被按单字节解码
      await runtime.current().contents.loadURL("data:text/html;charset=utf-8," + encodeURIComponent(
        "<title>探针页</title><body><h1>标题一</h1><button>提交</button><p>正文内容</p></body>"));
      await new Promise((resolve) => setTimeout(resolve, 600));
      const snapshot = await runtime.snapshot();
      const extracted = await runtime.extract();
      /*
       * 截图依赖页面正在呈现。浏览器归属某一次对话，自检里开的标签不属于界面此刻停留的
       * 对话，因此取不到帧——这不是故障，要的是它带着原因失败而不是把自检挂死。
       */
      const shot = await runtime.screenshot().catch((error) => ({ reason: error.message }));
      pieces.push(`快照节点 ${snapshot.nodes.length}`);
      pieces.push(`提取文本 ${JSON.stringify(extracted.text.replace(/\s+/g, "").slice(0, 10))}`);
      pieces.push(shot.reason ? `截图未取到：${shot.reason.slice(0, 24)}` : `截图 ${Math.round(shot.data.length / 1024)} KB`);
      check("浏览器页面结构", snapshot.nodes.length > 0);
      check("浏览器文本提取", extracted.text.includes("正文内容"));
      check("隐藏页面截图状态", !shot.reason || shot.reason.includes("页面当前未呈现在界面上"));
      // 页面自身跳转到非安全传输地址，应被拦下且留在原地
      const before = runtime.current().contents.getURL();
      await runtime.current().contents.executeJavaScript(
        'location.href = "http://example.com/"').catch(() => undefined);
      await new Promise((resolve) => setTimeout(resolve, 400));
      const navigationBlocked = runtime.current().contents.getURL() === before;
      pieces.push(`非安全跳转：${
        navigationBlocked ? "已拦下" : "未拦下"}`);
      check("浏览器非安全跳转", navigationBlocked);
      // 写操作：审批链路未接入，无论是否带令牌一律拒绝
      const write = await dispatch({ command: "browser_click", args: { x: 1, y: 1 } }, main.webContents)
        .then(() => "未拒绝").catch(() => "已拒绝");
      pieces.push(`AI 写操作：${write}`);
      check("浏览器写入确认", write === "已拒绝");

      // 人工接管：非接管态不受理界面输入，接管期间 AI 暂停
      // 驾驶权：员工接手期间 AI 的写操作被拒，交还后恢复。输入不再经宿主转送，无从拦截
      await dispatch({ command: "browser_takeover_begin", args: {} }, main.webContents);
      const duringEmployee = await runtime.click({ x: 1, y: 1 }).then(() => "未拒绝").catch(() => "已拒绝");
      await dispatch({ command: "browser_takeover_end", args: {} }, main.webContents);
      const afterReturn = await runtime.click({ x: 1, y: 1 }).then(() => "已受理").catch((error) => "失败：" + error.message);
      pieces.push(`员工接手期间 AI 写操作：${duringEmployee}，交还后：${afterReturn}`);
      check("浏览器接管与交还", duringEmployee === "已拒绝" && afterReturn === "已受理");

      /*
       * 界面上的运行面板。接管是必需能力：登录、验证码、支付确认必须由人完成，没有这个
       * 入口那些流程走不通。面板由宿主的运行状态通知驱动，因此这里连同通知一起验。
       */
      const panel = await main.webContents.executeJavaScript(`(() => {
        const el = document.querySelector(".ai-browser");
        if (!el) return "未呈现";
        return (el.querySelector(".ai-browser__driver button")?.textContent ?? "无接管入口");
      })()`);
      pieces.push(`界面面板：${panel === "未呈现" ? "未呈现" : "已呈现，接管入口「" + panel + "」"}`);

      browserResult = pieces.join("，");
      runtime.close();
      await new Promise((resolve) => setTimeout(resolve, 400));
      const closed = await main.webContents.executeJavaScript(
        `document.querySelector(".ai-browser")?.getAttribute("data-open") === "true"`);
      console.log(`AI 浏览器面板：${closed ? "仍展开" : "已收起（元素保留，标签不被杀掉）"}`);
      check("浏览器关闭状态", !closed);
    } catch (error) {
      browserResult = "失败：" + error.message;
      checkFailures.push("浏览器运行时");
    }
    console.log("AI 浏览器:", browserResult);
    await require('./headless-browser-fixture').removeBrowserFixture(main.webContents);

    const personalApplications = await service.call("app_server_request", { method: "capabilityCreation/list", params: {} });
    check("个人应用目录", Array.isArray(personalApplications.projects));
    for (const command of ["application_catalog_list", "application_launch_web", "application_install_local", "ai_browser_policy_get"]) {
      const removed = await service.call(command, {}).then(() => false).catch((error) => error.message.includes("personal_feature_removed"));
      check(`停用入口 ${command}`, removed);
    }

    // 事件通道：注册后收得到，注销后收不到——注销失效会让事件一直发往已卸载的组件
    let eventResult = "未执行";
    try {
      await main.webContents.executeJavaScript(`(async () => {
        window.__probe = [];
        const api = window.__CILAI_HOST__;
        window.__probeId = await api.invoke("plugin:event|listen", {
          event: "probe-event", target: { kind: "Any" },
          handler: api.transformCallback((payload) => window.__probe.push(payload)),
        });
      })()`);
      emitToRenderer("probe-event", { round: 1 });
      await new Promise((resolve) => setTimeout(resolve, 200));
      const received = await main.webContents.executeJavaScript("window.__probe.length");
      await main.webContents.executeJavaScript(`window.__CILAI_HOST__.invoke(
        "plugin:event|unlisten", { event: "probe-event", eventId: window.__probeId })`);
      emitToRenderer("probe-event", { round: 2 });
      await new Promise((resolve) => setTimeout(resolve, 200));
      const after = await main.webContents.executeJavaScript("window.__probe.length");
      eventResult = `注册后收到 ${received} 条，注销后新增 ${after - received} 条`;
      check("事件订阅与注销", received === 1 && after === received);
    } catch (error) {
      eventResult = "失败：" + error.message;
      checkFailures.push("事件订阅与注销");
    }
    console.log("事件通道:", eventResult);

    // 浏览器工具的 MCP 服务：握手、工具清单、调用，以及无令牌必须被拒
    let mcpResult = "未执行";
    try {
      const { endpoint, token } = await browserMcp.start();
      const post = (body, useToken = true) => fetch(endpoint, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          ...(useToken ? { authorization: `Bearer ${token}` } : {}),
        },
        body: JSON.stringify(body),
      });
      const hello = await post({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} })
        .then((response) => response.json());
      const list = await post({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} })
        .then((response) => response.json());
      // 非安全传输：工具应把拒绝原因回给模型，而不是当作协议错误中断整轮
      const call = await post({
        jsonrpc: "2.0", id: 3, method: "tools/call",
        params: { name: "browser_navigate", arguments: { url: "http://example.com/" } },
      }).then((response) => response.json());
      const anonymous = await post({ jsonrpc: "2.0", id: 4, method: "tools/list" }, false)
        .then((response) => response.status);
      mcpResult = `握手 ${hello.result?.serverInfo?.name}`
        + `，工具 ${list.result?.tools?.length} 个`
        + `，非安全传输：${call.result?.isError ? "已拒绝" : "未拒绝"}`
        + `，无令牌：${anonymous === 401 ? "已拒绝" : `未拒绝(${anonymous})`}`;
      check("浏览器工具准入", hello.result?.serverInfo?.name === "cilai-browser"
        && list.result?.tools?.length > 0 && call.result?.isError && anonymous === 401);

      // 写工具：凭据字段直接拒绝；普通写操作须等员工确认，未确认不得执行
      const { browserApprovals } = require("./bridge");
      const credential = await post({
        jsonrpc: "2.0", id: 5, method: "tools/call",
        params: {
          name: "browser_type",
          arguments: { text: "1234", fieldHint: "登录密码", purpose: "登录" },
        },
      }).then((response) => response.json());
      let seen = null;
      const stopWatch = (() => {
        const previousNotify = browserApprovals.notify;
        const handler = (payload) => { seen = payload; };
        browserApprovals.notify = (event, payload) => {
          if (event === "browser-approval-request") handler(payload);
        };
        return () => { browserApprovals.notify = previousNotify; };
      })();
      const pending = post({
        jsonrpc: "2.0", id: 6, method: "tools/call",
        params: { name: "browser_click", arguments: { x: 5, y: 5, purpose: "自检" } },
      }).then((response) => response.json());
      await new Promise((resolve) => setTimeout(resolve, 300));
      const raised = seen !== null;
      // 拒绝这次操作，确认工具把拒绝原因回给模型而不是照常执行
      if (raised) browserApprovals.resolve(seen.confirmationId, false);
      const rejected = await pending;
      stopWatch();
      mcpResult += `，凭据字段：${credential.result?.isError ? "已拒绝" : "未拒绝"}`
        + `，写操作请示：${raised ? "已发起" : "未发起"}`
        + `，员工拒绝后：${rejected.result?.isError ? "未执行" : "仍执行"}`;
      check("浏览器工具写入确认", credential.result?.isError && raised && rejected.result?.isError);
    } catch (error) {
      mcpResult = "失败：" + error.message;
      checkFailures.push("浏览器工具");
    }
    console.log("浏览器工具:", mcpResult);

    // 二进制通道：造一份真 PDF，走校验、页数、整文件读取与单页渲染的完整链路
    let pdfResult = "未执行";
    try {
      const { mkdtempSync, writeFileSync } = require("node:fs");
      const { tmpdir } = require("node:os");
      const root = mkdtempSync(path.join(tmpdir(), "cilai-pdf-"));
      const maker = new BrowserWindow({ show: false, webPreferences: { offscreen: true } });
      await maker.loadURL("data:text/html;charset=utf-8," + encodeURIComponent("<h1>预览探针</h1>"));
      writeFileSync(path.join(root, "探针.pdf"), await maker.webContents.printToPDF({}));
      maker.destroy();

      const call = (command, extra) => service.call(command, { localRoot: root, relativePath: "探针.pdf", ...extra });
      const mode = pdfSelfCheckMode(process.platform);
      const pieces = [];
      const decode = (value) => Buffer.from(value.__binary, "base64");
      const bytes = decode(await call("workspace_pdf_read_bytes"));
      pieces.push(`整文件 ${Math.round(bytes.length / 1024)} KB，文件头 ${bytes.subarray(0, 5).toString()}`);
      let rendererReady = false;
      if (mode === "native-renderer") {
        pieces.unshift(`页数 ${await call("workspace_pdf_page_count")}`);
        const page = decode(await call("workspace_render_pdf_page", { pageNumber: 1, maxWidth: 640 }));
        pieces.push(`单页位图 ${Math.round(page.length / 1024)} KB`);
        rendererReady = page.length > 0;
      } else {
        const assets = runtimePdfJsAssets(DIST);
        pieces.push(
          `PDF.js 随包资产：${assets.workerCount} 个 Worker、${assets.cMapCount} 个 CMap、`
          + `${assets.fontCount} 个字体、${assets.decoderCount} 个解码模块`,
        );
        const page = await require("./windows-headless-check").renderPdfProbe(main.webContents, DIST, bytes);
        pieces.push(`PDF.js 页数 ${page.pageCount}，位图 ${page.width}×${page.height}，非白像素 ${page.inkPixels}`);
        rendererReady = page.pageCount === 1 && page.width === 640 && page.height > 0 && page.inkPixels > 0;
      }
      // 非 PDF 内容必须被签名校验挡住，扩展名相同也不放行
      writeFileSync(path.join(root, "伪装.pdf"), "这不是 PDF");
      const forged = await service.call("workspace_pdf_read_bytes", { localRoot: root, relativePath: "伪装.pdf" })
        .then(() => "未拒绝").catch((error) => error.message.includes("signature_invalid") ? "已拒绝" : "拒绝原因不符");
      pieces.push(`伪装文件：${forged}`);
      pdfResult = pieces.join("，");
      check("二进制通道", bytes.subarray(0, 5).toString() === "%PDF-"
        && rendererReady && forged === "已拒绝");
    } catch (error) {
      pdfResult = "失败：" + error.message;
      checkFailures.push("二进制通道");
    }
    console.log("二进制通道:", pdfResult);

    // 界面截图：自检跑在无窗口模式下，画面仍可由渲染进程直接取出，
    // 用于人工复核"界面到底长什么样"，而不是只看几个布尔值。
    // 界面截图：自检没有窗口，画面仍可从渲染进程直接取出。布尔值说明不了界面长什么样，
    // 留一条取图的路径，便于人工复核。
    if (process.argv.includes("--capture")) {
      const shoot = async (name) => {
        const image = await main.webContents.capturePage();
        require("node:fs").writeFileSync(`/tmp/next-${name}.png`, image.toPNG());
        console.log(`界面截图 ${name}: /tmp/next-${name}.png`);
      };
      // 驱动界面打开第一个可用应用：布尔值证明不了业务应用真的跑起来了。这段按界面文案
      // 定位控件，界面改文案时需要跟着调整——这是它的代价，换来的是能看到真实画面。
      const opened = await main.webContents.executeJavaScript(`(async () => {
        const nav = [...document.querySelectorAll("button")].find((el) =>
          ((el.getAttribute("aria-label") || "") + (el.textContent || "")).includes("应用"));
        if (!nav) return "找不到应用入口";
        nav.click();
        for (let i = 0; i < 20; i += 1) {
          await new Promise((r) => setTimeout(r, 500));
          const open = [...document.querySelectorAll("button")].find((el) =>
            (el.getAttribute("aria-label") || "").startsWith("打开"));
          if (open) { const label = open.getAttribute("aria-label"); open.click(); return label; }
        }
        return "目录中没有可打开的应用";
      })()`);
      console.log("打开动作:", opened);
      await new Promise((resolve) => setTimeout(resolve, 6000));
      await shoot("bridged");
    }

    const ok = probe.hasRoot && probe.rootChildren > 0 && probe.hostChannel === "function"
      && failures.length === 0 && checkFailures.length === 0;
    if (checkFailures.length > 0) console.log(`未通过项目：${checkFailures.join("、")}`);
    if (failures.length > 0) console.log(`控制台报错：${failures.slice(0, 5).join("；")}`);
    console.log(HEADLESS_SCOPE_MARKER);
    console.log(AUTHENTICATED_CHECK_MARKER);
    console.log(`\n启动自检：${ok ? "通过" : "未通过"}`);
    backgroundLifecycle.exit(ok ? 0 : 1);
  }
});

// 与现有壳对齐的系统能力：剪贴板写入与文件对话框
ipcMain.handle("host:clipboard-write", (_event, text) => clipboard.writeText(String(text)));
ipcMain.handle("host:open-dialog", (_event, options) => dialog.showOpenDialog(main, options ?? {}));
