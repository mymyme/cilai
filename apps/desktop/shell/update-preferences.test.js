const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { updatePreferences } = require('./update-preferences');
test('自动下载默认开启，手动关闭跨启动保留；损坏配置明确报错', t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cilai-update-preferences-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const preferences = updatePreferences(root);
  assert.equal(preferences.load().autoDownload, true);
  preferences.save({ autoDownload: false });
  assert.equal(updatePreferences(root).load().autoDownload, false);
  fs.writeFileSync(path.join(root, 'update-preferences.json'), '{broken');
  assert.throws(() => preferences.load(), /无法读取更新偏好/);
});
