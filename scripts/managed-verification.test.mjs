import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { parseManagedArguments, runManagedVerification } from './managed-verification.mjs';

function fixture() {
  const calls = [];
  const options = { buildRoot: '/tmp/managed-build', source: '/tmp/managed-source', environment: { CILAI_BUILD_LOCK_TOKEN: 'held' },
    run: async (...args) => { calls.push(args); return 0; }, assertLock: (...args) => calls.push(['lock', ...args]), verify: (...args) => calls.push(['verify', ...args]) };
  return { calls, options };
}
test('首次验证只由既有构建锁创建生命周期，命令参数不经 shell', async () => {
  const f = fixture(); f.options.environment = {};
  f.options.lock = async (...args) => { f.calls.push(args); return 7; };
  assert.equal(await runManagedVerification({ ...f.options, command: ['node', 'fixture.mjs', 'a; b'] }), 7);
  assert.equal(f.calls.length, 1);
  assert.deepEqual(f.calls[0][0].slice(-4), ['--', 'node', 'fixture.mjs', 'a; b']);
  assert.equal(f.calls[0][1], '/tmp/managed-build');
});
test('继承锁先暂存一次，再在固定构建根执行并核验冻结清单', async () => {
  const f = fixture();
  assert.equal(await runManagedVerification({ ...f.options, command: ['node', 'scripts/probe.mjs'] }), 0);
  const runs = f.calls.filter((call) => typeof call[1] !== 'string');
  assert.equal(runs.length, 2);
  assert.deepEqual(runs[0][1], ['/tmp/managed-source/scripts/stage-build.mjs']);
  assert.equal(runs[0][2].cwd, '/tmp/managed-source');
  assert.deepEqual(runs[1][1], ['scripts/probe.mjs']);
  assert.equal(runs[1][2].cwd, '/tmp/managed-build');
  assert.equal(runs[1][2].env.CARGO_TARGET_DIR, '/tmp/managed-build/target');
  assert.equal(runs[1][2].env.CILAI_BUILD_LOCK_TOKEN, 'held');
  assert.equal(f.calls.filter((call) => call[0] === 'verify').length, 2);
});
test('质量检查的已暂存验证复用锁和源码，不再次暂存', async () => {
  const f = fixture(); f.options.source = f.options.buildRoot;
  f.options.assertLock = () => ({ manifestDigest: 'frozen' });
  await runManagedVerification({ ...f.options, command: ['node', 'probe.mjs'] });
  assert.equal(f.calls.filter((call) => typeof call[1] !== 'string').length, 1);
});
test('暂存或验证失败不继续执行、错误锁不写入目录', async () => {
  const f = fixture(); f.options.run = async (...args) => { f.calls.push(args); return 2; };
  assert.equal(await runManagedVerification({ ...f.options, command: ['node', 'probe.mjs'] }), 2);
  assert.equal(f.calls.filter((call) => typeof call[1] !== 'string').length, 1);
  f.options.assertLock = () => { throw new Error('wrong lock'); };
  await assert.rejects(runManagedVerification({ ...f.options, command: ['node', 'probe.mjs'] }), /wrong lock/);
});
test('Electron 使用构建根安装的运行时及 desktop cwd', async () => {
  const f = fixture();
  await runManagedVerification({ ...f.options, desktop: true, command: ['electron', 'prototype/main.js', '--verify-ime'] });
  const call = f.calls.at(-2);
  assert.equal(call[1][0], '/tmp/managed-build/apps/desktop/node_modules/electron/cli.js');
  assert.equal(call[2].cwd, '/tmp/managed-build/apps/desktop');
});
test('参数解析拒绝省略边界或空命令', () => {
  assert.deepEqual(parseManagedArguments(['--desktop', '--', 'electron', 'p.js']), { desktop: true, command: ['electron', 'p.js'] });
  for (const args of [[], ['node'], ['--']]) assert.throws(() => parseManagedArguments(args), /验证入口/);
});
test('两级标准测试与公开 Electron 验证入口自动纳管', () => {
  const root = JSON.parse(readFileSync(new URL('../package.json', import.meta.url)));
  const desktop = JSON.parse(readFileSync(new URL('../apps/desktop/package.json', import.meta.url)));
  assert.equal(root.scripts['verify:managed'], 'node scripts/managed-verification.mjs --');
  assert.equal(root.scripts.test, 'node scripts/managed-verification.mjs -- npm run test:unit');
  assert.match(root.scripts['test:unit'], /test:storage/);
  assert.equal(root.scripts.build, 'node scripts/managed-verification.mjs -- npm run build:ui');
  assert.equal(root.scripts['build:ui'], 'npm run build --workspace @contexthub/desktop-ui');
  for (const [name, value] of Object.entries(desktop.scripts).filter(([name]) => name.startsWith('verify:'))) {
    assert.match(value, /^node \.\.\/\.\.\/scripts\/managed-verification\.mjs --desktop -- electron /, name);
  }
});

test('个人版验证沿用独立缓存和依赖复用检查，暂存成功后才执行', async (t) => {
  const base = mkdtempSync(join(tmpdir(), 'cilai-managed-personal-'));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const f = fixture(); f.options.source = join(base, 'source'); f.options.buildRoot = join(base, 'build');
  mkdirSync(join(f.options.source, 'scripts'), { recursive: true });
  writeFileSync(join(f.options.source, 'scripts/personal-build.mjs'), '');
  await runManagedVerification({ ...f.options, desktop: true, command: ['electron', 'prototype/main.js', '--verify-perf'] });
  const runs = f.calls.filter((call) => typeof call[1] !== 'string');
  assert.equal(runs.length, 4);
  assert.match(runs[1][1][0], /ensure-build-dependencies\.mjs$/);
  assert.equal(runs[1][1][1], '--ci');
  assert.match(runs[2][1][0], /prepare-electron-runtime\.mjs$/);
  for (const run of runs) {
    assert.equal(run[2].env.CARGO_HOME, join(f.options.buildRoot, 'toolchains/cargo'));
    assert.equal(run[2].env.npm_config_cache, join(f.options.buildRoot, 'cache/npm'));
  }
  assert.ok(existsSync(join(f.options.buildRoot, 'config/npmrc')));
});
test('命令失败保留退出码，不把失败记为最终验证通过', async () => {
  const f = fixture(); f.options.source = f.options.buildRoot;
  f.options.assertLock = () => ({ manifestDigest: 'frozen' });
  f.options.run = async () => 9;
  assert.equal(await runManagedVerification({ ...f.options, command: ['node', 'probe.mjs'] }), 9);
  assert.equal(f.calls.filter((call) => call[0] === 'verify').length, 1);
});
test('使用调用方 npm CLI 的绝对路径，不引用源码树 npm bin', async () => {
  const f = fixture(); f.options.source = f.options.buildRoot;
  f.options.environment.npm_execpath = fileURLToPath(new URL('./managed-verification.test.mjs', import.meta.url));
  await runManagedVerification({ ...f.options, command: ['npm', 'run', 'test:unit'] });
  const run = f.calls.at(-2);
  assert.deepEqual(run[1], [f.options.environment.npm_execpath, 'run', 'test:unit']);
  delete f.options.environment.npm_execpath;
  await assert.rejects(runManagedVerification({ ...f.options, command: ['npm', 'run', 'test:unit'] }), /标准 npm 入口/);
});

test('已冻结锁从源码路径调用专项验证也不重复暂存，初次从构建根进入先登记', async () => {
  const f = fixture();
  f.options.assertLock = () => ({ manifestDigest: 'frozen' });
  await runManagedVerification({ ...f.options, command: ['node', 'scripts/probe.mjs'] });
  assert.equal(f.calls.filter((call) => typeof call[1] !== 'string').length, 1);
  f.calls.length = 0; f.options.source = f.options.buildRoot;
  f.options.assertLock = () => ({});
  await runManagedVerification({ ...f.options, command: ['node', 'scripts/probe.mjs'] });
  const runs = f.calls.filter((call) => typeof call[1] !== 'string');
  assert.equal(runs.length, 2);
  assert.equal(runs[0][1][0], '/tmp/managed-build/scripts/stage-build.mjs');
});

test('通用验证在固定构建根执行，自定义绝对输入输出参数原样传递', async () => {
  const f = fixture(); f.options.assertLock = () => ({ manifestDigest: 'frozen' });
  const args = ['python3', '/external/probe.py', '--input', '/external/input.json', '--report', '/external/report.json'];
  await runManagedVerification({ ...f.options, command: args });
  const runs = f.calls.filter((call) => typeof call[1] !== 'string');
  assert.equal(runs.length, 1);
  assert.equal(runs[0][0], 'python3');
  assert.deepEqual(runs[0][1], args.slice(1));
  assert.equal(runs[0][2].cwd, '/tmp/managed-build');
  assert.equal(runs[0][2].env.CARGO_TARGET_DIR, '/tmp/managed-build/target');
});
