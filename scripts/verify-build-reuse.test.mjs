import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, linkSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { acquireBuildLock, recordStagedBuild, releaseBuildLock } from './build-lock.mjs';
import { assertInstalledDependencies, parseVerifyArguments, sampleStorage, verifyBuildReuse } from './verify-build-reuse.mjs';

function fixture(t) {
  const parent = mkdtempSync(join(realpathSync(tmpdir()), 'cilai-reuse-test-'));
  const root = join(parent, 'build'), source = join(parent, 'source');
  mkdirSync(root); mkdirSync(source);
  const token = acquireBuildLock(root, 'fixture');
  t.after(() => { releaseBuildLock(root, token); rmSync(parent, { recursive: true, force: true }); });
  const calls = [];
  const runner = async (command, args, options) => {
    calls.push({ command, args, options });
    if (args[0].endsWith('stage-build.mjs')) {
      writeFileSync(join(root, 'fixture-source.txt'), 'unchanged source');
      const hash = createHash('sha256').update('unchanged source').digest('hex');
      writeFileSync(join(root, '.cilai-staged-files.json'), JSON.stringify({ version: 1, files: { 'fixture-source.txt': `file:${hash}` } }));
      recordStagedBuild(root, token);
    }
    if (args[0].endsWith('prepare-package.mjs')) {
      mkdirSync(join(root, 'target/sidecar/debug'), { recursive: true });
      writeFileSync(join(root, 'target/sidecar/debug/fixture-binary'), 'fixed output');
    }
  };
  let elapsed = 0;
  const options = { buildRoot: root, source, environment: { CILAI_BUILD_LOCK_TOKEN: token, PATH: process.env.PATH },
    runner, dependencies: () => {}, clock: () => elapsed += 100, nodeVersion: 'v24.18.0' };
  return { root, source, token, calls, runner, options };
}

test('两遍准备复用固定缓存，保留现用应用且报告不声称完整打包', async t => {
  const f = fixture(t);
  const app = join(f.root, 'target/desktop-electron-testing/mac-arm64/词来工作台.app');
  mkdirSync(app, { recursive: true }); writeFileSync(join(app, 'sentinel'), 'running-app');
  mkdirSync(join(f.root, 'target/debug'), { recursive: true });
  writeFileSync(join(f.root, 'target/debug/cache'), 'warm cache');
  const { report, output } = await verifyBuildReuse(f.options);
  assert.equal(report.passed, true);
  assert.equal(report.passes.length, 2);
  assert.equal(report.passes[0].manifestSha256, report.passes[1].manifestSha256);
  assert.equal(report.comparison.secondTargetLogicalBytesChange, 0);
  assert.equal(report.kind, 'build-preparation-reuse');
  assert.ok(report.excluded.includes('npm依赖安装'));
  assert.ok(report.excluded.includes('Electron最终组包和签名'));
  assert.equal(readFileSync(join(app, 'sentinel'), 'utf8'), 'running-app');
  assert.equal(readFileSync(join(f.root, 'target/debug/cache'), 'utf8'), 'warm cache');
  assert.equal(f.calls.length, 4);
  assert.equal(f.calls.filter(call => call.args[0].endsWith('stage-build.mjs')).length, 1);
  assert.equal(f.calls.filter(call => call.args[0].endsWith('prepare-package.mjs')).length, 2);
  for (const call of f.calls) {
    assert.equal(call.options.env.CARGO_TARGET_DIR, join(f.root, 'target'));
    assert.equal(call.options.env.CARGO_INCREMENTAL, '0');
    assert.equal(call.options.env.CILAI_BUILD_LOCK_TOKEN, f.token);
  }
  assert.equal(output, join(f.root, 'target/storage-reports/verify-build-reuse.json'));
  assert.equal(JSON.parse(readFileSync(output, 'utf8')).passed, true);
});

test('个人版环境只使用当前构建根的依赖和缓存', async t => {
  const f = fixture(t);
  await verifyBuildReuse({ ...f.options, personal: true,
    environment: { ...f.options.environment, CARGO_HOME: '/unrelated-enterprise-cache', CARGO_TARGET_DIR: '/wrong-target' } });
  for (const { options } of f.calls) {
    assert.equal(options.env.CARGO_HOME, join(f.root, 'toolchains/cargo'));
    assert.equal(options.env.npm_config_cache, join(f.root, 'cache/npm'));
    assert.equal(options.env.ELECTRON_BUILDER_CACHE, join(f.root, 'cache/electron-builder'));
    assert.equal(options.env.CARGO_TARGET_DIR, join(f.root, 'target'));
  }
});

test('准备失败留下失败报告，不继续第二遍或改动缓存', async t => {
  const f = fixture(t);
  await assert.rejects(verifyBuildReuse({ ...f.options, runner: async (...args) => {
    if (args[1][0].endsWith('prepare-package.mjs')) throw new Error('fixture compilation failed');
    return f.runner(...args);
  } }), /fixture compilation failed/);
  const report = JSON.parse(readFileSync(join(f.root, 'target/storage-reports/verify-build-reuse.json')));
  assert.equal(report.passed, false); assert.equal(report.failure.phase, 'prepare-1');
  assert.equal(report.passes.length, 1); assert.equal(report.passes[0].passed, false);
});

test('构建期间源码或清单改变会使比较失败', async t => {
  const f = fixture(t);
  await assert.rejects(verifyBuildReuse({ ...f.options, runner: async (...args) => {
    await f.runner(...args);
    if (args[1][0].endsWith('prepare-package.mjs')) writeFileSync(join(f.root, 'fixture-source.txt'), 'changed during build');
  } }), /源码被修改/);
  const report = JSON.parse(readFileSync(join(f.root, 'target/storage-reports/verify-build-reuse.json')));
  assert.equal(report.passed, false); assert.equal(report.passes.length, 1);
});

test('缺少依赖时不进行准备或自动安装', async t => {
  const f = fixture(t);
  await assert.rejects(verifyBuildReuse({ ...f.options, dependencies: assertInstalledDependencies }), /缺少已安装依赖/);
  assert.equal(f.calls.length, 1);
  const report = JSON.parse(readFileSync(join(f.root, 'target/storage-reports/verify-build-reuse.json')));
  assert.equal(report.failure.phase, 'installed-dependencies');
});

test('Node版本和锁身份错误在任何子命令前拒绝', async t => {
  const f = fixture(t);
  await assert.rejects(verifyBuildReuse({ ...f.options, nodeVersion: 'v22.0.0' }), /Node 24.18.0/);
  await assert.rejects(verifyBuildReuse({ ...f.options, environment: { CILAI_BUILD_LOCK_TOKEN: 'other' } }), /另一任务/);
  assert.equal(f.calls.length, 0);
});

test('目录统计对硬链接去重且不跟随目录链接，Windows明确无分配量', t => {
  const f = fixture(t);
  mkdirSync(join(f.root, 'target'));
  writeFileSync(join(f.root, 'target/one'), '12345');
  linkSync(join(f.root, 'target/one'), join(f.root, 'target/two'));
  writeFileSync(join(f.source, 'outside'), 'not counted');
  symlinkSync(f.source, join(f.root, 'outside-link'), process.platform === 'win32' ? 'junction' : 'dir');
  const sample = sampleStorage(f.root);
  assert.equal(sample.directories.target.logicalBytes, 5);
  assert.equal(sample.directories.target.files, 1);
  assert.equal(sample.directories.root.symbolicLinks, 1);
  assert.equal(sample.directories.temporary.exists, false);
  assert.ok(sample.filesystem.availableBytes > 0);
  assert.equal(sampleStorage(f.root, { platform: 'win32' }).directories.target.allocatedBytes, null);
});

test('报告目录链接不能把写入导向构建根外', async t => {
  const f = fixture(t);
  mkdirSync(join(f.root, 'target'));
  symlinkSync(f.source, join(f.root, 'target/storage-reports'), process.platform === 'win32' ? 'junction' : 'dir');
  await assert.rejects(verifyBuildReuse(f.options), /不能经过符号链接/);
  assert.equal(existsSync(join(f.source, 'verify-build-reuse.json')), false);
});

test('完整打包模式在同一锁内暂存一次后从构建根连续调用两次标准入口', async t => {
  for (const platform of ['darwin', 'win32']) await t.test(platform, async t => {
    const f = fixture(t), samples = [];
    mkdirSync(join(f.root, 'target/storage-reports'), { recursive: true });
    const prepareReport = join(f.root, 'target/storage-reports/verify-build-reuse.json');
    writeFileSync(prepareReport, 'previous preparation report');
    const { report, output } = await verifyBuildReuse({ ...f.options, mode: 'package', platform,
      dependencies: () => assert.fail('标准打包负责依赖校验和安装，不应提前阻止它修复缺失依赖'),
      sampler: root => { samples.push(root); return sampleStorage(root, { platform }); },
    });
    assert.equal(report.passed, true); assert.equal(report.kind, 'full-package-reuse');
    assert.equal(report.platform, platform); assert.equal(report.passes.length, 2);
    assert.deepEqual(samples, [f.root, f.root, f.root]);
    assert.equal(f.calls.length, 4);
    assert.equal(f.calls.filter(call => call.args[0].endsWith('stage-build.mjs')).length, 1);
    assert.equal(f.calls.filter(call => call.args[0].endsWith('check-toolchain.mjs')).length, 1);
    for (const call of f.calls.slice(2)) {
      assert.equal(call.command, platform === 'darwin' ? 'bash' : 'powershell.exe');
      assert.deepEqual(call.args, platform === 'darwin'
        ? [join(f.root, 'script/package.sh')]
        : ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(f.root, 'script/package.ps1')]);
      assert.equal(call.options.cwd, f.root);
      assert.equal(call.options.env.CILAI_BUILD_LOCK_TOKEN, f.token);
      assert.equal(call.options.env.CARGO_TARGET_DIR, join(f.root, 'target'));
    }
    assert.equal(report.passes[0].manifestSha256, report.passes[1].manifestSha256);
    assert.ok(report.included.includes('标准依赖校验及安装'));
    assert.ok(report.included.includes('Electron最终组包和本地测试签名'));
    assert.ok(report.included.includes('应用启动自检'));
    assert.ok(report.excluded.includes('常驻应用启动'));
    assert.equal(output, join(f.root, 'target/storage-reports/verify-package-reuse.json'));
    assert.equal(readFileSync(prepareReport, 'utf8'), 'previous preparation report');
    assert.doesNotMatch(report.comparison.interpretation, /不代表完整打包/);
  });
});

test('标准打包拒绝现用包时立即停止，保留应用及独立失败报告', async t => {
  const f = fixture(t), app = join(f.root, 'target/desktop-electron-testing/mac-arm64/词来工作台.app');
  mkdirSync(app, { recursive: true }); writeFileSync(join(app, 'sentinel'), 'running app');
  let packageCalls = 0;
  await assert.rejects(verifyBuildReuse({ ...f.options, mode: 'package', platform: 'darwin', runner: async (...args) => {
    if (args[1][0].endsWith('package.sh')) { packageCalls += 1; throw new Error('统一测试包仍有随包进程运行'); }
    return f.runner(...args);
  } }), /仍有随包进程运行/);
  assert.equal(packageCalls, 1);
  assert.equal(readFileSync(join(app, 'sentinel'), 'utf8'), 'running app');
  const report = JSON.parse(readFileSync(join(f.root, 'target/storage-reports/verify-package-reuse.json')));
  assert.equal(report.failure.phase, 'package-1'); assert.equal(report.passes.length, 1);
  assert.equal(report.passes[0].passed, false); assert.equal(report.passes[0].elapsedMs, 100);
  assert.equal(existsSync(join(f.root, 'target/storage-reports/verify-build-reuse.json')), false);
});

test('完整打包中源码改变即终止比较，不执行第二遍', async t => {
  const f = fixture(t); let packageCalls = 0;
  await assert.rejects(verifyBuildReuse({ ...f.options, mode: 'package', platform: 'darwin', runner: async (...args) => {
    await f.runner(...args);
    if (args[1][0].endsWith('package.sh')) { packageCalls += 1; writeFileSync(join(f.root, 'fixture-source.txt'), 'mutated'); }
  } }), /源码被修改/);
  assert.equal(packageCalls, 1);
  const report = JSON.parse(readFileSync(join(f.root, 'target/storage-reports/verify-package-reuse.json')));
  assert.equal(report.passed, false); assert.equal(report.failure.phase, 'package-1');
});

test('命令行只接受空参数或一次package开关，未知模式和平台在执行前拒绝', async t => {
  assert.equal(parseVerifyArguments([]), 'prepare'); assert.equal(parseVerifyArguments(['--package']), 'package');
  for (const args of [['--launch'], ['--package', '--launch'], ['--package', '--package'], ['/other/root'], ['--clean']]) {
    assert.throws(() => parseVerifyArguments(args), /只接受 --package/);
  }
  const f = fixture(t);
  await assert.rejects(verifyBuildReuse({ ...f.options, mode: 'other' }), /验证模式/);
  await assert.rejects(verifyBuildReuse({ ...f.options, platform: 'linux' }), /构建平台/);
  assert.equal(f.calls.length, 0);
});
