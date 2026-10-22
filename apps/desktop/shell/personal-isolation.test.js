const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { personalDirectories, rejectLinkedAncestors, APPLICATION_ID } = require('./personal-isolation');

test('个人资料与浏览器、日志、业务目录同属独立安装身份', () => {
  const dirs = personalDirectories({ packaged: true, appData: '/Users/test/Library/Application Support' });
  assert.equal(path.basename(dirs.userData), APPLICATION_ID);
  assert.equal(dirs.sessionData, path.join(dirs.userData, 'browser'));
  assert.deepEqual(dirs.hostArguments, ['--data-dir', path.join(dirs.userData, 'business')]);
  assert.throws(() => personalDirectories({ packaged: false, requestedUserData: '/enterprise' }), /独立资料目录/);
});

test('资料根和未创建的下级目录不能通过链接写向其他位置', t => {
  const root = fs.mkdtempSync(path.join(fs.realpathSync(os.tmpdir()), 'personal-links-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, 'outside'));
  fs.symlinkSync(path.join(root, 'outside'), path.join(root, 'linked'), 'dir');
  assert.throws(() => rejectLinkedAncestors(path.join(root, 'linked', 'new', 'data')), /符号链接/);
});
