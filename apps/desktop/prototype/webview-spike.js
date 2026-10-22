/*
 * 受控应用容器改用 <webview> 承载的限定验证。
 *
 * 目的只有一个：判断离屏承载是否可以被替换。四项都是离屏方案付出过代价、或当年否掉
 * 原生子视图时卡住的地方——逐请求管控、中文输入法、宿主浮层覆盖、分区登录态。
 *
 * 与原生子视图的区别是合成方式：那是独立原生层，宿主 DOM 盖不住；<webview> 合成进
 * 宿主页面的图层树，因此覆盖是否成立必须实测，不能按"都是 webview"类推。
 */
const { app, BrowserWindow, session, webContents } = require("electron");
const path = require("node:path");

const VERIFY = process.argv.includes("--verify-webview");
const TARGET = "https://archive.cilai.com.cn/";
const PARTITION = "persist:spike-archive";
const STAGE = { left: 16, top: 56, width: 1132, height: 680 };

const checks = [];
const record = (name, ok, detail = "") => checks.push({ name, ok, detail });

function createHost() {
  return new BrowserWindow({
    width: 1180, height: 800, show: !VERIFY,
    backgroundColor: "#0b0f14",
    webPreferences: { webviewTag: true, contextIsolation: true, nodeIntegration: false },
  });
}

/** 等待条件成立，避免用固定睡眠掩盖时序问题。 */
async function until(predicate, timeoutMs = 25000, stepMs = 250) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await predicate()) return true;
    await new Promise((resolve) => setTimeout(resolve, stepMs));
  }
  return false;
}

/**
 * 逐请求管控。方案 §2.2 把它列为改用 CDP 的实质增强，因此必须确认换承载后不丢。
 *
 * 拦截在导航之前启用：要验的不只是"能看到请求"，还包括页面在拦截器下能否正常加载。
 */
function interceptRequests(contents, seen) {
  const debug = contents.debugger;
  if (!debug.isAttached()) debug.attach("1.3");
  debug.on("message", (_event, method, params) => {
    if (method !== "Fetch.requestPaused") return;
    const url = params.request.url;
    seen.push(url);
    if (url.includes("blocked-probe")) {
      void debug.sendCommand("Fetch.failRequest",
        { requestId: params.requestId, errorReason: "BlockedByClient" }).catch(() => {});
      return;
    }
    void debug.sendCommand("Fetch.continueRequest", { requestId: params.requestId }).catch(() => {});
  });
  return debug.sendCommand("Fetch.enable", { patterns: [{ urlPattern: "*" }] });
}

/** 浮层覆盖：按截图取色判定，不看"元素在不在 DOM 里"——在 DOM 里不等于画得出来。 */
async function checkOverlay(host) {
  const image = await host.webContents.capturePage();
  const shotIndex = process.argv.indexOf("--shot");
  if (shotIndex > 0 && process.argv[shotIndex + 1]) {
    require("node:fs").writeFileSync(process.argv[shotIndex + 1], image.toPNG());
  }
  const size = image.getSize();
  const bitmap = image.toBitmap();                       // BGRA
  const scale = size.width / host.getContentSize()[0];
  const at = (x, y) => {
    const index = (Math.round(y * scale) * size.width + Math.round(x * scale)) * 4;
    return { r: bitmap[index + 2], g: bitmap[index + 1], b: bitmap[index] };
  };
  const center = at(STAGE.left + 240, STAGE.top + 140);   // 浮层正中
  const outside = at(STAGE.left + 800, STAGE.top + 500);  // 浮层之外，应是业务页面
  const corner = at(STAGE.left + 82, STAGE.top + 62);     // 圆角外侧，应被裁掉
  const isRed = center.r > 180 && center.g < 80 && center.b < 80;
  const cornerClipped = !(corner.r > 180 && corner.g < 80 && corner.b < 80);
  const appVisible = outside.r + outside.g + outside.b > 0 && !(outside.r > 180 && outside.g < 80);
  record("宿主浮层盖在应用之上", isRed, `浮层中心 rgb(${center.r},${center.g},${center.b})`);
  record("浮层圆角按宿主样式裁剪", cornerClipped, `圆角外侧 rgb(${corner.r},${corner.g},${corner.b})`);
  record("应用画面正常呈现", appVisible, `浮层外 rgb(${outside.r},${outside.g},${outside.b})`);
}

/** 分区登录态：跨容器销毁重建仍在，才说明存储按分区留存而不是随视图消失。 */
async function checkPartition(host) {
  const store = session.fromPartition(PARTITION);
  await store.cookies.set({ url: TARGET, name: "spike_login", value: "kept", expirationDate: Date.now() / 1000 + 3600 });
  await host.webContents.executeJavaScript(`(() => {
    const wv = document.getElementById("wv");
    wv.remove();
    const next = document.createElement("webview");
    next.id = "wv"; next.partition = ${JSON.stringify(PARTITION)};
    next.style.cssText = "position:absolute;inset:0;width:100%;height:100%";
    window.ready = false;
    next.addEventListener("did-finish-load", () => { window.ready = true; });
    document.getElementById("stage").prepend(next);
    next.src = ${JSON.stringify(TARGET)};
  })()`);
  const reloaded = await until(() => host.webContents.executeJavaScript("window.ready"));
  const kept = (await store.cookies.get({ name: "spike_login" })).length === 1;
  record("容器销毁重建后重新加载", reloaded);
  record("登录态按分区留存", kept);
  const shared = (await session.defaultSession.cookies.get({ name: "spike_login" })).length === 0;
  record("分区与默认会话相互隔离", shared);
}

app.whenReady().then(async () => {
  const host = createHost();
  const seen = [];
  let target = null;

  host.webContents.on("did-attach-webview", (_event, contents) => { target = contents; });
  await host.loadFile(path.join(__dirname, "webview-host.html"));
  await until(() => Boolean(host.webContents));

  // webview 元素先不带地址，等拦截就位再导航，否则首屏请求看不到
  await host.webContents.executeJavaScript(`document.getElementById("wv").src = ${JSON.stringify(TARGET)}`);
  await until(() => Boolean(target), 10000);
  if (!target) {
    console.log("失败  未取得 webview 的内容对象");
    app.quit();
    return;
  }

  record("承载不是离屏渲染（输入法由系统原生处理）", target.isOffscreen() === false,
    `isOffscreen=${target.isOffscreen()}`);

  await interceptRequests(target, seen).catch((error) => record("启用逐请求拦截", false, error.message));
  await host.webContents.executeJavaScript(`(() => {
    const wv = document.getElementById("wv"); window.ready = false;
    wv.addEventListener("did-finish-load", () => { window.ready = true; }); wv.reload();
  })()`);
  const loaded = await until(() => host.webContents.executeJavaScript("window.ready"));
  record("拦截器下页面仍能完整加载", loaded, `途经请求 ${seen.length} 条`);
  record("逐请求可见（拿到具体地址）", seen.length > 0, seen[0] ? seen[0].slice(0, 60) : "");
  if (process.argv.includes("--dump-requests")) for (const url of seen) console.log("    途经 " + url.slice(0, 110));

  const probe = await target.executeJavaScript(`(async () => {
    const one = await fetch(${JSON.stringify(TARGET)} + "?probe=allow").then(() => "放行", () => "被拦");
    const two = await fetch(${JSON.stringify(TARGET)} + "?probe=blocked-probe").then(() => "放行", () => "被拦");
    return JSON.stringify({ one, two });
  })()`).then(JSON.parse, () => ({ one: "异常", two: "异常" }));
  record("允许的请求放行", probe.one === "放行", probe.one);
  record("指定的请求被拦下", probe.two === "被拦", probe.two);

  await checkOverlay(host);
  await checkPartition(host);

  const failed = checks.filter((c) => !c.ok);
  for (const c of checks) console.log(`${c.ok ? "通过" : "失败"}  ${c.name}${c.detail ? "  — " + c.detail : ""}`);
  console.log(`\n结论：${checks.length - failed.length}/${checks.length} 项通过`);
  if (!VERIFY) {
    // 业务应用的登录态来自工作台注入的桥接，独立原型没有那条链路，进不去内页；
    // 待人工确认的是原生控件与输入法，换一个自检页即可，不必为此重建整条鉴权。
    await host.webContents.executeJavaScript(`document.getElementById("wv").src = "webview-controls.html"`);
    console.log("\n窗口已打开，已载入控件自检页。请逐项确认：");
    console.log("  1. 下拉列表、日期、日期时间、颜色、建议列表——弹层能否正常展开并选中");
    console.log("  2. 右键区域——上下文菜单能否弹出");
    console.log("  3. 中文输入框——候选窗位置、上屏内容是否正常");
    console.log("  4. 点「切换宿主浮层」——红色浮层应盖在内容之上");
    console.log("  5. 点「加载环保档案」——可看真实应用（停在未建立会话属正常，见说明）");
    return;
  }
  app.quit();
});
