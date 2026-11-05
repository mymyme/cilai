const INITIAL_FIXTURE_READY_TIMEOUT_MS = 10_000;

function observeInitialFixtureLoads(runtime, contents) {
  const guests = new Map();
  const attached = (_event, guest) => {
    if (runtime.current(null)?.contents !== guest || guests.has(guest)) return;
    const record = { finishedUrl: null, error: null };
    const finished = () => {
      try { record.finishedUrl = guest.getURL(); }
      catch (error) { record.error = error; }
    };
    const destroyed = () => { record.error = new Error("浏览器自检初始页面已关闭"); };
    guests.set(guest, { record, finished, destroyed });
    guest.on("did-finish-load", finished);
    guest.on("destroyed", destroyed);
  };
  contents.on("did-attach-webview", attached);
  return {
    recordOf: (guest) => guests.get(guest)?.record,
    dispose() {
      contents.removeListener("did-attach-webview", attached);
      for (const [guest, handlers] of guests) {
        guest.removeListener("did-finish-load", handlers.finished);
        guest.removeListener("destroyed", handlers.destroyed);
      }
      guests.clear();
    },
  };
}

function waitForInitialFixture({ runtime, page, tabId, deadline, initialLoads }) {
  const record = initialLoads.recordOf(page);
  if (!record) return Promise.reject(new Error("浏览器自检未观测到本轮初始页面挂载"));
  const initialUrl = `about:blank?cilaiTab=${encodeURIComponent(tabId)}`;
  return new Promise((resolve, reject) => {
    let settled = false;
    let timer;
    const events = ["dom-ready", "did-finish-load", "did-stop-loading"];
    const finish = (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      for (const event of events) page.removeListener(event, check);
      page.removeListener("destroyed", destroyed);
      if (error) reject(error);
      else resolve();
    };
    const destroyed = () => finish(new Error("浏览器自检初始页面已关闭"));
    const expired = () => {
      try {
        finish(new Error(`浏览器自检初始页面未在 ${INITIAL_FIXTURE_READY_TIMEOUT_MS} 毫秒内就绪：${JSON.stringify(page.getURL())}`));
      } catch (error) {
        finish(error);
      }
    };
    const check = () => {
      try {
        if (record.error) return finish(record.error);
        if (page.isDestroyed()) return destroyed();
        if (runtime.current(null)?.contents !== page || runtime.activeIdOf(null) !== tabId) {
          return finish(new Error("浏览器自检初始页面已被替换"));
        }
        if (Date.now() >= deadline) return expired();
        if (record.finishedUrl === initialUrl && page.getURL() === initialUrl) finish();
      } catch (error) {
        finish(error);
      }
    };
    // The pre-ensure observer records the URL at the actual finish event.
    // DOM readiness or loading UI state alone cannot prove it was consumed.
    for (const event of events) page.on(event, check);
    page.on("destroyed", destroyed);
    timer = setTimeout(expired, Math.max(0, deadline - Date.now()));
    check();
  });
}

const VIEW_ID = "headless-browser";

async function withBrowserFixture(runtime, contents, check) {
  const previousOpen = runtime.onOpenTab;
  const initialLoads = observeInitialFixtureLoads(runtime, contents);
  const deadline = Date.now() + INITIAL_FIXTURE_READY_TIMEOUT_MS;
  try {
    const mounted = Promise.withResolvers();
    runtime.onOpenTab = (tabId, conversationId) => {
      if (conversationId !== null) return previousOpen(tabId, conversationId);
      const partition = `persist:ai-browser-${runtime.profileId}`;
      const url = `about:blank?cilaiTab=${encodeURIComponent(tabId)}`;
      mounted.resolve(contents.executeJavaScript(`(() => {
        const view = document.createElement("webview");
        view.id = ${JSON.stringify(VIEW_ID)};
        view.setAttribute("partition", ${JSON.stringify(partition)});
        view.setAttribute("src", ${JSON.stringify(url)});
        view.style.cssText = "position:fixed;top:0;left:0;width:640px;height:480px";
        document.body.appendChild(view);
      })()`));
    };
    const [{ tabId }] = await Promise.all([runtime.openTab(null, { conversationId: null }), mounted.promise]);
    runtime.onOpenTab = previousOpen;
    const page = runtime.current(null)?.contents;
    if (!page) throw new Error("浏览器自检初始标签不可用");
    await waitForInitialFixture({ runtime, page, tabId, deadline, initialLoads });
    return await check(page, tabId);
  } finally {
    runtime.onOpenTab = previousOpen;
    initialLoads.dispose();
    try {
      runtime.close();
    } finally {
      await contents.executeJavaScript(`document.getElementById(${JSON.stringify(VIEW_ID)})?.remove()`);
    }
  }
}

module.exports = { withBrowserFixture };
