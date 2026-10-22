const path = require("node:path");

function createBackgroundLifecycle({
  app, Tray, Menu, nativeImage, nativeUpdater, getWindow, cleanup,
  platform = process.platform, headless = false,
}) {
  let quitting = false;
  let cleaned = false;
  let tray = null;

  function showWindow() {
    if (quitting) return;
    const win = getWindow();
    if (!win || win.isDestroyed()) return;
    if (win.isMinimized()) win.restore();
    win.show();
    win.focus();
  }

  function finish() {
    if (cleaned) return;
    cleaned = true;
    quitting = true;
    tray?.destroy();
    tray = null;
    cleanup();
  }

  app.on("before-quit", () => { quitting = true; });
  app.on("will-quit", finish);
  nativeUpdater.on("before-quit-for-update", () => { quitting = true; });
  if (!headless) app.on("activate", showWindow);
  app.on("window-all-closed", () => {
    if (headless) app.quit();
  });

  return {
    showWindow,
    exit(code) {
      finish();
      app.exit(code);
    },
    createTray() {
      if (headless || tray) return;
      const icons = { darwin: "trayTemplate.png", win32: "tray.png" };
      const file = icons[platform];
      if (!file) throw new Error(`后台运行暂不支持当前平台：${platform}`);
      const icon = nativeImage.createFromPath(path.join(__dirname, "assets", file));
      if (icon.isEmpty()) throw new Error("后台运行图标无法读取");
      if (platform === "darwin") icon.setTemplateImage(true);
      const created = new Tray(icon);
      try {
        created.setToolTip(`${app.getName()} · 后台运行`);
        created.setContextMenu(Menu.buildFromTemplate([
          { label: "打开", click: showWindow },
          { type: "separator" },
          { label: "退出", click: () => app.quit() },
        ]));
        if (platform === "win32") {
          created.on("click", showWindow);
          created.on("double-click", showWindow);
        }
        tray = created;
      } catch (error) {
        created.destroy();
        throw error;
      }
    },
    bindWindow(win) {
      win.on("close", (event) => {
        if (headless || quitting) return;
        event.preventDefault();
        win.hide();
      });
      win.webContents.on("will-prevent-unload", () => {
        quitting = false;
        showWindow();
      });
      if (platform === "win32") win.on("session-end", finish);
    },
  };
}

module.exports = { createBackgroundLifecycle };
