const { readFileSync, writeFileSync } = require("node:fs");
const path = require("node:path");

const DEFAULT_WINDOW_STATE = Object.freeze({ width: 1180, height: 760, maximized: false });
const MIN_WINDOW_SIZE = Object.freeze({ width: 900, height: 600 });

function positiveInteger(value) {
  return Number.isInteger(value) && value > 0 ? value : null;
}

function normalizeWindowState(value, workAreaSize = {}) {
  const availableWidth = positiveInteger(workAreaSize.width);
  const availableHeight = positiveInteger(workAreaSize.height);
  const maximumWidth = availableWidth ? Math.max(MIN_WINDOW_SIZE.width, availableWidth) : Number.MAX_SAFE_INTEGER;
  const maximumHeight = availableHeight ? Math.max(MIN_WINDOW_SIZE.height, availableHeight) : Number.MAX_SAFE_INTEGER;
  const storedWidth = positiveInteger(value?.width);
  const storedHeight = positiveInteger(value?.height);
  return {
    width: Math.min(Math.max(storedWidth ?? DEFAULT_WINDOW_STATE.width, MIN_WINDOW_SIZE.width), maximumWidth),
    height: Math.min(Math.max(storedHeight ?? DEFAULT_WINDOW_STATE.height, MIN_WINDOW_SIZE.height), maximumHeight),
    maximized: value?.maximized === true,
  };
}

/**
 * 把窗口拉宽到够放下某个内容宽度，返回该用的新外框；不需要动就返回 null。
 *
 * 只放大不缩小：缩小会把员工自己调好的窗口改掉，而拉宽是为了不让对话区被切——后者是
 * 员工没法自己发现的故障，前者只是他没要求过的改动。
 *
 * 拉宽后右边会越出屏幕工作区时把窗口整体往左挪，挪到工作区左边为止；屏幕本身放不下就
 * 拉到工作区那么宽，剩下的差额交给让宽逻辑，不把窗口推到看不见的地方去。
 */
function widenWindowBounds(input) {
  const bounds = input?.bounds;
  const workArea = input?.workArea;
  const contentWidth = positiveInteger(input?.contentWidth);
  const wanted = Number.isFinite(input?.wantedContentWidth) ? Math.ceil(input.wantedContentWidth) : null;
  if (!bounds || !workArea || contentWidth === null || wanted === null) return null;
  if (wanted <= contentWidth) return null;
  // 外框比内容多出来的那一圈。macOS 覆盖式标题栏下通常是零，Windows 上不是
  const frame = Math.max(0, bounds.width - contentWidth);
  const outerWidth = Math.min(wanted + frame, Math.max(MIN_WINDOW_SIZE.width, workArea.width));
  if (outerWidth <= bounds.width) return null;
  const x = Math.max(workArea.x, Math.min(bounds.x, workArea.x + workArea.width - outerWidth));
  return { x, y: bounds.y, width: outerWidth, height: bounds.height, contentWidth: outerWidth - frame };
}

function windowStateFile(userDataPath) {
  return path.join(userDataPath, "window-state.json");
}

function loadWindowState(filePath, workAreaSize) {
  try {
    return normalizeWindowState(JSON.parse(readFileSync(filePath, "utf8")), workAreaSize);
  } catch (error) {
    if (error?.code !== "ENOENT") console.warn(`[window] 上次窗口大小读取失败：${error.message}`);
    return normalizeWindowState(DEFAULT_WINDOW_STATE, workAreaSize);
  }
}

function saveWindowState(filePath, state) {
  writeFileSync(filePath, `${JSON.stringify(normalizeWindowState(state))}\n`, {
    encoding: "utf8",
    mode: 0o600,
  });
}

function currentWindowState(win) {
  const bounds = win.getNormalBounds();
  return normalizeWindowState({
    width: bounds.width,
    height: bounds.height,
    maximized: win.isMaximized(),
  });
}

module.exports = {
  DEFAULT_WINDOW_STATE,
  MIN_WINDOW_SIZE,
  currentWindowState,
  loadWindowState,
  normalizeWindowState,
  saveWindowState,
  widenWindowBounds,
  windowStateFile,
};
