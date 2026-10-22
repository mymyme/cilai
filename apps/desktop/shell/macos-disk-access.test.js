const test = require('node:test');
const assert = require('node:assert/strict');
const { createMacosDiskAccess } = require('./macos-disk-access');

function fixture() {
  const mainFrame = { url: 'app://localhost/index.html' };
  const owner = { mainFrame, isDestroyed: () => false };
  const calls = [];
  return { owner, event: { sender: owner, senderFrame: mainFrame }, calls,
    handler: createMacosDiskAccess(() => owner, (command, args) => { calls.push({ command, args }); return null; }) };
}

test('仅主界面的固定两操作可转交，无参数可以改变URL或应用路径', async () => {
  const { event, handler, calls } = fixture();
  for (const command of ['macos_disk_access_open_settings', 'macos_disk_access_reveal_application']) {
    assert.ok(handler.handles(command)); await handler.invoke(event, { command, args: {} });
  }
  assert.equal(calls.length, 2);
  assert.deepEqual(calls[0].args, {});
  assert.equal(handler.handles('macos_disk_access_grant'), false);
  for (const args of [null, [], { url: 'https://invalid.example' }, { path: '/Other.app' }, { command: 'grant' }]) {
    await assert.rejects(handler.invoke(event, { command: 'macos_disk_access_open_settings', args }), /不接受/);
  }
  assert.equal(calls.length, 2);
});

test('应用页面、子框架、销毁窗口和外部页面均不能调用', async () => {
  const { event, handler, calls, owner } = fixture();
  const message = { command: 'macos_disk_access_open_settings', args: {} };
  for (const other of [{ ...event, sender: {} }, { ...event, senderFrame: { url: 'app://localhost/child' } }]) await assert.rejects(handler.invoke(other, message), /主界面/);
  owner.mainFrame.url = 'https://example.com'; await assert.rejects(handler.invoke(event, message), /主界面/);
  owner.mainFrame.url = 'app://localhost/'; owner.isDestroyed = () => true; await assert.rejects(handler.invoke(event, message), /主界面/);
  assert.equal(calls.length, 0);
});
