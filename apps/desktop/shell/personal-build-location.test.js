const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { personalBuildRoot, sourceContext, registerPersonalBuild, registrationName } = require('./personal-build-location');

function fixture(t) {
  const directory = fs.mkdtempSync(path.join(fs.realpathSync(os.tmpdir()), 'cilai-personal-location-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const home = path.join(directory, 'home');
  const projectRoot = path.join(directory, 'project');
  const sourceRoot = path.join(projectRoot, '06代码索引/cilai-workbench');
  fs.mkdirSync(path.join(sourceRoot, 'apps/desktop'), { recursive: true });
  fs.writeFileSync(path.join(sourceRoot, 'apps/desktop/electron-builder.yml'), 'appId: cn.cilai.personal.workbench\n');
  return { directory, home, projectRoot, sourceRoot, buildRoot: personalBuildRoot({ home }) };
}

test('两平台固定用户本地根且拒绝云同步主目录', () => {
  assert.equal(personalBuildRoot({ platform: 'darwin', home: '/Users/person' }), '/Users/person/.cilai-personal-builds/desktop');
  assert.equal(personalBuildRoot({ platform: 'win32', home: 'C:\\Users\\person' }), 'C:\\Users\\person\\.cilai-personal-builds\\desktop');
  for (const home of ['/Users/person/Library/Mobile Documents/com~apple~CloudDocs', '/Users/person/Library/CloudStorage/OneDrive-Corp', '/Users/person/Dropbox', '/Users/person/iCloud Drive']) assert.throws(() => personalBuildRoot({ platform: 'darwin', home }), /非云同步目录/);
  for (const home of ['C:\\Users\\person\\OneDrive', 'C:\\Users\\person\\OneDrive - Corp', 'D:\\Google Drive']) assert.throws(() => personalBuildRoot({ platform: 'win32', home }), /非云同步目录/);
  assert.throws(() => personalBuildRoot({ platform: 'darwin', home: 'relative' }), /主目录/);
});

test('源码和暂存以登记定位同一工程，不从新根祖父目录猜测', t => {
  const f = fixture(t);
  const expected = { buildRoot: f.buildRoot, sourceRoot: f.sourceRoot, projectRoot: f.projectRoot };
  assert.deepEqual(sourceContext(f.sourceRoot, { home: f.home }), expected);
  fs.mkdirSync(f.buildRoot, { recursive: true });
  assert.throws(() => sourceContext(f.buildRoot, { home: f.home }), /尚未登记/);
  const receipt = registerPersonalBuild(f.sourceRoot, { home: f.home });
  assert.equal(receipt.applicationId, 'cn.cilai.personal.workbench');
  assert.deepEqual(sourceContext(f.buildRoot, { home: f.home }), expected);
  assert.deepEqual(sourceContext(path.join(f.buildRoot, 'runtime-source'), { home: f.home }), expected);
  assert.deepEqual(registerPersonalBuild(f.sourceRoot, { home: f.home }), receipt);
  assert.throws(() => sourceContext(path.join(f.projectRoot, '.personal-build'), { home: f.home }), /布局/);
  assert.equal(fs.existsSync(path.join(f.projectRoot, '.personal-build')), false);
});

test('登记缺失或篡改时明确停止，保留原登记', t => {
  const f = fixture(t);
  registerPersonalBuild(f.sourceRoot, { home: f.home });
  const file = path.join(f.buildRoot, registrationName);
  const value = JSON.parse(fs.readFileSync(file, 'utf8'));
  value.buildRoot = path.join(f.home, '.cilai-builds/desktop');
  fs.writeFileSync(file, JSON.stringify(value));
  assert.throws(() => sourceContext(f.sourceRoot, { home: f.home }), /登记无效/);
  assert.throws(() => sourceContext(f.buildRoot, { home: f.home }), /登记无效/);
  assert.throws(() => registerPersonalBuild(f.sourceRoot, { home: f.home }), /登记无效/);
  assert.equal(JSON.parse(fs.readFileSync(file, 'utf8')).buildRoot, value.buildRoot);
});

test('企业源码、源码改换和链接根均不获得个人构建登记', t => {
  const f = fixture(t);
  fs.writeFileSync(path.join(f.sourceRoot, 'apps/desktop/electron-builder.yml'), 'appId: cn.cilai.workbench\n');
  assert.throws(() => registerPersonalBuild(f.sourceRoot, { home: f.home }), /不是已登记的个人版/);
  assert.equal(fs.existsSync(f.buildRoot), false);
  fs.writeFileSync(path.join(f.sourceRoot, 'apps/desktop/electron-builder.yml'), 'appId: cn.cilai.personal.workbench\n');
  fs.mkdirSync(path.dirname(f.buildRoot), { recursive: true });
  fs.mkdirSync(path.join(f.directory, 'outside'));
  fs.symlinkSync(path.join(f.directory, 'outside'), f.buildRoot, 'dir');
  assert.throws(() => registerPersonalBuild(f.sourceRoot, { home: f.home }), /符号链接/);
  assert.deepEqual(fs.readdirSync(path.join(f.directory, 'outside')), []);
});
