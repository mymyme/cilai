const assert = require("node:assert/strict");
const test = require("node:test");
const { BrowserRuntime } = require("./browser");

/** 内容进程的替身：只记录施加了哪些管控与调用，不起真实进程。 */
function fakeContents() {
  const handlers = new Map();
  return {
    destroyed: false,
    url: "about:blank",
    title: "",
    devtools: false,
    inputs: [],
    windowOpenHandler: null,
    isDestroyed() { return this.destroyed; },
    isLoading() { return false; },
    getURL() { return this.url; },
    getTitle() { return this.title; },
    navigationHistory: { canGoBack: () => false, canGoForward: () => false, goBack() {}, goForward() {} },
    reload() {},
    isDevToolsOpened() { return this.devtools; },
    openDevTools() { this.devtools = true; },
    closeDevTools() { this.devtools = false; },
    sendInputEvent(event) { this.inputs.push(event.type); },
    async loadURL(url) { this.url = url; },
    setWindowOpenHandler(handler) { this.windowOpenHandler = handler; },
    on(name, handler) { handlers.set(name, handler); },
    once(name, handler) { handlers.set(name, handler); },
    emit(name, ...args) { return handlers.get(name)?.(...args); },
    debugger: {
      attached: false,
      listeners: [],
      isAttached() { return this.attached; },
      attach() { this.attached = true; },
      // 网络记录经调试协议的消息事件送达，替身要能挂监听
      on(event, handler) { this.listeners.push([event, handler]); },
      emit(event, method, params) {
        for (const [name, handler] of this.listeners) if (name === event) handler({}, method, params);
      },
      sendCommand: async (method) =>
        method === "Runtime.evaluate" ? { result: { value: "页面正文" } } : {},
    },
  };
}

/** 界面的替身：宿主请求开标签时立即把内容进程送上来。 */
/** 会话的替身：环境能力策略照常施加，方法都是空实现。 */
function fakeSession() {
  return {
    setPermissionRequestHandler() {}, setPermissionCheckHandler() {},
    setDevicePermissionHandler() {}, on() {}, clearStorageData: async () => undefined,
  };
}

function runtimeWithUi(policy = { allowedHosts: [] }) {
  const runtime = new BrowserRuntime(policy);
  const store = fakeSession();
  runtime.sessionOf = () => store;
  runtime.onState = (state) => {
    for (const tab of state.tabs) if (tab.captureLayout) queueMicrotask(() => runtime.captureLayoutReady({
      tabId: tab.id, requestId: tab.captureLayout.id,
      width: tab.captureLayout.size?.width ?? 500, height: tab.captureLayout.size?.height ?? 700,
    }));
  };
  const created = [];
  runtime.onOpenTab = (tabId, conversationId) => {
    const contents = fakeContents();
    created.push({ tabId, conversationId, contents });
    queueMicrotask(() => runtime.adopt(tabId, contents, conversationId ?? null));
  };
  return { runtime, created };
}

test("未运行时状态为停止，标签为空", () => {
  const runtime = new BrowserRuntime({ allowedHosts: [] });
  const state = runtime.state();
  assert.equal(state.running, false);
  assert.deepEqual(state.tabs, []);
  assert.equal(state.driver, "ai");
  assert.equal(state.profileId, "legacy");
});

test("存储维护阻止新导航并等全部在途连接结束，恢复不清除登录资料", async () => {
  const runtime = new BrowserRuntime({});
  let handler = null, finishConnections, stopped = false;
  const connectionExit = new Promise(resolve => { finishConnections = resolve; });
  const session = {
    webRequest: { onBeforeRequest(value) { handler = value; } },
    closeAllConnections() { return connectionExit; },
    clearStorageData() { throw new Error("must retain login storage"); },
  };
  runtime.tabs.set("active", {contents:{session,isDestroyed:()=>false,stop(){stopped=true;},isLoading:()=>false}});
  const lease = runtime.suspendForStorage(session);
  assert.equal(runtime.canAttach(), false);
  let decision;
  handler({}, value=>{decision=value;});
  assert.deepEqual(decision, {cancel:true});
  assert.throws(()=>runtime.setProfile("a".repeat(32)), /browser_paused/);
  await assert.rejects(runtime.openTab("https://example.com"), /browser_paused/);
  let drained = false;
  const pending = lease.drain().then(()=>{drained=true;});
  assert.equal(stopped, true);
  await Promise.resolve();
  assert.equal(drained, false);
  finishConnections();
  await pending;
  lease.resume();
  assert.equal(handler, null);
  assert.equal(runtime.canAttach(), true);
  assert.equal(typeof runtime.clearSession, "undefined");
  assert.throws(()=>lease.resume(), /pause_expired/);
});

test("缓存维护不接受其他会话，停止加载未确认时保持暂停", async () => {
  const runtime = new BrowserRuntime({});
  const session = {webRequest:{onBeforeRequest(){}},closeAllConnections:async()=>{}};
  const contents = {session,isDestroyed:()=>false,stop(){},isLoading:()=>true};
  runtime.tabs.set("active", {contents});
  assert.throws(()=>runtime.suspendForStorage({...session}), /scope_invalid/);
  assert.equal(runtime.canAttach(), true);
  const lease = runtime.suspendForStorage(session);
  await assert.rejects(lease.drain(), /exit_unconfirmed/);
  assert.equal(runtime.canAttach(), false);
  contents.isLoading=()=>false;
  await lease.drain();
  lease.resume();
});

test("AI 浏览器按租户数据上下文使用独立分区", () => {
  const runtime = new BrowserRuntime({ allowedHosts: [] });
  runtime.setProfile("b".repeat(32));
  assert.equal(runtime.state().profileId, "b".repeat(32));
  assert.equal(runtime.isPartition(`persist:ai-browser-${"b".repeat(32)}`), true);
  assert.equal(runtime.isPartition("persist:ai-browser-legacy"), false);
  runtime.setProfile("signed-out");
  assert.equal(runtime.isPartition("persist:ai-browser-signed-out"), true);
  assert.throws(() => runtime.setProfile("tenant-b"), /browser_profile_invalid/);
});

test("界面未提供承载时开标签报错，不无声失败", async () => {
  const runtime = new BrowserRuntime({ allowedHosts: [] });
  await assert.rejects(() => runtime.openTab("https://a.example.com/"), /界面未提供浏览器承载/);
});

test("驾驶权切换通知界面，界面因此不必自行记账", async () => {
  const { runtime } = runtimeWithUi();
  const seen = [];
  runtime.onState = (state) => seen.push(state.driver);
  runtime.setDriver("employee");
  runtime.setDriver("ai");
  assert.deepEqual(seen, ["employee", "ai"]);
});

test("站点白名单不启用时 AI 可以去任何站点，传输安全那条仍然拦得住", async () => {
  const { runtime } = runtimeWithUi({ allowedHosts: [] });
  await runtime.openTab(null);
  // 白名单停用是明确的产品决定：任何 https 站点都放行，不再有"未声明即拒绝"
  const opened = await runtime.navigate("https://anything.example/");
  assert.equal(opened.url, "https://anything.example/");
  // 传输安全不是白名单，没有跟着一起去掉
  await assert.rejects(() => runtime.navigate("http://anything.example/"), /仅允许 https/);
  await assert.rejects(() => runtime.navigate("ftp://localhost/file"), /仅允许 https/);
  assert.equal(runtime.checkOrigin("http://localhost/"), null);
  assert.equal(runtime.checkOrigin("http://[::1]/"), null);
  const result = await runtime.navigateByEmployee("https://anything.example/");
  assert.equal(result.url, "https://anything.example/");
  assert.equal(runtime.state().driver, "employee");
});

test("员工接手期间 AI 的写操作被拒，两边不同时动同一个页面", async () => {
  const { runtime } = runtimeWithUi({ allowedHosts: ["a.example.com"] });
  await runtime.openTab(null);
  runtime.setDriver("employee");
  await assert.rejects(() => runtime.click({ x: 1, y: 1 }), /写操作已暂停/);
  await assert.rejects(() => runtime.type("x"), /写操作已暂停/);
  runtime.setDriver("ai");
  await runtime.click({ x: 1, y: 1 });
  assert.deepEqual(runtime.current().contents.inputs, ["mouseDown", "mouseUp"]);
});

test("列出的标签包含员工自己打开的，AI 据此读取员工正在看的页面", async () => {
  const { runtime } = runtimeWithUi({ allowedHosts: ["a.example.com"] });
  await runtime.openTab(null);
  const own = runtime.state().activeId;
  await runtime.openTab("https://evil.example/", { byEmployee: true });
  const listed = runtime.listTabs();
  assert.equal(listed.tabs.length, 2);
  assert.ok(listed.tabs.some((tab) => tab.url === "https://evil.example/"),
    "员工手动打开的站点同样列出：策略约束的是 AI 自己能去哪里");
  assert.notEqual(listed.activeId, own);
});

test("读取可以指定标签，未知标签明确报错", async () => {
  const { runtime } = runtimeWithUi({ allowedHosts: ["a.example.com"] });
  await runtime.openTab(null);
  await assert.rejects(() => runtime.extract(undefined, "tab-不存在"), /未知标签/);
});

test("页面自身发起的跳转同样只判传输安全，https 一律放行", async () => {
  const { runtime } = runtimeWithUi({ allowedHosts: [] });
  await runtime.openTab(null);
  const contents = runtime.current().contents;
  let prevented = 0;
  const bump = { preventDefault: () => { prevented += 1; } };
  contents.emit("will-navigate", bump, "https://anything.example/");
  assert.equal(prevented, 0, "白名单停用后 https 跳转不再被拦");
  // 非 https 仍然拦下来：这条守的是传输安全，不是可访问范围
  contents.emit("will-navigate", bump, "http://anything.example/");
  assert.equal(prevented, 1);
  // 员工接手之后一概不约束：那时是员工在浏览
  runtime.setDriver("employee");
  contents.emit("will-navigate", bump, "http://anything.example/");
  assert.equal(prevented, 1);
});

test("新窗口不另开窗口，改为在新标签里打开", async () => {
  const { runtime, created } = runtimeWithUi({ allowedHosts: ["a.example.com"] });
  await runtime.openTab(null);
  const before = created.length;
  const decision = runtime.current().contents.windowOpenHandler({ url: "https://a.example.com/x" });
  assert.deepEqual(decision, { action: "deny" });
  await new Promise((resolve) => setTimeout(resolve, 30));
  assert.equal(created.length, before + 1);
});

test("开发者工具按标签开合，与浏览器里的检查一致", async () => {
  const { runtime } = runtimeWithUi();
  await runtime.openTab(null);
  assert.deepEqual(runtime.toggleDevTools(), { opened: true });
  assert.deepEqual(runtime.toggleDevTools(), { opened: false });
});

test("关闭全部标签后驾驶权复位，下次打开不带着上一轮的状态", async () => {
  const { runtime } = runtimeWithUi();
  await runtime.openTab(null);
  runtime.setDriver("employee");
  runtime.close();
  assert.equal(runtime.state().driver, "ai");
  assert.equal(runtime.state().running, false);
});

/*
 * 标签归属发起它的那次对话。切到别的对话就不该看到它——那是另一件事的上下文；两个对话
 * 各自开着标签时也不能互相顶掉当前标签。
 */

test("列标签按对话过滤，别的对话的标签不出现", async () => {
  const { runtime } = runtimeWithUi();
  await runtime.openTab(null, { conversationId: "c1" });
  await runtime.openTab(null, { conversationId: "c2" });
  assert.equal(runtime.listTabs("c1").tabs.length, 1);
  assert.equal(runtime.listTabs("c2").tabs.length, 1);
  // 不给对话时看到全部：自检与排障需要看得见整体
  assert.equal(runtime.listTabs().tabs.length, 2);
});

test("两个对话各记各的当前标签，切换不互相顶掉", async () => {
  const { runtime } = runtimeWithUi();
  await runtime.openTab(null, { conversationId: "c1" });
  const first = runtime.activeIdOf("c1");
  await runtime.openTab(null, { conversationId: "c2" });
  assert.notEqual(runtime.activeIdOf("c2"), first);
  assert.equal(runtime.activeIdOf("c1"), first, "开了别的对话的标签，本对话的当前标签不该变");
});

test("关闭标签只在同一对话内找替补，不跨对话顶替", async () => {
  const { runtime } = runtimeWithUi();
  await runtime.openTab(null, { conversationId: "c1" });
  const onlyOne = runtime.activeIdOf("c1");
  await runtime.openTab(null, { conversationId: "c2" });
  runtime.closeTab(onlyOne);
  assert.equal(runtime.activeIdOf("c1"), null, "本对话已无标签，不该拿别的对话的顶上");
  assert.equal(runtime.listTabs("c2").tabs.length, 1);
});

test("AI 读指定标签时把它记为该对话的当前标签", async () => {
  const { runtime } = runtimeWithUi();
  await runtime.openTab(null, { conversationId: "c1" });
  await runtime.openTab(null, { conversationId: "c1" });
  const [first] = runtime.listTabs("c1").tabs;
  await runtime.extract(undefined, first.id, "c1");
  assert.equal(runtime.activeIdOf("c1"), first.id);
});

test("页面未呈现时截图带着原因失败，不把整轮对话卡死", async () => {
  const { runtime } = runtimeWithUi();
  await runtime.openTab(null, { conversationId: "c1" });
  // 未呈现的页面没有帧可取，取帧的请求永远不会回来
  runtime.current("c1").debug.sendCommand = (method) =>
    method === "Page.captureScreenshot" ? new Promise(() => {}) : Promise.resolve({});
  await assert.rejects(
    () => runtime.screenshot(undefined, "c1", 40),
    /未呈现在界面上/,
  );
});


test("应用采集截图覆盖完整页面，普通对话仍取当前窗口", async () => {
  const {runtime}=runtimeWithUi();
  const scope="application:com.cilai.test:run";
  await runtime.openTab(null,{conversationId:scope});
  const calls=[];
  runtime.current(scope).debug.sendCommand=async(method,params)=>{
    calls.push({method,params});
    if(method === "Page.getLayoutMetrics")return {cssContentSize:{x:0,y:0,width:1440,height:3600}};
    return {data:"image"};
  };
  assert.equal((await runtime.screenshot(undefined,scope)).data,"image");
  const shot=calls.find(c=>c.method === "Page.captureScreenshot");
  assert.equal(shot.params.captureBeyondViewport,false);
  assert.deepEqual(calls.find(c=>c.method === "Emulation.setDeviceMetricsOverride").params,
    {width:1440,height:3600,deviceScaleFactor:1,mobile:false,scale:1});
  assert.equal(calls.at(-1).method,"Emulation.clearDeviceMetricsOverride");
  assert.deepEqual(shot.params.clip,{x:0,y:0,width:1440,height:3600,scale:1});
});

test("应用完整截图尺寸缺失或超限时不返回裁剪图片", async () => {
  const {runtime}=runtimeWithUi();const scope="application:com.cilai.test:run";
  await runtime.openTab(null,{conversationId:scope});
  let captured=false;
  runtime.current(scope).debug.sendCommand=async(method)=>{
    if(method === "Page.getLayoutMetrics")return {cssContentSize:{x:0,y:0,width:1440,height:30000}};
    captured=true;return {data:"partial"};
  };
  await assert.rejects(()=>runtime.screenshot(undefined,scope),/尺寸限制/);
  assert.equal(captured,false);
  runtime.current(scope).debug.sendCommand=async()=>({});
  await assert.rejects(()=>runtime.screenshot(undefined,scope),/无法确定完整页面尺寸/);
});


test("应用截图失败或超时后恢复页面视口，拒绝同页同时截图", async () => {
  const {runtime}=runtimeWithUi();const scope="application:com.cilai.test:run";
  await runtime.openTab(null,{conversationId:scope});
  const calls=[];
  runtime.current(scope).debug.sendCommand=async(method)=>{
    calls.push(method);
    if(method === "Page.getLayoutMetrics")return {cssContentSize:{x:0,y:0,width:1440,height:3600}};
    if(method === "Page.captureScreenshot")return new Promise(()=>{});
    return {};
  };
  const capture=runtime.screenshot(undefined,scope,30);
  await assert.rejects(()=>runtime.screenshot(undefined,scope),/正在截图/);
  await assert.rejects(()=>capture,/截图超时/);
  assert.equal(calls.at(-1),"Emulation.clearDeviceMetricsOverride");
  assert.equal(runtime.current(scope).capturing,false);
  let reads=0;
  runtime.current(scope).debug.sendCommand=async(method)=>{
    calls.push(method);
    if(method === "Page.getLayoutMetrics")return {cssContentSize:{x:0,y:0,width:1440,height:++reads===1?3600:4000}};
    return {};
  };
  await assert.rejects(()=>runtime.screenshot(undefined,scope),/布局发生变化/);
  assert.equal(calls.at(-1),"Emulation.clearDeviceMetricsOverride");
});


test("完整截图等待实际浏览器尺寸，布局确认不匹配时不取图", async () => {
  const {runtime}=runtimeWithUi();const scope="application:com.cilai.test:run";
  await runtime.openTab(null,{conversationId:scope});let captured=false;
  runtime.current(scope).debug.sendCommand=async(method)=>{
    if(method === "Page.getLayoutMetrics")return {cssContentSize:{x:0,y:0,width:1440,height:3600}};
    if(method === "Page.captureScreenshot")captured=true;
    return {};
  };
  runtime.onState=(state)=>{
    for(const tab of state.tabs)if(tab.captureLayout)queueMicrotask(()=>runtime.captureLayoutReady({
      tabId:tab.id,requestId:tab.captureLayout.id,width:500,height:700,
    }));
  };
  await assert.rejects(()=>runtime.screenshot(undefined,scope),/承载尺寸未就绪/);
  assert.equal(captured,false);
  assert.equal(runtime.current(scope).captureLayout,undefined);
  assert.deepEqual(runtime.captureLayoutReady({tabId:'missing',requestId:'stale',width:1,height:1}),{accepted:false});
});
