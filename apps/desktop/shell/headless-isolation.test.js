const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const { createHeadlessIsolation } = require('./headless-isolation');

test('普通启动不创建自检目录，自检每次新建且不复用员工资料或系统身份', () => {
  const temporaryDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'cilai-headless-isolation-test-'));
  const originalHome = process.env.HOME;
  const employee = path.join(temporaryDirectory, 'employee-data');
  fs.mkdirSync(employee);
  fs.writeFileSync(path.join(employee, 'session.json'), 'employee');
  try {
    assert.equal(createHeadlessIsolation({ headless: false, temporaryDirectory }), null);
    assert.deepEqual(fs.readdirSync(temporaryDirectory), ['employee-data']);
    const first = createHeadlessIsolation({ headless: true, temporaryDirectory });
    const second = createHeadlessIsolation({ headless: true, temporaryDirectory });
    assert.notEqual(first.root, second.root);
    for (const item of [first, second]) {
      assert.ok(path.isAbsolute(item.root));
      assert.ok(path.basename(item.root).startsWith('cilai-headless-'));
      for (const directory of [item.userData, item.sessionData, item.logs]) {
        assert.equal(path.dirname(directory), item.root);
        assert.equal(fs.statSync(directory).isDirectory(), true);
      }
      assert.deepEqual(item.hostArguments, ['--headless-check', '--data-dir', path.join(item.root, 'host')]);
      assert.equal(fs.existsSync(item.hostArguments[2]), false);
    }
    assert.equal(process.env.HOME, originalHome);
    assert.equal(fs.readFileSync(path.join(employee, 'session.json'), 'utf8'), 'employee');
    assert.throws(() => createHeadlessIsolation({ headless: true, temporaryDirectory: 'relative' }), /绝对路径/);
  } finally {
    fs.rmSync(temporaryDirectory, { recursive: true, force: true });
  }
});
