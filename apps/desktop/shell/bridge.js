const { app, BrowserWindow, clipboard, dialog, nativeImage, screen } = require("electron");
const { HostService, resolveBinary } = require("./service");
const { BrowserRuntime } = require("./browser");
const { BrowserMcpServer } = require("./browser-mcp");
const { BrowserApprovals } = require("./browser-approval");
const { widenWindowBounds } = require("./window-state");
const { createDiagnosticExport } = require("./diagnostic-export");
const { preparePersonalPackageImport } = require("./personal-package-import");
const personalData = require("./personal-data");
const { createPersonalNamespaceStorage } = require("./personal-namespace-storage");
const { isDelegatedCommand } = require("./host-commands.generated");

/**
 * 命令路由。
 *
 * 界面直接使用的宿主命令必须在本地实现或明确转交宿主服务。命令契约测试会逐项核对界面调用、
 * Electron 路由和 Rust 宿主服务；表外命令仍显式拒绝，不以空值掩盖缺口。
 */

/** 事件通道：命名沿用现有实现，迁移期两边语义保持一致。 */
const listeners = new Map();

/**
 * 监听表的键。
 *
 * 回调号由各个渲染进程自行计数，都从一开始，只用回调号作键会让不同窗口的监听互相覆盖。
 * 加上窗口标识才能把它们分开。
 */
function listenerKey(sender, handler) {
  return `${sender.id}:${handler}`;
}

function emitToRenderer(event, payload) {
  let delivered = 0;
  for (const [key, entry] of listeners) {
    if (entry.event !== event) continue;
    const window = BrowserWindow.fromWebContents(entry.sender);
    if (!window || window.isDestroyed()) {
      listeners.delete(key);
      continue;
    }
    const id = entry.handler;
    entry.sender.send("host:callback", { id, payload: { event, id, payload } });
    delivered++;
  }
  return delivered;
}

/**
 * AI 浏览器运行时。
 *
 * 站点范围显式声明，未列出的地址拒绝打开并说明原因。写操作须经审批后放行——审批链路
 * 接入 Harness 之前一律拒绝，不以"暂未接入"为由放行。
 */
const browser = new BrowserRuntime({ allowedHosts: [] });
/** 写操作的审批：请求经事件送到界面，员工的决定经命令回来。 */
const browserApprovals = new BrowserApprovals((event, payload) => emitToRenderer(event, payload));
/** 工具以本地 MCP 形式注册，AI 调用浏览器与调用业务应用因此走同一套机制。 */
const browserMcp = new BrowserMcpServer(browser, browserApprovals);
/*
 * 运行状态透给界面。
 *
 * AI 何时开始浏览由模型决定，界面无从预知；没有这条通知，界面只能靠帧的有无去猜，而
 * 结束是猜不出来的——画面会一直挂在那里。接管态一并带上，界面不自行记状态。
 */
browser.onState = (state) => emitToRenderer("browser-state", state);
/*
 * 请界面开一个标签。
 *
 * 内容进程只能由界面创建——webview 元素附着时才存在，宿主自己建不出来。AI 调用工具时
 * 界面未必已经开过标签，这条通知补上那一步。
 */
browser.onOpenTab = (tabId, conversationId) =>
  emitToRenderer("browser-open-tab", { tabId, conversationId: conversationId ?? null });

/** 写操作：具有外部副作用，逐次确认，未获批准不执行。 */
const BROWSER_WRITE_TOOLS = new Set(["browser_click", "browser_type", "browser_set_value"]);

/** 业务服务：新壳不再把业务逻辑编进自身，一律转交进程外的服务。 */
const service = new HostService(resolveBinary());

/** 业务进程只向主界面发送已登记事件。 */
service.onHostRequest = async (command, args) => {
  if (command === "personal_page_ai_ui_action") {
    if (!personalPageAi) throw new Error("personal_page_bridge_unavailable");
    return personalPageAi.action(args);
  }
  if (command === "personal_namespace_storage_execute") return namespaceStorage.perform(args);
  /*
   * 业务侧向界面播报进度。登录这类一次调用到底的命令没有中途上报的余地，
   * 界面只能干等——按钮转上几秒，员工看不出是在工作还是卡死了。
   */
  if (command === "notify_renderer") {
    emitToRenderer(String(args?.event ?? ""), args?.payload ?? {});
    return null;
  }
  throw new Error(`未知的宿主能力：${command}`);
};

let personalApplicationCloser = null;
let personalPageAi = null;
let personalNotifications = null;
function setPersonalNotifications(value) {
  if (personalNotifications || typeof value?.preferences !== "function" || typeof value?.setProfile !== "function") throw new Error("personal_notification_configuration_invalid");
  personalNotifications = value;
}
async function withPersonalNotificationsPaused(action) {
  if (!personalNotifications) throw new Error("personal_notification_configuration_invalid");
  await personalNotifications.pause();
  try { return await action(); } finally { personalNotifications.resume(); }
}
function setPersonalPageAi(value) {
  if (personalPageAi || typeof value?.action !== "function") throw new Error("personal_page_bridge_invalid");
  personalPageAi = value;
}
function setPersonalApplicationCloser(closer) {
  if (personalApplicationCloser || typeof closer?.close !== "function" || typeof closer?.acknowledge !== "function") throw new Error("personal_application_closer_invalid");
  personalApplicationCloser = closer;
}
async function closeApplicationContainers() {
  if (!personalApplicationCloser) throw new Error("personal_application_closer_unavailable");
  await personalApplicationCloser.close();
}

const diagnosticExport = createDiagnosticExport({ diagnostics: () => service.testDiagnostics,
  collect: () => service.call("test_runtime_diagnostics", {}), dialog });
let verifiedPersonalProfileId = null;
const namespaceStorage = createPersonalNamespaceStorage({ app, browser,
  getVerifiedProfileId: () => verifiedPersonalProfileId, service, closeApplications: closeApplicationContainers });

const implemented = {
  personal_notification_preferences_get: ({ args }) => {
    if (!personalNotifications) throw new Error("personal_notification_unavailable");
    return personalNotifications.preferences("personal_notification_preferences_get", args);
  },
  personal_notification_preferences_save: ({ args }) => {
    if (!personalNotifications) throw new Error("personal_notification_unavailable");
    return personalNotifications.preferences("personal_notification_preferences_save", args);
  },
  personal_applications_close_ack: ({ args, sender }) => {
    if (!personalApplicationCloser) throw new Error("personal_application_closer_unavailable");
    return personalApplicationCloser.acknowledge(args, sender);
  },
  personal_capability_upgrade: ({ args }) => withPersonalNotificationsPaused(() => personalData.runPersonalApplicationChange({ command: "personal_capability_upgrade", args, service, closeApplications: closeApplicationContainers })),
  personal_package_import_commit: ({ args }) => withPersonalNotificationsPaused(() => personalData.runPersonalApplicationChange({ command: "personal_package_import_commit", args, service, closeApplications: closeApplicationContainers })),
  personal_application_data_rollback: ({ args }) => withPersonalNotificationsPaused(() => personalData.runPersonalApplicationChange({ command: "personal_application_data_rollback", args, service, closeApplications: closeApplicationContainers })),
  personal_memory_reset_confirm: ({ args }) => withPersonalNotificationsPaused(() => personalData.runPersonalMaintenance({ command: "personal_memory_reset_confirm", args, service, closeApplications: closeApplicationContainers })),
  personal_namespace_storage_inventory: ({ args }) => {
    if (!args || typeof args !== "object" || Array.isArray(args) || Object.keys(args).length) throw new Error("personal_storage_request_invalid");
    return namespaceStorage.inventory();
  },
  personal_namespace_storage_prepare: ({ args }) => namespaceStorage.prepare(args),
  personal_namespace_storage_execute: ({ args }) => withPersonalNotificationsPaused(() => namespaceStorage.execute(args)),
  personal_namespace_storage_finish: ({ args }) => namespaceStorage.finish(args),
  personal_storage_execute: ({ args }) => withPersonalNotificationsPaused(() => personalData.runPersonalMaintenance({ command: "personal_storage_execute", args, service, closeApplications: closeApplicationContainers })),
  personal_data_restore_prepare: ({ args, window }) => personalData.preparePersonalRestore({ args, window, dialog, service }),
  personal_data_restore_workspace_plan: ({ args, window }) => personalData.planPersonalWorkspaceRestore({ args, window, dialog, service }),
  personal_data_backup_export: ({ args, window }) => personalData.exportPersonalBackup({ args, window, dialog, service }),
  personal_data_backup_capture: ({ args }) => withPersonalNotificationsPaused(() => personalData.runPersonalMaintenance({ command: "personal_data_backup_capture", args, service, closeApplications: closeApplicationContainers })),
  personal_data_restore_apply: ({ args }) => withPersonalNotificationsPaused(() => personalData.runPersonalMaintenance({ command: "personal_data_restore_apply", args, service, closeApplications: closeApplicationContainers })),
  personal_data_restart: () => personalData.restartAfterPersonalRestore({ service, app }),
  model_connection_auth_open: async ({ args }) => {
    const url = require("./model-auth-browser").officialAuthorizationUrl(args?.url);
    await require("electron").shell.openExternal(url);
    return null;
  },
  test_diagnostics_settings: () => {
    if (!service.testDiagnostics) throw new Error("测试日志无法初始化，请检查应用数据目录权限。");
    return service.testDiagnostics.status();
  },
  test_diagnostics_set_enabled: ({ args }) => {
    if (!service.testDiagnostics) throw new Error("测试日志无法初始化，请检查应用数据目录权限。");
    return service.testDiagnostics.setEnabled(args?.enabled);
  },
  test_diagnostics_preview: ({ sender }) => diagnosticExport.preview(sender.id),
  test_diagnostics_export: ({ sender, window, args }) => diagnosticExport.save(sender.id, window, args),
  // 事件通道
  "plugin:event|listen": ({ args, sender }) => {
    listeners.set(listenerKey(sender, args.handler), {
      event: args.event, handler: args.handler, sender,
    });
    return args.handler;
  },
  /**
   * 注销监听。
   *
   * 不实现这条，界面每次卸载组件都会收到一条命令未迁移的错误，而监听表只增不减：
   * 事件继续发往已经不存在的组件，表本身也一直涨。
   */
  "plugin:event|unlisten": ({ args, sender }) => {
    listeners.delete(listenerKey(sender, args.eventId));
    return null;
  },
  "plugin:event|emit": ({ args }) => {
    emitToRenderer(args.event, args.payload);
    return null;
  },

  local_profile_status: async () => applyLocalProfile(await service.call("local_profile_status", {})),

  // AI 浏览器操作：与应用容器共用引擎，各用独立会话分区
  browser_navigate: ({ args }) => browser.navigate(args.url),
  browser_snapshot: ({ args }) => browser.snapshot(args.maxNodes, args.tabId, args.conversationId ?? null),
  browser_screenshot: ({ args }) => browser.screenshot(args.tabId),
  browser_extract: ({ args }) => browser.extract(args.selector, args.tabId),
  browser_tabs: () => browser.listTabs(),
  // 员工自己的导航与页内动作。不经站点策略：策略约束的是 AI 能去哪里
  browser_employee_navigate: ({ args }) =>
    browser.navigateByEmployee(String(args.url ?? ""), args.conversationId ?? null),
  browser_tab_open: ({ args }) =>
    browser.openTab(args.url ?? null, { byEmployee: true, conversationId: args.conversationId ?? null }),
  browser_tab_close: ({ args }) => browser.closeTab(String(args.tabId ?? "")),
  browser_tab_activate: ({ args }) => browser.activateTab(String(args.tabId ?? "")),
  browser_back: ({ args }) => browser.goBack(args.conversationId ?? null),
  browser_forward: ({ args }) => browser.goForward(args.conversationId ?? null),
  browser_reload: ({ args }) => browser.reload(args.conversationId ?? null),
  browser_devtools: ({ args }) => browser.toggleDevTools(args.conversationId ?? null),
  browser_state: () => browser.state(),
  browser_capture_layout_ready: ({ args, sender }) => {
    if (!sender.getURL().startsWith("app://localhost/")) throw new Error("browser_layout_sender_forbidden");
    return browser.captureLayoutReady(args);
  },
  browser_scroll: ({ args }) => browser.scroll(args.deltaY ?? -240),
  browser_wait_for: ({ args }) => browser.waitFor(args.expression, args.timeoutMs),
  browser_console: ({ args }) => browser.consoleMessages(
    { level: args.level, limit: args.limit }, args.tabId, args.conversationId ?? null),
  browser_network: ({ args }) => browser.networkRequests(
    { failedOnly: args.failedOnly, limit: args.limit }, args.tabId, args.conversationId ?? null),
  // 人工接管：员工直接操作页面，AI 在此期间暂停
  /** 员工对一次写操作的决定。未知确认号不受理——它要么已超时，要么根本不存在。 */
  browser_approval_resolve: ({ args }) =>
    browserApprovals.resolve(
      String(args.confirmationId ?? ""), args.approved === true, args.grantSite === true),

  // 驾驶权。同一时刻只有一方操作页面，员工接手期间 AI 的写操作被拒
  /*
   * 员工接手时收回该对话已给出的站点授权：授权是"我看着 AI 在这个站点做事"的前提下给的，
   * 员工自己动过页面之后，页面已经不是 AI 当时看到的样子，原来的授权不该继续生效。
   */
  browser_takeover_begin: ({ args }) => {
    browserApprovals.revokeScope(args?.conversationId ?? null);
    return browser.setDriver("employee");
  },
  browser_takeover_end: () => browser.setDriver("ai"),

  // 窗口控制：界面自绘标题栏经此调用，macOS 的系统窗口按钮不走这里
  "plugin:window|minimize": ({ window }) => (window?.minimize(), null),
  "plugin:window|maximize": ({ window }) => (window?.maximize(), null),
  "plugin:window|unmaximize": ({ window }) => (window?.unmaximize(), null),
  "plugin:window|toggle_maximize": ({ window }) => {
    if (!window) return null;
    if (window.isMaximized()) window.unmaximize();
    else window.maximize();
    return null;
  },
  "plugin:window|is_maximized": ({ window }) => Boolean(window?.isMaximized()),
  "plugin:window|close": ({ window }) => (window?.close(), null),
  "plugin:window|set_focus": ({ window }) => (window?.focus(), null),
  "plugin:window|start_dragging": () => null,
  "plugin:window|scale_factor": () => 1,
  "plugin:window|inner_size": ({ window }) => {
    const [width, height] = window?.getContentSize() ?? [0, 0];
    return { width, height };
  },
  /*
   * 对话页展开浏览器时，窗口不够宽就把它拉宽。不拉的后果不是"浏览器窄一点"，而是对话区
   * 被压到下限以下、内容溢出后被切掉——那一截看上去像被浏览器盖住，员工查不出是谁干的。
   *
   * 只放大不缩小，且不碰最大化和全屏：那是员工自己摆好的状态，此时返回当前宽度，由界面
   * 那边的让宽逻辑收场。业务应用不得改工作台窗口，与其他会越界的命令同样拦在这里。
   */
  "plugin:window|ensure_content_width": ({ args, sender, window }) => {
    const contentWidth = window?.getContentSize()[0] ?? 0;
    if (!window || window.isMaximized() || window.isFullScreen()) return { width: contentWidth };
    const bounds = window.getBounds();
    const next = widenWindowBounds({
      bounds,
      contentWidth,
      wantedContentWidth: Number(args?.width),
      workArea: screen.getDisplayMatching(bounds).workArea,
    });
    if (next === null) return { width: contentWidth };
    window.setBounds({ x: next.x, y: next.y, width: next.width, height: next.height });
    return { width: next.contentWidth };
  },

  // 剪贴板：界面不使用浏览器接口，统一由宿主代写
  "plugin:clipboard-manager|write_text": ({ args }) => {
    // 内容在 text。协议里的 label 是窗口标签，此前也被当作内容取用，那会让一次传错
    // 参数的调用照常"成功"，把错误推迟到员工发现粘贴出来的东西不对。
    clipboard.writeText(String(args?.text ?? ""));
    return null;
  },
  "plugin:clipboard-manager|read_text": () => clipboard.readText(),

  /**
   * 目录选择。对话框由壳持有的窗口承载，选中结果只回传路径本身；
   * 取消与选中是两种正常结果，不视为失败。
   */
  workspace_pick_directory: async ({ window }) => {
    const result = await dialog.showOpenDialog(window, { properties: ["openDirectory"] });
    return result.canceled ? null : result.filePaths[0];
  },
  personal_capability_select_file: async ({ args, window }) => {
    const state = await service.call("personal_capability_status", { creation_project_id: args.creationProjectId });
    if (!state.enabled) throw new Error("个人能力未启用。");
    const selected = await dialog.showOpenDialog(window, { title: "选择本次使用的文件", properties: ["openFile"] });
    if (selected.canceled || !selected.filePaths.length) return null;
    return service.call("personal_capability_register_file", { request: { creationProjectId: args.creationProjectId, packageDigest: state.project.packageDigest, path: selected.filePaths[0] } });
  },
  personal_capability_confirm_file_write: async ({ args, window }) => {
    if (typeof args.content !== "string" || args.content.length > 131072) throw new Error("文件内容超过限制。");
    const target = await service.call("personal_capability_file_write_preview", { request: { creationProjectId: args.creationProjectId, packageDigest: args.packageDigest, grantId: args.grantId, content: args.content } });
    const confirmation = await dialog.showMessageBox(window, { type: "question", title: "写入选定文件", message: `确认写入“${target.name}”？`, detail: `${target.path}\n\n${args.content.slice(0, 2000)}${args.content.length > 2000 ? "\n…（仅展示前 2000 个字符）" : ""}`, buttons: ["取消", "写入"], defaultId: 0, cancelId: 0, noLink: true });
    if (confirmation.response !== 1) return null;
    return service.call("personal_capability_apply_file", { request: { creationProjectId: args.creationProjectId, packageDigest: args.packageDigest, grantId: args.grantId, content: args.content } });
  },
  personal_capability_export: async ({ args, window }) => {
    const result = await dialog.showSaveDialog(window, { defaultPath: "个人应用.zip", filters: [{name:"ZIP",extensions:["zip"]}] });
    if (result.canceled || !result.filePath) return false;
    await service.call("personal_capability_export", { creation_project_id: args.creationProjectId, destination_path: result.filePath });
    return true;
  },
  personal_package_import_prepare: ({ args, window }) => preparePersonalPackageImport({ args, window, dialog, service }),
  workspace_save_copy: async ({ args, window }) => {
    const fileName = String(args?.relativePath ?? "")
      .replaceAll("\\", "/")
      .split("/")
      .filter(Boolean)
      .at(-1);
    if (!fileName) throw new Error("workspace_copy_file_name_missing");
    const result = await dialog.showSaveDialog(window, { defaultPath: fileName });
    if (result.canceled || !result.filePath) return false;
    await service.call("workspace_item_export_copy", {
      local_root: args.localRoot,
      relative_path: args.relativePath,
      destination_path: result.filePath,
    });
    return true;
  },
  workspace_copy_item_content: async ({ args }) => {
    const content = await service.call("workspace_item_clipboard_content", {
      local_root: args.localRoot,
      relative_path: args.relativePath,
    });
    if (content?.kind === "text" && typeof content.text === "string") {
      clipboard.writeText(content.text);
      return null;
    }
    if (content?.kind === "image" && typeof content.dataBase64 === "string") {
      const image = nativeImage.createFromBuffer(Buffer.from(content.dataBase64, "base64"));
      if (image.isEmpty()) throw new Error("workspace_item_clipboard_image_invalid");
      clipboard.writeImage(image);
      return null;
    }
    throw new Error("workspace_item_clipboard_content_invalid");
  },
  skill_pick_archive: async ({ window }) => {
    const result = await dialog.showOpenDialog(window, {
      properties: ["openFile"],
      filters: [{ name: "技能包", extensions: ["zip"] }],
    });
    return result.canceled ? null : result.filePaths[0];
  },
};

function applyLocalProfile(result) {
  const profileId = result?.profileId;
  if (typeof profileId !== "string" || !/^[a-f0-9]{32}$/.test(profileId)) throw new Error("local_profile_missing");
  personalNotifications?.setProfile(profileId);
  browser.setProfile(profileId);
  verifiedPersonalProfileId = profileId;
  return result;
}

/**
 * 宿主服务以 base64 回传字节，此处还原为字节数组——按行分隔的协议本身只承载文本。
 * 界面侧接收 PDF 内容与附件时按字节数组处理，与现有壳的二进制返回等价。
 */
function decodeBinary(value) {
  if (value === null || typeof value !== "object" || typeof value.__binary !== "string") return value;
  return new Uint8Array(Buffer.from(value.__binary, "base64"));
}

async function dispatch({ command, args }, sender) {
  if (BROWSER_WRITE_TOOLS.has(command)) {
    // 接管期间 AI 暂停：两边同时动同一个页面，谁改了什么都说不清，事后也无从复核
    if (browser.takeover) throw new Error("人工接管进行中，AI 操作已暂停");
    // 写操作具有外部副作用，须逐次审批。链路接入 Harness 之前一律拒绝——此处原先只在
    // 带了审批令牌时才拒绝，等于不带令牌反而放行，与红线正相反。
    throw new Error("浏览器写操作的审批链路尚未接入，暂不受理");
  }
  if (isDelegatedCommand(command)) return decodeBinary(await service.call(command, args));
  const handler = implemented[command];
  const window = BrowserWindow.fromWebContents(sender);
  if (!handler) {
    const error = `命令未迁移到新壳：${command}`;
    console.warn("[bridge]", error);
    throw new Error(error);
  }
  return handler({ args, sender, window });
}

/** 把窗口尺寸变化转成界面可监听的事件，界面据此同步最大化状态。 */
function bindWindowEvents(window) {
  const notify = () => emitToRenderer("tauri://resize", {
    width: window.getContentSize()[0],
    height: window.getContentSize()[1],
  });
  window.on("resize", notify);
  window.on("maximize", notify);
  window.on("unmaximize", notify);
}

module.exports = {
  setPersonalPageAi,
  dispatch, emitToRenderer, service, bindWindowEvents, browser, browserMcp,
  browserApprovals,
  setPersonalApplicationCloser,
  setPersonalNotifications,
};
