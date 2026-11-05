const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const test = require("node:test");
const vm = require("node:vm");
const { withBrowserFixture } = require("./headless-browser-fixture");

function browserProbe() {
  let attached;
  const views = new Map();
  const page = Object.assign(new EventEmitter(), {
    getURL: () => views.get("headless-browser").src,
    isDestroyed: () => false,
  });
  const host = Object.assign(new EventEmitter(), {
    executeJavaScript: async code => vm.runInNewContext(code, {
      document: {
        createElement: () => ({ style: {}, setAttribute(name, value) { this[name] = value; }, remove() { views.delete(this.id); } }),
        body: { appendChild(view) {
          views.set(view.id, view);
          host.emit("did-attach-webview", {}, page);
          attached();
        } },
        getElementById: id => views.get(id),
      },
    }),
  });
  const onOpenTab = () => assert.fail("自检标签不能通过普通界面打开");
  const runtime = {
    profileId: "isolated-profile",
    current: () => ({ contents: page }),
    activeIdOf: () => "tab&1",
    close() { this.closed = true; },
    onOpenTab,
    openTab(_url, { conversationId }) {
      return new Promise(resolve => {
        attached = () => resolve({ tabId: "tab&1" });
        this.onOpenTab("tab&1", conversationId);
      });
    },
  };
  return { host, views, runtime, onOpenTab, page };
}

test("个人版自检只在初始页面加载完成后操作本次真实标签并还原宿主", async () => {
  const { host, views, runtime, page, onOpenTab } = browserProbe();
  let checked = false;
  const result = withBrowserFixture(runtime, host, (active, tabId) => {
    assert.equal(active, page);
    assert.equal(tabId, "tab&1");
    assert.equal(runtime.onOpenTab, onOpenTab);
    checked = true;
    return "通过";
  });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(checked, false);
  const view = views.get("headless-browser");
  assert.equal(view.partition, "persist:ai-browser-isolated-profile");
  assert.equal(new URL(view.src).searchParams.get("cilaiTab"), "tab&1");
  page.emit("did-finish-load");
  assert.equal(await result, "通过");
  assert.equal(host.listenerCount("did-attach-webview"), 0);
  assert.equal(views.size, 0);
  assert.equal(runtime.closed, true);
});
