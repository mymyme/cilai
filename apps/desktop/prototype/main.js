const { app, BrowserWindow, ipcMain } = require("electron");
const path = require("node:path");

const VERIFY = process.argv.includes("--verify-ime");
const PERF = process.argv.includes("--verify-perf");
if (VERIFY || PERF) {
  const { mkdtempSync } = require("node:fs");
  const { tmpdir } = require("node:os");
  app.setPath("userData", mkdtempSync(path.join(tmpdir(), "cilai-electron-verification-")));
}
const SIZE = { width: 800, height: 400 };

/** 离屏页面承载被测内容，宿主窗口只负责呈现它的画面并转发输入。 */
let offscreen = null;
let host = null;

function createOffscreen() {
  const win = new BrowserWindow({
    ...SIZE,
    show: false,
    webPreferences: { offscreen: true },
  });
  win.webContents.setFrameRate(60);
  return win;
}

function createHost() {
  const win = new BrowserWindow({
    width: 860,
    height: 520,
    show: !VERIFY,
    webPreferences: { preload: path.join(__dirname, "preload.js") },
  });
  return win;
}

/**
 * 输入法组合经调试协议送入离屏页面。
 *
 * 宿主窗口那个透明输入框只负责让系统输入法工作起来并产出组合串；真正要把字符落进
 * 被测页面，走的是 Input.imeSetComposition 与 Input.insertText，而不是合成按键——
 * 合成按键无法表达组合中的中间态，也拼不出候选词选定后的结果。
 */
function sendComposition(payload) {
  const debug = offscreen.webContents.debugger;
  if (payload.phase === "update") {
    return debug.sendCommand("Input.imeSetComposition", {
      text: payload.text,
      selectionStart: payload.text.length,
      selectionEnd: payload.text.length,
    });
  }
  return debug
    .sendCommand("Input.imeSetComposition", { text: "", selectionStart: 0, selectionEnd: 0 })
    .catch(() => undefined)
    .then(() => debug.sendCommand("Input.insertText", { text: payload.text }));
}

function sendPointer(payload) {
  const type = payload.type === "down" ? "mousePressed" : "mouseReleased";
  return offscreen.webContents.debugger.sendCommand("Input.dispatchMouseEvent", {
    type,
    x: payload.x,
    y: payload.y,
    button: "left",
    clickCount: 1,
  });
}

async function readScrollTop() {
  const { result } = await offscreen.webContents.debugger.sendCommand("Runtime.evaluate", {
    expression: "Math.round(window.scrollY)",
    returnByValue: true,
  });
  return result.value;
}

async function readFieldValue() {
  const { result } = await offscreen.webContents.debugger.sendCommand("Runtime.evaluate", {
    expression: "window.__fieldValue()",
    returnByValue: true,
  });
  return result.value;
}

/** 自动化验证：组合事件能否完整落进离屏页面。 */
async function runVerification() {
  const checks = [];
  const record = (name, ok, detail) => {
    checks.push({ name, ok, detail });
    console.log(`${ok ? "通过" : "失败"}  ${name}${detail ? "  " + detail : ""}`);
  };

  // 画面回传
  let framesSeen = 0;
  offscreen.webContents.on("paint", () => { framesSeen += 1; });
  await new Promise((resolve) => setTimeout(resolve, 1200));
  record("离屏页面回传帧", framesSeen > 0, `帧数 ${framesSeen}`);

  // 指针转发并聚焦
  await sendPointer({ type: "down", x: 120, y: 90 });
  await sendPointer({ type: "up", x: 120, y: 90 });

  // 英文直入
  await sendComposition({ phase: "commit", text: "abc" });
  await new Promise((resolve) => setTimeout(resolve, 150));
  let value = await readFieldValue();
  record("英文提交", value === "abc", `字段值 ${JSON.stringify(value)}`);

  // 组合中间态：应当可见但未提交
  await sendComposition({ phase: "update", text: "zhong" });
  await new Promise((resolve) => setTimeout(resolve, 150));
  const composing = await readFieldValue();
  record("组合中间态进入页面", composing.includes("zhong"), `字段值 ${JSON.stringify(composing)}`);

  // 候选选定后提交中文
  await sendComposition({ phase: "commit", text: "中文" });
  await new Promise((resolve) => setTimeout(resolve, 200));
  value = await readFieldValue();
  record("中文提交", value === "abc中文", `字段值 ${JSON.stringify(value)}`);

  // 连续中文
  await sendComposition({ phase: "update", text: "shuru" });
  await sendComposition({ phase: "commit", text: "输入" });
  await new Promise((resolve) => setTimeout(resolve, 200));
  value = await readFieldValue();
  record("连续中文提交", value === "abc中文输入", `字段值 ${JSON.stringify(value)}`);

  const failed = checks.filter((check) => !check.ok);
  console.log(`\n结论：${checks.length - failed.length}/${checks.length} 项通过`);
  if (failed.length > 0) {
    console.log("未通过项：" + failed.map((check) => check.name).join("、"));
  }
  app.exit(failed.length === 0 ? 0 : 1);
}


/** 帧流性能实测：滚动一段长列表，统计帧率与单帧开销。 */
async function runPerformance() {
  const debug = offscreen.webContents.debugger;
  const samples = { frames: 0, dataUrlBytes: 0, dataUrlMs: 0, bitmapBytes: 0, bitmapMs: 0 };

  offscreen.webContents.on("paint", (_event, dirty, image) => {
    samples.frames += 1;
    // 两种取帧方式各测一次开销：编码为数据地址，或直接取原始位图
    let start = process.hrtime.bigint();
    const url = image.toDataURL();
    samples.dataUrlMs += Number(process.hrtime.bigint() - start) / 1e6;
    samples.dataUrlBytes += url.length;
    start = process.hrtime.bigint();
    const bitmap = image.toBitmap();
    samples.bitmapMs += Number(process.hrtime.bigint() - start) / 1e6;
    samples.bitmapBytes += bitmap.length;
    samples.lastDirty = dirty;
  });

  // 静置：无变化时不应产生帧
  await new Promise((resolve) => setTimeout(resolve, 1500));
  const idleFrames = samples.frames;
  console.log(`静置 1.5 秒帧数：${idleFrames}`);

  // 先单独确认滚轮事件能否经调试协议生效：这关系到输入转发的覆盖面
  const scrollBefore = await readScrollTop();
  await debug.sendCommand("Input.dispatchMouseEvent", {
    type: "mouseWheel", x: 400, y: 200, deltaX: 0, deltaY: -240,
  });
  await new Promise((resolve) => setTimeout(resolve, 300));
  const scrollAfter = await readScrollTop();
  const wheelWorks = scrollAfter !== scrollBefore;
  console.log(`滚轮事件 Input.dispatchMouseEvent 生效：${wheelWorks ? "是" : "否"}（${scrollBefore} → ${scrollAfter}）`);

  // 备选一 Input.synthesizeScrollGesture 在离屏模式下会使进程段错误崩溃，已排除，不要重新启用。

  // 备选二：主进程的输入事件接口
  await debug.sendCommand("Runtime.evaluate", { expression: "window.scrollTo(0, 0)" });
  before = await readScrollTop();
  offscreen.webContents.sendInputEvent({
    type: "mouseWheel", x: 400, y: 200, deltaX: 0, deltaY: -400, canScroll: true,
  });
  await new Promise((resolve) => setTimeout(resolve, 400));
  console.log(`滚轮事件 webContents.sendInputEvent 生效：${(await readScrollTop()) !== before ? "是" : "否"}`);
  await debug.sendCommand("Runtime.evaluate", { expression: "window.scrollTo(0, 0)" });

  // 连续滚动 3 秒，制造持续重绘
  samples.frames = 0;
  samples.dataUrlBytes = 0; samples.dataUrlMs = 0;
  samples.bitmapBytes = 0; samples.bitmapMs = 0;
  const started = Date.now();
  let offset = 0;
  while (Date.now() - started < 3000) {
    offset += 40;
    if (wheelWorks) {
      await debug.sendCommand("Input.dispatchMouseEvent", {
        type: "mouseWheel", x: 400, y: 200, deltaX: 0, deltaY: -40,
      });
    } else {
      // 滚轮不生效时改由脚本滚动，性能测的是重绘本身，与触发方式无关
      await debug.sendCommand("Runtime.evaluate", { expression: `window.scrollTo(0, ${offset})` });
    }
    await new Promise((resolve) => setTimeout(resolve, 16));
  }
  await new Promise((resolve) => setTimeout(resolve, 300));

  const seconds = (Date.now() - started) / 1000;
  const fps = samples.frames / seconds;
  const perFrameKb = samples.frames ? samples.dataUrlBytes / samples.frames / 1024 : 0;
  const perFrameRawKb = samples.frames ? samples.bitmapBytes / samples.frames / 1024 : 0;
  console.log(`滚动 ${seconds.toFixed(1)} 秒：帧数 ${samples.frames}，帧率 ${fps.toFixed(1)}/秒`);
  console.log(`单帧数据地址 ${perFrameKb.toFixed(0)} KB，编码耗时 ${(samples.dataUrlMs / Math.max(samples.frames,1)).toFixed(2)} 毫秒`);
  console.log(`单帧原始位图 ${perFrameRawKb.toFixed(0)} KB，取用耗时 ${(samples.bitmapMs / Math.max(samples.frames,1)).toFixed(2)} 毫秒`);
  console.log(`按数据地址方式估算带宽 ${(perFrameKb * fps / 1024).toFixed(1)} MB/秒`);
  console.log(`\n静置无帧：${idleFrames === 0 ? "是" : "否（" + idleFrames + " 帧）"}`);
  app.exit(0);
}

app.whenReady().then(async () => {
  offscreen = createOffscreen();
  host = createHost();

  // 先把加载的 promise 拿到手再等待：loadFile 返回的 promise 不会像事件那样错过。
  const offscreenLoaded = offscreen.loadFile(path.join(__dirname, PERF ? "perf-target.html" : "target.html"));
  const hostLoaded = host.loadFile(path.join(__dirname, "host.html"));

  offscreen.webContents.debugger.attach("1.3");
  await offscreen.webContents.debugger.sendCommand("Runtime.enable");

  offscreen.webContents.on("paint", (_event, _dirty, image) => {
    if (VERIFY || PERF || host.isDestroyed()) return;
    host.webContents.send("frame", image.toDataURL());
  });

  ipcMain.on("pointer", (_event, payload) => void sendPointer(payload));
  ipcMain.on("composition", (_event, payload) => void sendComposition(payload));

  await Promise.all([offscreenLoaded, hostLoaded]);

  if (VERIFY) {
    await runVerification().catch((error) => {
      console.error("验证异常：", error);
      app.exit(2);
    });
  } else if (PERF) {
    await runPerformance().catch((error) => {
      console.error("性能实测异常：", error);
      app.exit(2);
    });
  }
});

app.on("window-all-closed", () => app.quit());
