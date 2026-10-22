const assert = require('node:assert/strict');
const vm = require('node:vm');
const test = require('node:test');
const { installBrowserFixture, removeBrowserFixture } = require('./headless-browser-fixture');

function browserProbe(loginPage) {
  const views = [];
  const subscriptions = new Map();
  const calls = [];
  const api = {
    transformCallback: callback => callback,
    async invoke(command, input) {
      calls.push({ command, input });
      if (command === 'browser_state') return { profileId: 'isolated-profile' };
      if (command === 'plugin:event|listen') { subscriptions.set(17, input); return 17; }
      if (command === 'plugin:event|unlisten') { subscriptions.delete(input.eventId); return; }
      throw new Error('unexpected call');
    },
  };
  const context = vm.createContext({ window: { __CILAI_HOST__: api }, document: {
    querySelector: () => loginPage,
    createElement: tag => ({ tag, attributes: {}, style: {}, setAttribute(name, value) { this.attributes[name] = value; },
      remove() { views.splice(views.indexOf(this), 1); } }),
    body: { appendChild: view => views.push(view) },
    getElementById: id => views.find(view => view.id === id),
  } });
  return { contents: { executeJavaScript: code => vm.runInContext(code, context) }, views, subscriptions, calls };
}

test('隔离登录页上的浏览器自检以当前profile创建受控WebView并注销监听', async () => {
  const probe = browserProbe(true);
  await installBrowserFixture(probe.contents);
  const subscription = probe.subscriptions.get(17);
  assert.equal(subscription.event, 'browser-open-tab');
  await subscription.handler({ payload: { tabId: 'tab&1', conversationId: 'conversation&1' } });
  assert.equal(probe.views.length, 1);
  assert.equal(probe.views[0].tag, 'webview');
  assert.equal(probe.views[0].attributes.partition, 'persist:ai-browser-isolated-profile');
  const url = new URL(probe.views[0].attributes.src);
  assert.equal(url.searchParams.get('cilaiTab'), 'tab&1');
  assert.equal(url.searchParams.get('cilaiConversation'), 'conversation&1');
  await removeBrowserFixture(probe.contents);
  assert.equal(probe.subscriptions.size, 0);
  assert.equal(probe.views.length, 0);
});

test('已登录页面保留真实浏览器事件处理，不注入匿名自检监听', async () => {
  const probe = browserProbe(false);
  await installBrowserFixture(probe.contents);
  await removeBrowserFixture(probe.contents);
  assert.equal(probe.calls.length, 0);
  assert.equal(probe.views.length, 0);
});
