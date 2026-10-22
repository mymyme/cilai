const { randomUUID } = require("node:crypto");
const { downloadCollectionAttachment } = require("./collection-attachment-download");
const { collectDocument } = require("./browser-collection");
const { session } = require("electron");
const { denyAmbientCapabilities } = require("./session-policy");

/** 控制台与网络各自留多少条近期记录。够 AI 判断刚才那一步发生了什么，又不至于占住内存。 */
const CONSOLE_LIMIT = 100;
/** 导航前最多等附着多久。超过就先加载，记录晚一点开始好过导航卡死。 */
const ATTACH_SETTLE_TIMEOUT_MS = 3000;
const REQUEST_LIMIT = 100;

const partitionName = (profileId) => `persist:ai-browser-${profileId}`;

/**
 * AI 浏览器运行时。
 *
 * 承载方式与业务应用容器相同：内容嵌在工作台界面里，由渲染引擎直接呈现。此前用离屏页面
 * 加画面回传，那条路做不了标签页与开发者工具——离屏没有可附着的窗口，页面之外的原生弹层
 * 也画不出来。改为嵌入承载后画面、指针、键盘、输入法与开发者工具都按常规处理。
 *
 * 内容进程由界面创建（webview 元素附着时才存在），宿主据此接管：施加站点策略、拒绝新窗口、
 * 并按标签登记。AI 的读写工具都作用在当前标签上。
 *
 * 驾驶权只有一个。AI 在操作时员工的站点策略照常生效；员工接手后 AI 的写操作一律被拒，
 * 且不再以站点策略约束员工自己的导航——地址栏若仍受策略约束就没有意义，而策略要约束的
 * 本来就是 AI 能去哪里，不是员工能去哪里。
 */
class BrowserRuntime {
  constructor(policy) {
    this.policy = policy;
    /** tabId -> { contents, debug, runtimeEnabled, axEnabled, conversationId } */
    this.tabs = new Map();
    /*
     * 标签按对话归属，当前标签也按对话各记一个。
     *
     * 浏览器是某一次对话里开出来的，切到别的对话就不该再看到它——那是另一件事的上下文。
     * 记一个全局的当前标签不够：两个对话各自开着标签时，切换对话会把对方的标签顶掉。
     */
    this.activeByConversation = new Map();
    this.lastActiveId = null;
    /** "ai" 或 "employee"。同一时刻只有一方在操作页面。 */
    this.driver = "ai";
    /** 站点范围的取用点，由桥接接入企业侧下发。 */
    this.loadPolicy = null;
    /** 运行状态的对外通知，界面据此呈现标签、地址与驾驶权。 */
    this.onState = null;
    /** 请界面开一个标签：内容进程只能由界面创建，宿主自己建不出来。 */
    this.onOpenTab = null;
    /** tabId -> resolve，等界面把内容进程送上来。 */
    this.waiting = new Map();
    this.profileId = "legacy";
    this.storageMaintenance = null;
  }

  assertStorageAvailable() {
    if (this.storageMaintenance) throw new Error("personal_namespace_browser_paused");
  }

  canAttach() {
    return this.storageMaintenance === null;
  }

  suspendForStorage(expectedSession) {
    this.assertStorageAvailable();
    if (!expectedSession || typeof expectedSession.closeAllConnections !== "function"
        || typeof expectedSession.webRequest?.onBeforeRequest !== "function") {
      throw new Error("personal_namespace_browser_scope_invalid");
    }
    for (const { contents } of this.tabs.values()) {
      if (!contents.isDestroyed() && contents.session !== expectedSession) {
        throw new Error("personal_namespace_browser_scope_invalid");
      }
    }
    const token = {};
    this.storageMaintenance = token;
    try {
      expectedSession.webRequest.onBeforeRequest((_details, callback) => callback({ cancel: true }));
    } catch (error) {
      this.storageMaintenance = null;
      throw error;
    }
    let resumed = false;
    const check = () => {
      if (resumed || this.storageMaintenance !== token) throw new Error("personal_namespace_browser_pause_expired");
    };
    return {
      drain: async () => {
        check();
        for (const { contents } of this.tabs.values()) {
          if (!contents.isDestroyed()) contents.stop();
        }
        await expectedSession.closeAllConnections();
        check();
        if ([...this.tabs.values()].some(({ contents }) => !contents.isDestroyed() && contents.isLoading())) {
          throw new Error("personal_namespace_browser_exit_unconfirmed");
        }
      },
      resume: () => {
        check();
        expectedSession.webRequest.onBeforeRequest(null);
        this.storageMaintenance = null;
        resumed = true;
      },
    };
  }

  /** 会话的取用点单独成一个方法：标签编排可以被检验，不必起真实会话。 */
  sessionOf() {
    this.assertStorageAvailable();
    return session.fromPartition(partitionName(this.profileId));
  }

  isPartition(value) {
    return value === partitionName(this.profileId);
  }

  setProfile(profileId) {
    if (profileId !== "legacy" && profileId !== "signed-out" && !/^[a-f0-9]{32}$/.test(profileId)) {
      throw new Error("browser_profile_invalid");
    }
    if (profileId === this.profileId) return;
    this.assertStorageAvailable();
    this.close();
    this.waiting.clear();
    this.profileId = profileId;
    this.notifyState();
  }

  /** 某个对话此刻的当前标签。不给对话时取最后碰过的那个。 */
  activeIdOf(conversationId) {
    if (conversationId === undefined || conversationId === null) return this.lastActiveId;
    return this.activeByConversation.get(conversationId) ?? null;
  }

  /** 供界面呈现的完整状态。地址与标题取自页面本身，不用调用方传入的值。 */
  state() {
    const tabs = [...this.tabs.entries()]
      .filter(([, tab]) => !tab.contents.isDestroyed())
      .map(([id, tab]) => ({
        id,
        conversationId: tab.conversationId ?? null,
        url: tab.contents.getURL(),
        title: tab.contents.getTitle(),
        loading: tab.contents.isLoading(),
        ...(tab.captureLayout ? { captureLayout: tab.captureLayout } : {}),
        canGoBack: tab.contents.navigationHistory.canGoBack(),
        canGoForward: tab.contents.navigationHistory.canGoForward(),
      }));
    return {
      profileId: this.profileId,
      running: tabs.length > 0,
      driver: this.driver,
      activeId: this.lastActiveId,
      activeByConversation: Object.fromEntries(this.activeByConversation),
      tabs,
    };
  }

  notifyState() {
    this.onState?.(this.state());
  }

  /** 当前标签的内容进程。没有标签时给出空值，由调用方决定是开一个还是报错。 */
  current(conversationId) {
    this.assertStorageAvailable();
    const id = this.activeIdOf(conversationId);
    const tab = id ? this.tabs.get(id) : null;
    if (!tab || tab.contents.isDestroyed()) return null;
    return tab;
  }

  /** AI 导航只接受安全传输，或本机回环地址的 HTTP。 */
  checkOrigin(url) {
    let parsed;
    try {
      parsed = new URL(url);
    } catch {
      return "地址无法解析";
    }
    const loopback = ["localhost", "127.0.0.1", "[::1]"].includes(parsed.hostname);
    if (parsed.protocol !== "https:" && !(parsed.protocol === "http:" && loopback)) {
      return `仅允许 https 与本机回环地址，当前为 ${parsed.protocol}//`;
    }
    /*
     * 站点白名单当前不启用：AI 可以访问任何站点。约束只剩传输安全这一条。
     *
     * 白名单原本是授权边界，由企业侧下发。停用它是一个明确的产品决定，不是"取不到策略
     * 就放行"——真要恢复，是把下面这段判定加回来并重新接上下发通道，而不是改一个开关。
     * 写操作（点击、输入）仍然每次都要员工确认，那条边界没有动。
     */
    return null;
  }

  /** 界面的 webview 附着。此刻内容进程刚建立，页面尚未加载。 */
  adopt(tabId, contents, conversationId = null) {
    this.assertStorageAvailable();
    /*
     * refs 是上一次快照给出的元素句柄表：ref_N → 该元素的后端节点标识。
     * AI 按句柄操作元素，而不是按坐标——坐标一滚动、一改窗口宽度就全废，句柄绑在元素上。
     */
    const tab = { contents, debug: contents.debugger, runtimeEnabled: false, axEnabled: false,
      conversationId, refs: new Map(), refSeq: 0, console: [], requests: [] };
    this.tabs.set(tabId, tab);
    denyAmbientCapabilities(this.sessionOf(), "browser");
    /*
     * 标签一接管就附着，网络记录因此覆盖它这一生的全部加载——等到第一次读页面才附着，
     * 首屏那一段就永远看不到了，而"页面为什么是空的"要查的恰恰是那一段。
     *
     * 不等它完成：在还没有任何文档的渲染器上，Runtime.enable 与 Network.enable 会永久
     * 挂起（实测如此）。界面创建标签时带着 about:blank，正常情况下有文档；万一没有，
     * 附着悬在那里也不该把接管标签这件事一起拖住。
     */
    tab.attaching = this.attach(tab).catch((error) => {
      console.warn(`[browser] 标签 ${tabId} 附着失败，网络记录将从首次读取页面时开始：${error.message}`);
    });
    /*
     * 页面自身发起的跳转在 AI 操作期间同样受站点策略约束。只在导航命令上判定是不够的——
     * 页面里的一次跳转就能把 AI 带到策略之外，而 AI 未必看得出自己已经越界。
     * 员工接手之后不再约束：那时是员工在浏览，策略管的是 AI。
     */
    const guard = (event, target) => {
      if (this.driver !== "ai") return;
      const refusal = this.checkOrigin(target);
      if (!refusal) return;
      event.preventDefault();
      console.warn(`[browser] 导航越界已拒绝：${target}（${refusal}）`);
    };
    /*
     * 控制台与网络各留一段近期记录，供 AI 自证"这一步到底成没成"。
     *
     * 没有这个，AI 只能凭工具返回值猜；工具返回成功而页面其实报了错的情况，它看不出来，
     * 于是照着编一段"已经完成了"。两段记录都是环形的，只留最近若干条，不占内存。
     */
    contents.on("console-message", (event) => {
      tab.console.push({
        level: event.level ?? "info",
        text: String(event.message ?? "").slice(0, 400),
        source: event.sourceId ? `${event.sourceId}:${event.lineNumber ?? 0}` : undefined,
        at: Date.now(),
      });
      if (tab.console.length > CONSOLE_LIMIT) tab.console.shift();
    });

    // 换了文档，上一次快照的句柄全部失效：拿旧句柄去点会点到别的元素上，必须让它明确失败
    contents.on("did-navigate", () => { tab.refs.clear(); });
    contents.on("did-navigate-in-page", () => { tab.refs.clear(); });
    contents.on("will-navigate", (event, target) => guard(event, target));
    contents.on("will-frame-navigate", (event) => guard(event, event.url));
    contents.on("will-redirect", (event, target) => guard(event, target));
    // 新窗口不在标签登记内，也不受站点策略约束，因此不另开窗口，改为在新标签里打开
    contents.setWindowOpenHandler(({ url }) => {
      void this.openTab(url, { byEmployee: this.driver !== "ai" }).catch((error) =>
        console.warn(`[browser] 新标签打开失败：${error.message ?? error}`));
      return { action: "deny" };
    });
    for (const event of ["did-navigate", "did-navigate-in-page", "page-title-updated",
      "did-start-loading", "did-stop-loading"]) {
      contents.on(event, () => this.notifyState());
    }
    contents.once("destroyed", () => {
      if (this.tabs.get(tabId) === tab) this.tabs.delete(tabId);
      this.forgetTab(tabId, tab.conversationId ?? null);
      this.notifyState();
    });
    this.activeByConversation.set(conversationId, tabId);
    this.lastActiveId = tabId;
    this.waiting.get(tabId)?.();
    this.waiting.delete(tabId);
    this.notifyState();
  }

  /** 开一个标签并等界面把内容进程送上来。界面先渲染元素、宿主再接管，顺序由这里把住。 */
  async openTab(url, { byEmployee = false, conversationId = null } = {}) {
    this.assertStorageAvailable();
    if (!this.onOpenTab) throw new Error("界面未提供浏览器承载，无法打开标签");
    const tabId = `tab-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
    const ready = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("browser_tab_attach_timeout")), 10000);
      this.waiting.set(tabId, () => {
        clearTimeout(timer);
        resolve();
      });
    });
    this.onOpenTab(tabId, conversationId);
    await ready;
    if (url) await (byEmployee ? this.navigateByEmployee(url, conversationId) : this.navigate(url, conversationId));
    else this.notifyState();
    return { tabId };
  }

  closeTab(tabId) {
    const tab = this.tabs.get(tabId);
    if (!tab) return { closed: false };
    this.tabs.delete(tabId);
    this.forgetTab(tabId, tab.conversationId ?? null);
    this.notifyState();
    return { closed: true };
  }

  /** 撤销一个标签后补上当前标签：只在它所属的对话里找替补，不跨对话顶替。 */
  forgetTab(tabId, conversationId) {
    if (this.lastActiveId === tabId) this.lastActiveId = null;
    if (this.activeByConversation.get(conversationId) !== tabId) return;
    const sibling = [...this.tabs.entries()]
      .find(([, item]) => (item.conversationId ?? null) === conversationId);
    if (sibling) this.activeByConversation.set(conversationId, sibling[0]);
    else this.activeByConversation.delete(conversationId);
    this.lastActiveId = this.lastActiveId ?? (sibling ? sibling[0] : null);
  }

  activateTab(tabId) {
    const tab = this.tabs.get(tabId);
    if (!tab) throw new Error("未知标签");
    this.activeByConversation.set(tab.conversationId ?? null, tabId);
    this.lastActiveId = tabId;
    this.notifyState();
    return { activeId: tabId };
  }

  /** 驾驶权。同一时刻只有一方操作页面，两边同时动同一个页面就说不清谁改了什么。 */
  setDriver(driver) {
    this.driver = driver === "employee" ? "employee" : "ai";
    this.notifyState();
    return { driver: this.driver };
  }

  /** 保证有一个可用标签。AI 的工具可能在界面还没开过标签时就被调用。 */
  async ensure(conversationId) {
    if (this.current(conversationId)) return;
    await this.openTab(null, { conversationId: conversationId ?? null });
  }

  /**
   * 列出当前全部标签。
   *
   * 员工自己开的标签同样在内：策略约束的是 AI 自己能去哪里，员工在工作台里打开的页面
   * 等于递到 AI 面前，读它是这套东西的用处所在。页面内容按数据看待，不作指令。
   */
  listTabs(conversationId) {
    const all = this.state().tabs;
    const scoped = conversationId === undefined
      ? all
      : all.filter((tab) => tab.conversationId === (conversationId ?? null));
    const activeId = this.activeIdOf(conversationId);
    return { activeId, tabs: scoped.map((tab) => ({ ...tab, active: tab.id === activeId })) };
  }

  /** 附着调试器并启用所需的域，重复调用无副作用。 */
  async attach(tab) {
    if (!tab.debug.isAttached()) tab.debug.attach("1.3");
    if (!tab.runtimeEnabled) {
      await tab.debug.sendCommand("Runtime.enable");
      tab.runtimeEnabled = true;
    }
    /*
     * 网络记录走调试协议而不是会话级拦截：会话是整个分区共用的，拿不到"这一条属于哪个
     * 标签"，而 AI 问的永远是"我刚操作的那个页面发生了什么"。
     */
    if (!tab.networkEnabled) {
      tab.debug.on("message", (_event, method, params) => {
        if (method === "Network.responseReceived") {
          tab.requests.push({
            url: String(params.response?.url ?? "").slice(0, 300),
            status: params.response?.status,
            type: params.type,
            at: Date.now(),
          });
          if (tab.requests.length > REQUEST_LIMIT) tab.requests.shift();
        } else if (method === "Network.loadingFailed") {
          tab.requests.push({
            url: undefined,
            status: 0,
            failed: String(params.errorText ?? "加载失败"),
            type: params.type,
            at: Date.now(),
          });
          if (tab.requests.length > REQUEST_LIMIT) tab.requests.shift();
        }
      });
      await tab.debug.sendCommand("Network.enable");
      tab.networkEnabled = true;
    }
    return tab;
  }

  /**
   * 等接管标签时发起的那次附着落定，再让页面开始加载。
   *
   * 不等的话，Network.enable 会落在首屏请求之后，整段加载记录就丢了——附着本身是在
   * 接管标签时就发起的，这里等的是它的结果，不会重复发起。
   *
   * 带期限：在还没有任何文档的渲染器上，Runtime.enable 会永久挂起（实测如此）。界面创建
   * 标签时带着 about:blank，正常不会走到这一步；真走到了，宁可网络记录晚一点开始，
   * 也不能让导航本身卡死——那会让整轮对话停在这里且看不出原因。
   */
  async settleAttach(tab, timeoutMs = ATTACH_SETTLE_TIMEOUT_MS) {
    if (!tab.attaching) return;
    let timer;
    const deadline = new Promise((resolve) => {
      timer = setTimeout(() => {
        console.warn("[browser] 附着未在限期内完成，网络记录将从稍后开始");
        resolve();
      }, timeoutMs);
    });
    await Promise.race([tab.attaching, deadline]);
    clearTimeout(timer);
  }

  /** 近期控制台记录。level 可筛，只要报错时传 "error"。 */
  async consoleMessages({ level, limit = 50 } = {}, tabId, conversationId) {
    const tab = await this.connect(tabId, conversationId);
    const entries = level
      ? tab.console.filter((entry) => String(entry.level) === String(level))
      : tab.console;
    return { url: tab.contents.getURL(), entries: entries.slice(-limit) };
  }

  /** 近期网络记录。failedOnly 只看失败的那些，排查"页面为什么是空的"最直接。 */
  async networkRequests({ failedOnly = false, limit = 50 } = {}, tabId, conversationId) {
    const tab = await this.connect(tabId, conversationId);
    const entries = failedOnly
      ? tab.requests.filter((entry) => entry.failed || (entry.status ?? 0) >= 400)
      : tab.requests;
    return { url: tab.contents.getURL(), entries: entries.slice(-limit) };
  }

  /**
   * 取要操作的标签并附着调试器。
   *
   * 指定标签时直接用它，并把它记为所属对话的当前标签——AI 读员工正在看的页面走的就是
   * 这条路。不指定时取该对话自己的当前标签，没有就开一个。
   */
  async connect(tabId, conversationId) {
    this.assertStorageAvailable();
    if (tabId) {
      const target = this.tabs.get(tabId);
      if (!target) throw new Error(`未知标签：${tabId}`);
      this.activeByConversation.set(target.conversationId ?? null, tabId);
      this.lastActiveId = tabId;
      return this.attach(target);
    }
    await this.ensure(conversationId);
    return this.attach(this.current(conversationId));
  }

  /** AI 发起的导航。站点白名单当前不启用，只判传输安全。 */
  async navigate(url, conversationId) {
    const refusal = this.checkOrigin(url);
    if (refusal) throw new Error(`站点策略拒绝：${refusal}`);
    await this.ensure(conversationId);
    const tab = this.current(conversationId);
    await this.settleAttach(tab);
    await tab.contents.loadURL(url);
    return { url: tab.contents.getURL(), title: tab.contents.getTitle() };
  }

  /**
   * 员工发起的导航，不经站点策略。
   *
   * 策略约束的是 AI 能去哪里；员工在自己的工作台里输入地址，与他在系统浏览器里输入没有
   * 区别，用同一套策略挡住他只会让地址栏形同虚设。
   */
  async navigateByEmployee(url, conversationId) {
    this.setDriver("employee");
    await this.ensure(conversationId);
    const tab = this.current(conversationId);
    await this.settleAttach(tab);
    await tab.contents.loadURL(url);
    return { url: tab.contents.getURL(), title: tab.contents.getTitle() };
  }

  goBack(conversationId) {
    this.current(conversationId)?.contents.navigationHistory.goBack();
    return null;
  }

  goForward(conversationId) {
    this.current(conversationId)?.contents.navigationHistory.goForward();
    return null;
  }

  reload(conversationId) {
    this.current(conversationId)?.contents.reload();
    return null;
  }

  /** 开发者工具挂在标签自身上，与浏览器里的"检查"一致。 */
  toggleDevTools(conversationId) {
    const tab = this.current(conversationId);
    if (!tab) throw new Error("没有可检查的标签");
    if (tab.contents.isDevToolsOpened()) tab.contents.closeDevTools();
    else tab.contents.openDevTools({ mode: "bottom" });
    return { opened: tab.contents.isDevToolsOpened() };
  }

  /**
   * 页面快照，取可访问性树。
   *
   * 相比截图，可访问性树对 AI 更准确也更省上下文：它给出角色、名称与状态，而不是让模型
   * 从像素里猜。截图留给可访问性树不足以判断的视觉场景。
   */
  async snapshot(maxNodes = 200, tabId, conversationId) {
    const tab = await this.connect(tabId, conversationId);
    // 可访问性域按需启用：在空白页上预先启用会挂住，故推迟到真正取树时
    if (!tab.axEnabled) {
      await tab.debug.sendCommand("Accessibility.enable");
      tab.axEnabled = true;
    }
    const { nodes } = await tab.debug.sendCommand("Accessibility.getFullAXTree");
    /*
     * 每次快照重建句柄表。给出的 ref 是操作元素的唯一凭据——此前快照返回的是可访问性树的
     * 内部节点号，而点击只收坐标，那个号没有任何工具能用，AI 只能靠截图目测像素去点。
     */
    tab.refs.clear();
    const useful = nodes
      .filter((node) => !node.ignored && node.role?.value && node.name?.value)
      .slice(0, maxNodes)
      .map((node) => {
        const ref = `ref_${(tab.refSeq += 1)}`;
        if (node.backendDOMNodeId !== undefined) tab.refs.set(ref, node.backendDOMNodeId);
        return {
          ref,
          role: node.role.value,
          name: node.name.value,
          value: node.value?.value,
          focusable: node.properties?.some((p) => p.name === "focusable" && p.value?.value),
        };
      })
      // 拿不到后端节点的元素给不出可用句柄，列出来只会让 AI 去点一个必然失败的东西
      .filter((node) => tab.refs.has(node.ref));
    return { url: tab.contents.getURL(), nodes: useful, truncated: nodes.length > maxNodes };
  }

  /**
   * 截图。带期限，超时即失败并说明原因。
   *
   * 页面没有呈现在界面上时没有帧可取，截图会一直等下去——浏览器归属某一次对话，员工切到
   * 别处它就不呈现，这种情形是常态而非异常。让工具带着原因失败，AI 才能改用快照或取文；
   * 不设期限则整轮对话卡死在这里，且看不出卡在哪。
   */
  requestCaptureLayout(tab, size) {
    const id = randomUUID();
    tab.captureLayout = { id, size };
    const ready = new Promise((resolve, reject) => { tab.captureLayoutWaiter = { id, resolve, reject }; });
    this.notifyState();
    return ready;
  }

  captureLayoutReady({ tabId, requestId, width, height }) {
    const tab = this.tabs.get(tabId), pending = tab?.captureLayoutWaiter;
    if (!pending || pending.id !== requestId) return { accepted: false };
    const size = tab.captureLayout.size;
    if (![width, height].every(Number.isFinite) || width <= 0 || height <= 0 ||
        (size && (Math.abs(width - size.width) > 1 || Math.abs(height - size.height) > 1))) {
      pending.reject(new Error("浏览器承载尺寸未就绪"));
    } else pending.resolve();
    delete tab.captureLayoutWaiter;
    return { accepted: true };
  }

  async screenshot(tabId, conversationId, timeoutMs = 3000) {
    const tab = await this.connect(tabId, conversationId);
    if (tab.capturing) throw new Error("该页面正在截图，请稍后重试");
    tab.capturing = true;
    const fullPage = typeof conversationId === "string" && conversationId.startsWith("application:");
    const deadline = Date.now() + timeoutMs;
    const within = async (promise, limit = Math.max(1, deadline - Date.now())) => {
      let timer;
      try {
        return await Promise.race([promise, new Promise((_resolve, reject) => {
          timer = setTimeout(() => reject(new Error(fullPage
            ? "页面截图超时，未取得完整画面"
            : "页面当前未呈现在界面上，取不到画面。切到该对话即可看到；页面结构与文本可改用快照或取文。")), limit);
        })]);
      } finally { clearTimeout(timer); }
    };
    let emulated = false, layoutRequested = false;
    try {
      const params = { format: "jpeg", quality: 70 };
      if (fullPage) {
        const { cssContentSize: size } = await within(tab.debug.sendCommand("Page.getLayoutMetrics"));
        if (!size || ![size.x, size.y, size.width, size.height].every(Number.isFinite) || size.x < 0 || size.y < 0 || size.width <= 0 || size.height <= 0)
          throw new Error("无法确定完整页面尺寸");
        const width = Math.ceil(size.x + size.width), height = Math.ceil(size.y + size.height);
        if (width > 8000 || height > 16000 || width * height > 16000000)
          throw new Error("页面超出完整截图尺寸限制");
        layoutRequested = true;
        await within(this.requestCaptureLayout(tab, { width, height }));
        emulated = true;
        await within(tab.debug.sendCommand("Emulation.setDeviceMetricsOverride", {
          width, height, deviceScaleFactor: 1, mobile: false, scale: 1,
        }));
        await within(tab.debug.sendCommand("Runtime.evaluate", {
          expression: "new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))",
          awaitPromise: true, returnByValue: true,
        }));
        const { cssContentSize: rendered } = await within(tab.debug.sendCommand("Page.getLayoutMetrics"));
        if (!rendered || ![rendered.x, rendered.y, rendered.width, rendered.height].every(Number.isFinite) ||
            rendered.x < 0 || rendered.y < 0 || rendered.width <= 0 || rendered.height <= 0 ||
            rendered.x + rendered.width > width || rendered.y + rendered.height > height)
          throw new Error("页面布局发生变化，无法取得完整截图");
        params.captureBeyondViewport = false;
        params.fromSurface = true;
        params.clip = { x: 0, y: 0, width, height, scale: 1 };
      }
      const result = await within(tab.debug.sendCommand("Page.captureScreenshot", params));
      return { data: result.data, format: "jpeg" };
    } finally {
      try {
        if (emulated) await within(tab.debug.sendCommand("Emulation.clearDeviceMetricsOverride"), 3000);
      } finally {
        try {
          if (layoutRequested) await within(this.requestCaptureLayout(tab, null), 3000);
        } finally {
          delete tab.captureLayout;
          delete tab.captureLayoutWaiter;
          tab.capturing = false;
          this.notifyState();
        }
      }
    }
  }

  async collect(selector, linkSelector, tabId, conversationId, attachmentCapture = false) {
    if (typeof selector !== "string" || !selector || selector.length > 300 || typeof linkSelector !== "string" || linkSelector.length > 300) throw new Error("采集选择规则无效");
    const tab = await this.connect(tabId, conversationId);
    const expression = `(${collectDocument.toString()})(${JSON.stringify(selector)}, ${JSON.stringify(linkSelector)}, ${JSON.stringify(attachmentCapture)})`;
    const { result, exceptionDetails } = await tab.debug.sendCommand("Runtime.evaluate", { expression, returnByValue: true });
    if (exceptionDetails) throw new Error("页面采集失败：" + exceptionDetails.text);
    return {...result.value,obtainedAt:new Date().toISOString()};
  }

  async captureAttachment(args, conversationId, origins) {
    const tab = await this.connect(args.tabId, conversationId);
    const currentUrl = tab.contents.getURL(), profileId = this.profileId, browserSession = this.sessionOf();
    if (!origins.includes(new URL(currentUrl).origin)) throw new Error("附件来源页面超出本次授权");
    const { result, exceptionDetails } = await tab.debug.sendCommand("Runtime.evaluate", {
      expression: `Array.from(document.querySelectorAll('a[href],iframe[src],embed[src],object[data]')).some(node => { try { return new URL(node.getAttribute('href') || node.getAttribute('src') || node.getAttribute('data'), location.href).href === ${JSON.stringify(args.url)}; } catch { return false; } })`, returnByValue: true,
    });
    if (exceptionDetails || result.value !== true) throw new Error("附件未由当前实际页面声明");
    const current = () => {
      if (this.profileId !== profileId || tab.contents.isDestroyed() || tab.contents.getURL() !== currentUrl) throw new Error("附件下载期间来源或身份已变化");
    };
    current();
    const evidence = await downloadCollectionAttachment((url, init) => { current(); return browserSession.fetch(url, init); }, args, origins, url => this.checkOrigin(url));
    current();
    return evidence;
  }

  async extract(selector, tabId, conversationId) {
    const tab = await this.connect(tabId, conversationId);
    const expression = selector
      ? `Array.from(document.querySelectorAll(${JSON.stringify(selector)})).map((n) => n.innerText).join("\\n")`
      : "document.body.innerText";
    const { result, exceptionDetails } = await tab.debug.sendCommand("Runtime.evaluate", {
      expression,
      returnByValue: true,
    });
    if (exceptionDetails) throw new Error(`页面取文失败：${exceptionDetails.text}`);
    return { text: String(result.value).slice(0, 20000) };
  }

  /** AI 的写操作。员工接手期间一律拒绝——两边同时动同一个页面，事后无从复核。 */
  /**
   * 把快照给出的句柄解析成页面上的元素，并返回它的可点位置。
   *
   * 句柄失效是常态而非异常——页面一跳转、一重新渲染，上一次快照的句柄就不再指向同一个
   * 元素。这种情况必须明确失败并让 AI 重新取快照，不能退化成"点个大概位置"：点错地方
   * 比点不到危险得多，而 AI 无法从结果里看出自己点错了。
   */
  async resolveRef(tab, ref) {
    const backendNodeId = tab.refs.get(ref);
    if (backendNodeId === undefined) {
      throw new Error(`句柄 ${ref} 不在当前页面的快照里，请重新调用 browser_snapshot 取句柄`);
    }
    const resolved = await tab.debug.sendCommand("DOM.resolveNode", { backendNodeId });
    const objectId = resolved?.object?.objectId;
    if (!objectId) throw new Error(`句柄 ${ref} 已失效，页面可能已经变化，请重新取快照`);
    // 元素可能在视口外，先滚进来再取位置，否则算出的坐标落在可视区之外
    const box = await tab.debug.sendCommand("Runtime.callFunctionOn", {
      objectId,
      returnByValue: true,
      functionDeclaration: `function () {
        this.scrollIntoView({ block: "center", inline: "center" });
        const rect = this.getBoundingClientRect();
        return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2,
          width: rect.width, height: rect.height };
      }`,
    });
    const position = box?.result?.value;
    if (!position || position.width <= 0 || position.height <= 0) {
      throw new Error(`句柄 ${ref} 对应的元素当前不可见，无法操作`);
    }
    return { objectId, x: Math.round(position.x), y: Math.round(position.y) };
  }

  assertAiMayWrite() {
    if (this.driver !== "ai") throw new Error("员工正在操作该页面，AI 的写操作已暂停");
  }

  /**
   * 点击。优先按快照给出的句柄定位，没有句柄才退到坐标。
   *
   * 句柄绑在元素上，页面滚动、窗口改宽、缩放变化都不影响；坐标是 AI 从截图上目测出来的，
   * 上述任何一项一变就点到别处，而且标签没呈现时根本取不到截图。坐标这条路保留只是为了
   * 极少数快照里没有的元素（如画布内的图形），不是常规做法。
   */
  async click({ ref, x, y }, conversationId) {
    this.assertAiMayWrite();
    await this.ensure(conversationId);
    const tab = this.current(conversationId);
    let point = { x, y };
    if (ref) {
      await this.attach(tab);
      const resolved = await this.resolveRef(tab, ref);
      point = { x: resolved.x, y: resolved.y };
    } else if (!Number.isFinite(x) || !Number.isFinite(y)) {
      throw new Error("点击需要 ref 或者 x、y 坐标；建议先用 browser_snapshot 取句柄");
    }
    for (const type of ["mouseDown", "mouseUp"]) {
      tab.contents.sendInputEvent({ type, x: point.x, y: point.y, button: "left", clickCount: 1 });
    }
    return { clicked: ref ? { ref, ...point } : point };
  }

  /**
   * 直接给表单元素设值，并派发页面能听到的事件。
   *
   * 下拉框、复选框用"点开再点选项"去凑极不可靠：选项可能渲染在页面之外、可能是原生控件
   * 根本不在 DOM 里。按句柄设值一步到位，还能覆盖复选框与文本框。
   */
  async setValue(ref, value, conversationId) {
    this.assertAiMayWrite();
    const tab = await this.connect(undefined, conversationId);
    const { objectId } = await this.resolveRef(tab, ref);
    const applied = await tab.debug.sendCommand("Runtime.callFunctionOn", {
      objectId,
      returnByValue: true,
      arguments: [{ value }],
      functionDeclaration: `function (next) {
        const tag = this.tagName ? this.tagName.toLowerCase() : "";
        if (tag === "select") {
          const wanted = String(next);
          const option = Array.from(this.options).find((item) =>
            item.value === wanted || item.text === wanted);
          if (!option) return { ok: false, reason: "没有这个选项", options: Array.from(this.options).map((item) => item.text) };
          this.value = option.value;
        } else if (this.type === "checkbox" || this.type === "radio") {
          this.checked = next === true || next === "true" || next === "on";
        } else if ("value" in this) {
          this.value = String(next);
        } else if (this.isContentEditable) {
          this.textContent = String(next);
        } else {
          return { ok: false, reason: "这个元素不接受设值" };
        }
        // 页面通常监听的是这两个事件；只改 value 不派发，框架层读不到新值
        this.dispatchEvent(new Event("input", { bubbles: true }));
        this.dispatchEvent(new Event("change", { bubbles: true }));
        return { ok: true, value: this.value ?? this.textContent, checked: this.checked };
      }`,
    });
    const result = applied?.result?.value;
    if (!result?.ok) {
      throw new Error(`句柄 ${ref} 设值未成功：${result?.reason ?? "未知原因"}${
        result?.options ? `（可选项：${result.options.join("、")}）` : ""}`);
    }
    return { ref, value: result.value, checked: result.checked };
  }

  async type(text, conversationId) {
    this.assertAiMayWrite();
    const tab = await this.connect(undefined, conversationId);
    // 经调试协议写入：可覆盖组合输入，也不受合成按键的可信标记限制
    await tab.debug.sendCommand("Input.insertText", { text });
    return { typed: text.length };
  }

  async scroll(deltaY, conversationId) {
    await this.ensure(conversationId);
    // 滚轮必须走主进程输入接口
    this.current(conversationId).contents.sendInputEvent({
      type: "mouseWheel", x: 640, y: 400, deltaX: 0, deltaY, canScroll: true,
    });
    return { scrolled: deltaY };
  }

  /** 以条件成立为准，不以固定延时代替确定性等待。 */
  async waitFor(expression, timeoutMs = 10000, conversationId) {
    const tab = await this.connect(undefined, conversationId);
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const { result } = await tab.debug.sendCommand("Runtime.evaluate", {
        expression, returnByValue: true,
      });
      if (result.value) return { satisfied: true };
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
    throw new Error(`等待条件未在 ${timeoutMs} 毫秒内成立`);
  }

  /** 关闭全部标签。驾驶权随之复位，下次打开不带着上一轮的状态。 */
  close() {
    if (this.tabs.size === 0) return;
    this.tabs.clear();
    this.activeByConversation.clear();
    this.lastActiveId = null;
    this.driver = "ai";
    this.notifyState();
  }

}

module.exports = { BrowserRuntime, browserPartitionName: partitionName };
