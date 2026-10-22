function browserIdentity(src) {
  try {
    const parsed = new URL(String(src ?? ""), "about:blank");
    return { tabId: parsed.searchParams.get("cilaiTab"), conversationId: parsed.searchParams.get("cilaiConversation") };
  } catch {
    return { tabId: null, conversationId: null };
  }
}
function guardBrowserWebviews(window, browser) {
  let pending = null;
  window.webContents.on("will-attach-webview", (event, preferences, params) => {
    preferences.nodeIntegration = false;
    preferences.contextIsolation = true;
    preferences.backgroundThrottling = false;
    delete preferences.preload;
    const identity = browserIdentity(params.src);
    if (browser.canAttach?.() === false || !browser.isPartition(typeof params.partition === "string" ? params.partition : "") || !identity.tabId) {
      pending = null;
      event.preventDefault();
      return;
    }
    pending = identity;
  });
  window.webContents.on("did-attach-webview", (_event, contents) => {
    const identity = pending;
    pending = null;
    if (browser.canAttach?.() === false) {
      contents.close({ waitForBeforeUnload: false });
      return;
    }
    if (identity) browser.adopt(identity.tabId, contents, identity.conversationId ?? null);
  });
}
module.exports = { guardBrowserWebviews };
