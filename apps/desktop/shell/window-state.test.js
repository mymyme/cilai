const assert = require("node:assert/strict");
const { mkdtempSync, readFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const path = require("node:path");
const test = require("node:test");
const {
  DEFAULT_WINDOW_STATE,
  currentWindowState,
  loadWindowState,
  normalizeWindowState,
  saveWindowState,
  widenWindowBounds,
  windowStateFile,
} = require("./window-state");

test("首次启动使用默认窗口大小", () => {
  const root = mkdtempSync(path.join(tmpdir(), "cilai-window-state-"));
  assert.deepEqual(loadWindowState(windowStateFile(root), { width: 1440, height: 900 }), DEFAULT_WINDOW_STATE);
});

test("关闭前保存的窗口大小可在下次启动恢复", () => {
  const root = mkdtempSync(path.join(tmpdir(), "cilai-window-state-"));
  const filePath = windowStateFile(root);
  saveWindowState(filePath, { width: 1320, height: 840, maximized: false });
  assert.deepEqual(loadWindowState(filePath, { width: 1728, height: 1117 }), {
    width: 1320,
    height: 840,
    maximized: false,
  });
  assert.equal(JSON.parse(readFileSync(filePath, "utf8")).width, 1320);
});

test("窗口大小不超出当前屏幕可用区域", () => {
  assert.deepEqual(normalizeWindowState({ width: 2600, height: 1600, maximized: true }, { width: 1440, height: 877 }), {
    width: 1440,
    height: 877,
    maximized: true,
  });
});

test("最大化关闭时保留恢复后的普通窗口大小", () => {
  const state = currentWindowState({
    getNormalBounds: () => ({ x: 20, y: 30, width: 1280, height: 800 }),
    isMaximized: () => true,
  });
  assert.deepEqual(state, { width: 1280, height: 800, maximized: true });
});

/*
 * 对话页展开浏览器时的自动拉宽。三条红线：不缩小、不越出屏幕、不动最大化窗口（最后一条
 * 在 bridge 里判定，这里只管几何）。
 */
const screenWorkArea = { x: 0, y: 25, width: 1800, height: 1100 };

test("窗口不够宽时拉到要的宽度", () => {
  const next = widenWindowBounds({
    bounds: { x: 100, y: 50, width: 1436, height: 800 },
    contentWidth: 1436,
    wantedContentWidth: 1527,
    workArea: screenWorkArea,
  });
  assert.deepEqual(next, { x: 100, y: 50, width: 1527, height: 800, contentWidth: 1527 });
});

test("已经够宽就不动窗口", () => {
  assert.equal(widenWindowBounds({
    bounds: { x: 0, y: 50, width: 1600, height: 800 },
    contentWidth: 1600,
    wantedContentWidth: 1527,
    workArea: screenWorkArea,
  }), null);
});

test("永远不缩小窗口", () => {
  assert.equal(widenWindowBounds({
    bounds: { x: 0, y: 50, width: 1600, height: 800 },
    contentWidth: 1600,
    wantedContentWidth: 1000,
    workArea: screenWorkArea,
  }), null);
});

test("拉宽后右边越界就把窗口往左挪，不推到屏幕外面", () => {
  const next = widenWindowBounds({
    bounds: { x: 600, y: 50, width: 1436, height: 800 },
    contentWidth: 1436,
    wantedContentWidth: 1527,
    workArea: screenWorkArea,
  });
  assert.deepEqual(next, { x: 273, y: 50, width: 1527, height: 800, contentWidth: 1527 });
  assert.ok(next.x + next.width <= screenWorkArea.x + screenWorkArea.width);
});

test("屏幕本身放不下就拉到工作区那么宽，剩下的交给让宽逻辑", () => {
  const next = widenWindowBounds({
    bounds: { x: 0, y: 50, width: 1200, height: 800 },
    contentWidth: 1200,
    wantedContentWidth: 1527,
    workArea: { x: 0, y: 25, width: 1280, height: 1100 },
  });
  assert.deepEqual(next, { x: 0, y: 50, width: 1280, height: 800, contentWidth: 1280 });
});

test("Windows 那种有外框的窗口按内容宽度算，不把边框算进去", () => {
  const next = widenWindowBounds({
    bounds: { x: 0, y: 50, width: 1452, height: 800 },
    contentWidth: 1436,
    wantedContentWidth: 1527,
    workArea: screenWorkArea,
  });
  assert.equal(next.contentWidth, 1527);
  assert.equal(next.width, 1543);
});

test("拿不到宽度或工作区时什么都不做", () => {
  assert.equal(widenWindowBounds({ bounds: null, contentWidth: 1200, wantedContentWidth: 1500, workArea: screenWorkArea }), null);
  assert.equal(widenWindowBounds({ bounds: { x: 0, y: 0, width: 1200, height: 800 }, contentWidth: 0, wantedContentWidth: 1500, workArea: screenWorkArea }), null);
  assert.equal(widenWindowBounds({ bounds: { x: 0, y: 0, width: 1200, height: 800 }, contentWidth: 1200, wantedContentWidth: Number.NaN, workArea: screenWorkArea }), null);
  assert.equal(widenWindowBounds({ bounds: { x: 0, y: 0, width: 1200, height: 800 }, contentWidth: 1200, wantedContentWidth: 1500, workArea: null }), null);
});
