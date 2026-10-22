const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const { createBackgroundLifecycle } = require("./background-lifecycle");

function fixture(platform, { headless = false, emptyIcon = false, menuFails = false } = {}) {
  const app = new EventEmitter();
  const nativeUpdater = new EventEmitter();
  const win = new EventEmitter();
  win.webContents = new EventEmitter();
  const state = { visible: true, minimized: false, destroyed: false, cleanups: 0, quits: 0, focuses: 0 };
  win.isDestroyed = () => state.destroyed;
  win.isMinimized = () => state.minimized;
  win.restore = () => { state.minimized = false; };
  win.show = () => { state.visible = true; };
  win.focus = () => { state.focuses++; };
  win.hide = () => { state.visible = false; };
  win.close = () => {
    const event = { prevented: false, preventDefault() { this.prevented = true; } };
    win.emit("close", event);
    if (!event.prevented) {
      if (state.vetoUnload) win.webContents.emit("will-prevent-unload");
      else { state.destroyed = true; win.emit("closed"); }
    }
    return event;
  };
  app.getName = () => "词来工作台";
  app.quit = () => {
    state.quits++;
    app.emit("before-quit");
    win.close();
    if (state.destroyed) app.emit("will-quit");
  };
  const trays = [];
  class Tray extends EventEmitter {
    constructor(icon) { super(); this.icon = icon; trays.push(this); }
    setToolTip(value) { this.tooltip = value; }
    setContextMenu(menu) { this.menu = menu; }
    destroy() { this.destroyed = true; }
  }
  const lifecycle = createBackgroundLifecycle({
    app, Tray, nativeUpdater, platform, headless, getWindow: () => win,
    Menu: { buildFromTemplate: (items) => { if (menuFails) throw new Error("menu failed"); return items; } },
    nativeImage: { createFromPath: (file) => ({ file, isEmpty: () => emptyIcon,
      setTemplateImage(value) { this.template = value; } }) },
    cleanup: () => { state.cleanups++; },
  });
  lifecycle.bindWindow(win);
  return { app, nativeUpdater, win, state, trays, lifecycle };
}

for (const platform of ["darwin", "win32"]) {
  test(`${platform} 关闭按钮只隐藏原窗口，后台服务与页面保持存活`, () => {
    const f = fixture(platform);
    f.lifecycle.createTray();
    for (let i = 0; i < 3; i++) {
      assert.equal(f.win.close().prevented, true);
      assert.equal(f.state.visible, false);
      assert.equal(f.state.destroyed, false);
      assert.equal(f.state.cleanups, 0);
      assert.equal(f.state.quits, 0);
      f.trays[0].menu[0].click();
      assert.equal(f.state.visible, true);
    }
    assert.equal(f.trays.length, 1);
  });

  test(`${platform} 恢复入口先还原最小化窗口，再显示并聚焦`, () => {
    const f = fixture(platform);
    f.state.minimized = true;
    f.state.visible = false;
    f.lifecycle.showWindow();
    assert.equal(f.state.minimized, false);
    assert.equal(f.state.visible, true);
    assert.equal(f.state.focuses, 1);
  });

  test(`${platform} 托盘退出真正结束进程，只清理一次`, () => {
    const f = fixture(platform);
    f.lifecycle.createTray();
    f.win.close();
    f.trays[0].menu[2].click();
    assert.equal(f.state.quits, 1);
    assert.equal(f.state.destroyed, true);
    assert.equal(f.state.cleanups, 1);
    assert.equal(f.trays[0].destroyed, true);
    f.app.emit("will-quit");
    f.lifecycle.showWindow();
    assert.equal(f.state.cleanups, 1);
    assert.equal(f.state.visible, false);
  });

  test(`${platform} 系统菜单退出与原生更新先关闭窗口的顺序均不被隐藏逻辑阻挡`, () => {
    const regular = fixture(platform);
    regular.app.quit();
    assert.equal(regular.state.destroyed, true);
    assert.equal(regular.state.cleanups, 1);
    const updating = fixture(platform);
    updating.nativeUpdater.emit("before-quit-for-update");
    assert.equal(updating.win.close().prevented, false);
    assert.equal(updating.state.cleanups, 0);
    updating.app.emit("before-quit");
    updating.app.emit("will-quit");
    assert.equal(updating.state.cleanups, 1);
  });

  test(`${platform} 草稿阻止退出时不停止服务，重新关闭仍转后台且可以再次主动退出`, () => {
    const f = fixture(platform);
    f.lifecycle.createTray();
    f.win.close();
    f.state.vetoUnload = true;
    f.app.quit();
    assert.equal(f.state.destroyed, false);
    assert.equal(f.state.cleanups, 0);
    assert.equal(f.trays[0].destroyed, undefined);
    assert.equal(f.state.visible, true);
    f.state.vetoUnload = false;
    assert.equal(f.win.close().prevented, true);
    f.app.quit();
    assert.equal(f.state.cleanups, 1);
  });
}

test("Windows 托盘单击及双击均恢复窗口，系统会话结束只清理一次", () => {
  const f = fixture("win32");
  f.lifecycle.createTray();
  for (const event of ["click", "double-click"]) {
    f.win.close();
    f.trays[0].emit(event);
    assert.equal(f.state.visible, true);
  }
  f.win.emit("query-session-end", { preventDefault() { assert.fail("不得阻止系统关机"); } });
  assert.equal(f.state.cleanups, 0);
  f.win.emit("session-end");
  assert.equal(f.win.close().prevented, false);
  f.app.emit("will-quit");
  assert.equal(f.state.cleanups, 1);
});

test("macOS Dock 激活恢复原窗口，菜单栏使用模板图标", () => {
  const f = fixture("darwin");
  f.lifecycle.createTray();
  f.win.close();
  f.app.emit("activate");
  assert.equal(f.state.visible, true);
  assert.equal(f.trays[0].icon.template, true);
});

test("自检模式不创建托盘、不隐藏或唤起窗口", () => {
  const f = fixture("darwin", { headless: true });
  f.lifecycle.createTray();
  f.app.emit("activate");
  assert.equal(f.trays.length, 0);
  assert.equal(f.state.focuses, 0);
  assert.equal(f.win.close().prevented, false);
  f.app.emit("window-all-closed");
  assert.equal(f.state.quits, 1);
});

test("自检成功和失败直接退出前完成一次清理，保留真实退出码", () => {
  for (const code of [0, 1]) {
    const f = fixture("darwin", { headless: true });
    let beforeQuitEvents = 0;
    f.app.on("before-quit", () => { beforeQuitEvents++; });
    f.app.exit = actualCode => {
      assert.equal(actualCode, code);
      assert.equal(f.state.cleanups, 1);
    };
    f.lifecycle.exit(code);
    assert.equal(beforeQuitEvents, 0);
    f.app.emit("will-quit");
    assert.equal(f.state.cleanups, 1);
    assert.equal(f.win.close().prevented, false);
  }
});

test("托盘初始化失败明确报错并释放未完成的托盘", () => {
  assert.throws(() => fixture("win32", { emptyIcon: true }).lifecycle.createTray(), /图标无法读取/);
  const f = fixture("win32", { menuFails: true });
  assert.throws(() => f.lifecycle.createTray(), /menu failed/);
  assert.equal(f.trays[0].destroyed, true);
  assert.throws(() => fixture("linux").lifecycle.createTray(), /暂不支持/);
});

test("两端图标包含普通和高分辨率资源，随 shell 目录打包", () => {
  for (const name of ["tray", "trayTemplate"]) {
    for (const [suffix, size] of [["", 18], ["@2x", 36]]) {
      const bytes = readFileSync(path.join(__dirname, "assets", `${name}${suffix}.png`));
      assert.equal(bytes.subarray(1, 4).toString(), "PNG");
      assert.equal(bytes.readUInt32BE(16), size);
      assert.equal(bytes.readUInt32BE(20), size);
    }
  }
});

test("主入口接入关闭、恢复、原生更新与后台渲染，不保留窗口关闭即退出路径", () => {
  const main = readFileSync(path.join(__dirname, "main.js"), "utf8");
  assert.match(main, /backgroundLifecycle\.bindWindow\(win\)/);
  assert.match(main, /backgroundLifecycle\.createTray\(\)/);
  assert.match(main, /second-instance[\s\S]*?backgroundLifecycle\.showWindow\(\)/);
  assert.match(main, /autoUpdater: nativeUpdater/);
  assert.match(main, /backgroundThrottling: false/);
  assert.match(main, /guardBrowserWebviews\(win,browser\)/);
  const browserGuard = readFileSync(path.join(__dirname, "browser-webviews.js"), "utf8");
  assert.match(browserGuard, /preferences\.backgroundThrottling = false/);
  assert.doesNotMatch(main, /app\.on\("window-all-closed", \(\) => app\.quit\(\)\)/);
  assert.doesNotMatch(main, /app\.on\("before-quit"/);
});
