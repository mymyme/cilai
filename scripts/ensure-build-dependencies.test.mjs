import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { acquireBuildLock, releaseBuildLock } from './build-lock.mjs';
import { ensureBuildDependencies } from './ensure-build-dependencies.mjs';

const json = (path, value) => { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, JSON.stringify(value, null, 2)); };
const read = path => JSON.parse(readFileSync(path, 'utf8'));
const runtime = { node: 'v24.18.0', platform: 'darwin', arch: 'arm64' };

function fixture(t) {
  const parent = mkdtempSync(join(realpathSync(tmpdir()), 'cilai-dependency-reuse-'));
  const root = join(parent, 'personal'); mkdirSync(root);
  json(join(root, 'package.json'), { name: 'workspace', version: '1.0.0', workspaces: ['apps/desktop-ui'] });
  json(join(root, 'apps/desktop-ui/package.json'), { name: '@local/ui', version: '1.0.0' });
  json(join(root, 'package-lock.json'), { lockfileVersion: 3, packages: {
    '': { version: '1.0.0' }, 'node_modules/alpha': { version: '1.0.0' },
    'node_modules/optional-package': { version: '1.0.0', optional: true },
  } });
  json(join(root, 'apps/desktop/package.json'), { name: 'desktop', version: '1.0.0' });
  json(join(root, 'apps/desktop/package-lock.json'), { lockfileVersion: 3, packages: {
    '': { version: '1.0.0' }, 'node_modules/beta': { version: '2.0.0' },
  } });
  const token = acquireBuildLock(root, 'fixture');
  t.after(() => { releaseBuildLock(root, token); rmSync(parent, { recursive: true, force: true }); });
  const calls = [];
  const installer = async (directory, environment, actualRuntime) => {
    calls.push({ directory, environment, runtime: actualRuntime });
    const packages = read(join(directory, 'package-lock.json')).packages;
    const hidden = {};
    rmSync(join(directory, 'node_modules'), { recursive: true, force: true });
    mkdirSync(join(directory, 'node_modules'));
    for (const [name, entry] of Object.entries(packages)) {
      if (!name.startsWith('node_modules/') || entry.optional) continue;
      hidden[name] = entry;
      const destination = join(directory, name);
      if (entry.link) {
        mkdirSync(dirname(destination), { recursive: true });
        symlinkSync(join(directory, entry.resolved), destination, process.platform === 'win32' ? 'junction' : 'dir');
      } else json(join(destination, 'package.json'), { name: name.split('/').at(-1), version: entry.version });
    }
    json(join(directory, 'node_modules/.package-lock.json'), { lockfileVersion: 3, packages: hidden });
  };
  const options = { buildRoot: root, expectedRoot: root, runtime,
    environment: { CILAI_BUILD_LOCK_TOKEN: token, PATH: process.env.PATH }, installer };
  return { root, parent, token, calls, installer, options, state: join(root, '.cilai-dependency-state.json') };
}

test('首次成功安装两组后记录，重复调用验证实际目录并复用', async t => {
  const f = fixture(t);
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['installed', 'installed']);
  const state = readFileSync(f.state, 'utf8');
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['reused', 'reused']);
  assert.equal(f.calls.length, 2);
  assert.equal(readFileSync(f.state, 'utf8'), state);
  for (const call of f.calls) {
    assert.equal(call.environment.npm_config_cache, join(f.root, 'cache/npm'));
    assert.equal(call.environment.ELECTRON_CACHE, join(f.root, 'cache/electron'));
    assert.equal(call.environment.CILAI_BUILD_LOCK_TOKEN, f.token);
  }
});

test('UI工作区输入变化只重装root组，平台架构变化重装两组', async t => {
  const f = fixture(t); await ensureBuildDependencies(f.options);
  json(join(f.root, 'apps/desktop-ui/package.json'), { name: '@local/ui', version: '1.0.0', private: true });
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['installed', 'reused']);
  assert.deepEqual((await ensureBuildDependencies({ ...f.options, runtime: { ...runtime, arch: 'x64' } })).map(x => x.action), ['installed', 'installed']);
});

test('主锁版本或已安装package版本变化均重新安装对应组', async t => {
  const f = fixture(t); await ensureBuildDependencies(f.options);
  const lock = read(join(f.root, 'package-lock.json')); lock.packages['node_modules/alpha'].version = '1.1.0';
  json(join(f.root, 'package-lock.json'), lock);
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['installed', 'reused']);
  json(join(f.root, 'apps/desktop/node_modules/beta/package.json'), { name: 'beta', version: 'incorrect' });
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['reused', 'installed']);
});

test('hidden-lock摘要变化即使包版本相同也需要重建成功记录', async t => {
  const f = fixture(t); await ensureBuildDependencies(f.options);
  const hidden = join(f.root, 'node_modules/.package-lock.json');
  writeFileSync(hidden, readFileSync(hidden, 'utf8') + '\n');
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['installed', 'reused']);
});

test('npm ci失败不写成功记录，后置目录校验失败也不写', async t => {
  const f = fixture(t);
  await assert.rejects(ensureBuildDependencies({ ...f.options, installer: async () => { throw new Error('fixture npm failure'); } }), /fixture npm failure/);
  assert.equal(existsSync(f.state), false);
  await assert.rejects(ensureBuildDependencies({ ...f.options, installer: async () => {} }), /npm ci后workspace依赖校验失败/);
  assert.equal(existsSync(f.state), false);
});

test('第二组安装失败不撤销已验证成功的第一组，下次只补装第二组', async t => {
  const f = fixture(t);
  await assert.rejects(ensureBuildDependencies({ ...f.options, installer: async (...args) => {
    if (args[0].endsWith(join('apps', 'desktop'))) throw new Error('desktop failed');
    await f.installer(...args);
  } }), /desktop failed/);
  assert.deepEqual(Object.keys(read(f.state).groups), ['workspace']);
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['reused', 'installed']);
});

test('未持有构建锁、非标准根或Node版本不符时不调用安装', async t => {
  const f = fixture(t);
  await assert.rejects(ensureBuildDependencies({ ...f.options, environment: { CILAI_BUILD_LOCK_TOKEN: '' } }), /构建锁无效/);
  await assert.rejects(ensureBuildDependencies({ ...f.options, expectedRoot: f.parent }), /标准构建根/);
  await assert.rejects(ensureBuildDependencies({ ...f.options, runtime: { ...runtime, node: 'v22.0.0' } }), /Node 24.18.0/);
  assert.equal(f.calls.length, 0); assert.equal(existsSync(f.state), false);
});

test('依赖目录或嵌套包链接越界时拒绝，不借重装改动外部目录', async t => {
  const f = fixture(t);
  const external = join(f.parent, 'enterprise'); mkdirSync(external);
  json(join(external, 'package.json'), { version: '1.0.0' });
  symlinkSync(external, join(f.root, 'node_modules'), process.platform === 'win32' ? 'junction' : 'dir');
  await assert.rejects(ensureBuildDependencies(f.options), /越过构建根/);
  rmSync(join(f.root, 'node_modules'));
  await ensureBuildDependencies(f.options);
  const alpha = join(f.root, 'node_modules/alpha'); rmSync(alpha, { recursive: true });
  symlinkSync(external, alpha, process.platform === 'win32' ? 'junction' : 'dir');
  const count = f.calls.length;
  await assert.rejects(ensureBuildDependencies(f.options), /越过构建根/);
  assert.equal(f.calls.length, count);
  assert.equal(read(join(external, 'package.json')).version, '1.0.0');
});

test('根内workspace链接可复用，optional未安装不造成循环重装', async t => {
  const f = fixture(t);
  const lock = read(join(f.root, 'package-lock.json'));
  lock.packages['apps/desktop-ui'] = { version: '1.0.0' };
  lock.packages['node_modules/@local/ui'] = { resolved: 'apps/desktop-ui', link: true };
  json(join(f.root, 'package-lock.json'), lock);
  await ensureBuildDependencies(f.options);
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['reused', 'reused']);
  assert.equal(f.calls.length, 2);
  assert.equal(realpathSync(join(f.root, 'node_modules/@local/ui')), join(f.root, 'apps/desktop-ui'));
});

test('输入在npm ci期间变化时拒绝记录，后续仍需重装', async t => {
  const f = fixture(t);
  await assert.rejects(ensureBuildDependencies({ ...f.options, installer: async (...args) => {
    await f.installer(...args);
    json(join(f.root, 'package.json'), { name: 'changed', version: '1.0.0' });
  } }), /依赖输入发生变化/);
  assert.equal(existsSync(f.state), false);
});

test('hidden-lock声明的包被移除或必需包从hidden-lock消失均会重装', async t => {
  const f = fixture(t); await ensureBuildDependencies(f.options);
  rmSync(join(f.root, 'node_modules/alpha'), { recursive: true });
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['installed', 'reused']);
  json(join(f.root, 'node_modules/.package-lock.json'), { lockfileVersion: 3, packages: {} });
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['installed', 'reused']);
});


test('file本地归档同名内容变化会重装所属组，未改动组继续复用', async t => {
  const f = fixture(t), archive = join(f.root, 'packages/local-ui.tgz');
  mkdirSync(dirname(archive), { recursive: true }); writeFileSync(archive, 'archive-version-one');
  json(join(f.root, 'apps/desktop-ui/package.json'), { name: '@local/ui', version: '1.0.0', dependencies: { alpha: 'file:../../packages/local-ui.tgz' } });
  const lock = read(join(f.root, 'package-lock.json'));
  lock.packages['node_modules/alpha'].resolved = 'file:packages/local-ui.tgz';
  lock.packages['apps/desktop-ui'] = { dependencies: { alpha: 'file:../../packages/local-ui.tgz' } };
  json(join(f.root, 'package-lock.json'), lock);
  await ensureBuildDependencies(f.options);
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['reused', 'reused']);
  writeFileSync(archive, 'archive-version-two');
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['installed', 'reused']);
});

test('仅主锁resolved引用的本地归档内容也进入摘要', async t => {
  const f = fixture(t), archive = join(f.root, 'apps/desktop/desktop.tgz');
  writeFileSync(archive, 'first');
  const lock = read(join(f.root, 'apps/desktop/package-lock.json'));
  lock.packages['node_modules/beta'].resolved = 'file:desktop.tgz';
  json(join(f.root, 'apps/desktop/package-lock.json'), lock);
  await ensureBuildDependencies(f.options); writeFileSync(archive, 'second');
  assert.deepEqual((await ensureBuildDependencies(f.options)).map(x => x.action), ['reused', 'installed']);
});

test('本地归档越界或通过链接指向其他工程时拒绝安装及写复用记录', async t => {
  const f = fixture(t), external = join(f.parent, 'enterprise.tgz'); writeFileSync(external, 'enterprise archive');
  json(join(f.root, 'package.json'), { name: 'workspace', version: '1.0.0', dependencies: { alpha: 'file:../enterprise.tgz' } });
  await assert.rejects(ensureBuildDependencies(f.options), /越过构建根/);
  symlinkSync(external, join(f.root, 'linked.tgz'));
  json(join(f.root, 'package.json'), { name: 'workspace', version: '1.0.0', dependencies: { alpha: 'file:linked.tgz' } });
  await assert.rejects(ensureBuildDependencies(f.options), /越过构建根/);
  assert.equal(f.calls.length, 0); assert.equal(existsSync(f.state), false);
  assert.equal(readFileSync(external, 'utf8'), 'enterprise archive');
});

test('本地归档在安装期间变化时不记录成功', async t => {
  const f = fixture(t), archive = join(f.root, 'local.tgz'); writeFileSync(archive, 'before');
  json(join(f.root, 'package.json'), { name: 'workspace', version: '1.0.0', dependencies: { alpha: 'file:local.tgz' } });
  await assert.rejects(ensureBuildDependencies({ ...f.options, installer: async (...args) => {
    await f.installer(...args); writeFileSync(archive, 'after');
  } }), /依赖输入发生变化/);
  assert.equal(existsSync(f.state), false);
});
