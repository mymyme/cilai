const test = require("node:test");
const assert = require("node:assert/strict");
const { createPersonalNotificationNavigation } = require("./personal-notification-navigation");
const profileId = "1".repeat(32), notificationId = "a".repeat(32);
const request = { profileId, notificationId };
function fixture(options = {}) {
  const calls = [], emitted = [];
  let profile = profileId;
  let owner = { isDestroyed: () => false, mainFrame: { processId: 1, routingId: 2, detached: false } };
  let event = { sender: owner, senderFrame: owner.mainFrame };
  const state = {
    target: { kind: "run", runId: "run-one", conversationId: "conversation-one" },
    location: { creationProjectId: "personal-one", documentUrl: "app://personal-capability/11111111-1111-4111-8111-111111111111" },
    get owner() { return owner; }, get event() { return event; },
    setProfile(value) { profile = value; },
    replaceFrame() { owner.mainFrame = { processId: 3, routingId: 4, detached: false }; event = { sender: owner, senderFrame: owner.mainFrame }; },
  };
  const broker = createPersonalNotificationNavigation({
    documents: { verify(input) { if (input !== event) throw Error("frame_rejected"); } },
    service: { async call(command, args) { calls.push({ command, args }); if (state.read) await state.read(); return state.target; } },
    pageAi: { async notificationLocation(target) { calls.push({ page: target }); if (state.pageFailure) throw Error("page_closed"); return state.location; } },
    getOwner: () => owner, getProfile: () => profile,
    emit(name, payload) { emitted.push({ name, payload }); return options.listeners ?? 1; },
    timeoutMs: options.timeoutMs ?? 1000,
  });
  return { state, broker, calls, emitted, ready: () => broker.ready(state.event, { profileId }) };
}

test("冷启动点击等真实主界面订阅就绪后才核目标并定位，只有一次只读查询", async () => {
  const f = fixture();
  try {
    const operation = f.broker.open(request);
    assert.deepEqual(f.calls, []);
    f.ready(); await operation;
    assert.deepEqual(f.calls, [{ command: "personal_notifications_open_internal", args: { request } }]);
    assert.deepEqual(f.emitted, [{ name: "personal-notification-open", payload: { ...request, target: f.state.target } }]);
  } finally { f.broker.stop(); }
});

test("主页面失效或资料切换后，迟到目标不会导航或重做查询", async () => {
  for (const change of ["profile", "frame", "invalidate"]) {
    const f = fixture(); f.ready();
    let release; f.state.read = () => new Promise(resolve => { release = resolve; });
    const operation = f.broker.open(request);
    const rejected = assert.rejects(operation, /target_missing/);
    if (change === "profile") f.state.setProfile("2".repeat(32));
    if (change === "frame") f.state.replaceFrame();
    if (change === "invalidate") f.broker.invalidate();
    release(); await rejected;
    assert.equal(f.emitted.length, 0); assert.equal(f.calls.length, 1); f.broker.stop();
  }
});

test("页面通知只定位宿主仍持有的同一文档，不打开页面或运行模型", async () => {
  const f = fixture(); f.ready();
  try {
    f.state.target = { kind: "application", applicationId: "app-one", runId: "run-one", sessionId: "session-one" };
    await f.broker.open(request);
    assert.equal(f.calls.length, 2);
    assert.equal(f.calls[0].command, "personal_notifications_open_internal");
    assert.deepEqual(f.calls[1], { page: f.state.target });
    assert.deepEqual(f.emitted[0].payload.target, { ...f.state.target, ...f.state.location });
    f.state.pageFailure = true;
    await assert.rejects(f.broker.open(request), /page_closed/);
    assert.equal(f.emitted.length, 1);
  } finally { f.broker.stop(); }
});

test("过期或丢失的主界面点击不补播，目标失败不重试", async () => {
  const f = fixture({ timeoutMs: 15 });
  try {
    await assert.rejects(f.broker.open(request), /target_missing/);
    f.ready(); await new Promise(resolve => setImmediate(resolve));
    assert.equal(f.calls.length, 0); assert.equal(f.emitted.length, 0);
    f.state.read = () => { throw Error("missing"); };
    await assert.rejects(f.broker.open(request), /missing/);
    assert.equal(f.calls.length, 1); assert.equal(f.emitted.length, 0);
  } finally { f.broker.stop(); }
});

test("导航握手拒绝子页面、外来字段和跨资料点击；无订阅回执不标成功", async () => {
  const f = fixture({ listeners: 0 });
  try {
    assert.throws(() => f.broker.ready({}, { profileId }), /frame_rejected/);
    assert.throws(() => f.broker.ready(f.state.event, { profileId, path: "/injected" }), /target_missing/);
    await assert.rejects(f.broker.open({ ...request, profileId: "2".repeat(32) }), /target_missing/);
    await assert.rejects(f.broker.open({ ...request, notificationId: "../path" }), /target_missing/);
    f.ready(); await assert.rejects(f.broker.open(request), /target_missing/);
    assert.equal(f.calls.length, 1);
    assert.equal(f.broker.handles("personal_notifications_open_internal"), false);
  } finally { f.broker.stop(); }
});
