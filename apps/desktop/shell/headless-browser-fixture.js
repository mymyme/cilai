async function installBrowserFixture(contents) {
  await contents.executeJavaScript(`(async () => {
    if (!document.querySelector('.enterprise-login-page')) return;
    const api = window.__CILAI_HOST__;
    const handler = api.transformCallback(async ({ payload }) => {
      const state = await api.invoke('browser_state', {});
      const view = document.createElement('webview');
      view.id = 'headless-browser';
      view.setAttribute('partition', 'persist:ai-browser-' + state.profileId);
      view.setAttribute('src', 'about:blank?cilaiTab=' + encodeURIComponent(payload.tabId)
        + (payload.conversationId ? '&cilaiConversation=' + encodeURIComponent(payload.conversationId) : ''));
      view.style.cssText = 'width:640px;height:480px';
      document.body.appendChild(view);
    });
    window.__headlessBrowserFixture = await api.invoke('plugin:event|listen', {
      event: 'browser-open-tab', target: { kind: 'Any' }, handler,
    });
  })()`);
}

async function removeBrowserFixture(contents) {
  await contents.executeJavaScript(`(async () => {
    if (window.__headlessBrowserFixture === undefined) return;
    await window.__CILAI_HOST__.invoke('plugin:event|unlisten', {
      event: 'browser-open-tab', eventId: window.__headlessBrowserFixture,
    });
    document.getElementById('headless-browser')?.remove();
    delete window.__headlessBrowserFixture;
  })()`);
}

module.exports = { installBrowserFixture, removeBrowserFixture };
