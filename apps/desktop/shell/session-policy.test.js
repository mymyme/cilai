const assert = require("node:assert/strict");
const test = require("node:test");
const { EventEmitter } = require("node:events");
const { denyAmbientCapabilities } = require("./session-policy");

function fakeSession() {
  return Object.assign(new EventEmitter(), {
    setPermissionRequestHandler(handler) { this.permission = handler; },
    setPermissionCheckHandler(handler) { this.check = handler; },
    setDevicePermissionHandler(handler) { this.device = handler; },
  });
}

test("浏览器无文件保存处理器时拒绝下载，其他环境权限继续拒绝", () => {
  const session = fakeSession();
  denyAmbientCapabilities(session, "browser");
  let denied = false;
  session.emit("will-download", { preventDefault() { denied = true; } }, { getFilename: () => "report.pdf" });
  assert.equal(denied, true);
  session.permission(null, "camera", (allowed) => assert.equal(allowed, false));
  assert.equal(session.check(), false);
  assert.equal(session.device(), false);
});

test("会话重新打开只保留当前保存处理器，拒绝和异常均阻止下载", () => {
  const session = fakeSession();
  denyAmbientCapabilities(session, "container", () => true);
  denyAmbientCapabilities(session, "container", () => false);
  assert.equal(session.listenerCount("will-download"), 1);
  let denied = 0;
  const event = { preventDefault() { denied += 1; } };
  const item = { getFilename: () => "report.pdf" };
  session.emit("will-download", event, item);
  denyAmbientCapabilities(session, "container", () => { throw new Error("invalid origin"); });
  session.emit("will-download", event, item);
  assert.equal(denied, 2);
  denyAmbientCapabilities(session, "container", () => true);
  session.emit("will-download", event, item);
  assert.equal(denied, 2);
});
