import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { existsSync, linkSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertManagedPath, beginStorageRun, directoryBytes, finishStorageRun, launchEnvironment, launchPackagedMacApp, openProcesses, storageSnapshot, storageEnvironment } from './storage-lifecycle.mjs';

const GB = 1000 ** 3;
const sample = (total = 0, free = 200 * GB) => ({ totalLogicalBytes: total, freeBytes: free, directoryLogicalBytes: {}, at: new Date().toISOString() });
function fixture(t) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'cilai-storage-lifecycle-test-')));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  t.mock.method(console, 'log', () => {}); t.mock.method(console, 'warn', () => {});
  return root;
}
const begin = (root) => beginStorageRun(root, '/bin/build', { measure: () => sample() });
const finish = (state, status = 0, extra = {}) => finishStorageRun(state, status, { measure: () => sample(), inspect: () => [], ...extra });
const read = (path) => JSON.parse(readFileSync(path, 'utf8'));

test('成功报告先保存，再检查整个run占用并清理临时目录', (t) => {
  const root = fixture(t), state = begin(root); writeFileSync(join(state.temporary, 'result.tmp'), 'result');
  let inspected = false;
  const result = finish(state, 0, { inspect: (path) => {
    inspected = true; assert.equal(path, state.run); assert.ok(existsSync(state.temporary));
    const report = read(state.report); assert.equal(report.status, 'passed'); assert.equal(report.cleanup, 'checking');
    return [];
  } });
  assert.ok(inspected); assert.equal(existsSync(state.run), false);
  assert.equal(read(state.report).cleanup, 'removed'); assert.equal(result.temporaryBytes, 6);
});

test('失败构建保留本轮临时目录和完整失败报告，不执行占用清理', (t) => {
  const root = fixture(t), state = begin(root); writeFileSync(join(state.temporary, 'failure.log'), 'evidence');
  const result = finish(state, 7, { inspect: () => assert.fail('失败构建不应尝试清理') });
  assert.equal(result.status, 'failed'); assert.equal(result.cleanup, 'retained-failure');
  assert.equal(readFileSync(join(state.temporary, 'failure.log'), 'utf8'), 'evidence');
  assert.equal(read(state.report).status, 'failed');
});

test('活跃进程保留且占用检查失败明确标为needs-review', (t) => {
  const root = fixture(t), active = begin(root);
  assert.equal(finish(active, 0, { inspect: () => [123, 456] }).cleanup, 'retained-active');
  assert.ok(existsSync(active.run)); assert.deepEqual(read(active.report).activePids, [123, 456]);
  const unknown = begin(root);
  assert.equal(finish(unknown, 0, { inspect: () => { throw new Error('lsof unavailable'); } }).cleanup, 'needs-review');
  assert.ok(existsSync(unknown.run)); assert.match(read(unknown.report).cleanupError, /lsof unavailable/);
});

test('持久缓存、未知临时目录和其他失败轮次均不删除', (t) => {
  const root = fixture(t);
  for (const name of ['cache', 'toolchains', 'node_modules', 'target/debug', 'tmp/legacy', 'tmp/storage-runs/unregistered']) {
    mkdirSync(join(root, name), { recursive: true }); writeFileSync(join(root, name, 'keep'), name);
  }
  const failed = begin(root); finish(failed, 1);
  const state = begin(root); writeFileSync(join(state.temporary, 'remove'), 'temporary'); finish(state);
  for (const name of ['cache', 'toolchains', 'node_modules', 'target/debug', 'tmp/legacy', 'tmp/storage-runs/unregistered']) assert.equal(readFileSync(join(root, name, 'keep'), 'utf8'), name);
  assert.ok(existsSync(failed.run));
});

test('同名未登记报告即使JSON损坏也不解析或删除', (t) => {
  const root = fixture(t), first = begin(root), unknown = join(root, 'target/storage-reports', `build-${randomUUID()}.json`);
  writeFileSync(unknown, 'not JSON and belongs to another task'); finish(first);
  const state = begin(root); finish(state);
  assert.equal(readFileSync(unknown, 'utf8'), 'not JSON and belongs to another task');
});

test('路径越界、祖先符号链接和temporary路径篡改拒绝清理', (t) => {
  const root = fixture(t), outside = realpathSync(mkdtempSync(join(tmpdir(), 'cilai-storage-outside-')));
  t.after(() => rmSync(outside, { recursive: true, force: true }));
  writeFileSync(join(outside, 'keep'), 'private');
  assert.throws(() => assertManagedPath(root, outside), /越界/);
  assert.throws(() => assertManagedPath(root, root), /越界/);
  symlinkSync(outside, join(root, 'tmp')); assert.throws(() => begin(root), /符号链接/); rmSync(join(root, 'tmp'));
  const state = begin(root); state.temporary = outside;
  assert.throws(() => finish(state), /空间登记无效/);
  assert.ok(existsSync(state.run)); assert.equal(readFileSync(join(outside, 'keep'), 'utf8'), 'private');
});

test('owner和report符号链接不读取，run内未知内容不删除', (t) => {
  const root = fixture(t), state = begin(root), owner = join(state.run, 'owner.json'), outside = join(root, 'private-data');
  writeFileSync(outside, 'private, not JSON'); rmSync(owner); symlinkSync(outside, owner);
  assert.throws(() => finish(state), /符号链接/); assert.ok(existsSync(state.run));
  const second = begin(root); rmSync(second.report); symlinkSync(outside, second.report);
  assert.throws(() => finish(second), /符号链接/); assert.equal(readFileSync(outside, 'utf8'), 'private, not JSON');
  const third = begin(root); writeFileSync(join(third.run, 'unregistered-important'), 'keep');
  assert.throws(() => finish(third), /未登记内容/); assert.ok(existsSync(third.run));
});

test('临时目录中的外部链接仅删除链接，链接目标不参与大小统计或删除', (t) => {
  const root = fixture(t), external = join(root, 'keep-source'); mkdirSync(external); writeFileSync(join(external, 'data'), 'keep');
  const state = begin(root); symlinkSync(external, join(state.temporary, 'external'));
  assert.equal(directoryBytes(state.temporary), 0); finish(state);
  assert.equal(readFileSync(join(external, 'data'), 'utf8'), 'keep');
});

test('重复成功构建不累积临时数据，成功报告最多保留30份', (t) => {
  const root = fixture(t), failed = begin(root); finish(failed, 1);
  const unknown = join(root, 'target/storage-reports', `build-${randomUUID()}.json`); writeFileSync(unknown, 'unregistered');
  for (let i = 0; i < 34; i++) {
    const state = begin(root); writeFileSync(join(state.temporary, 'payload'), Buffer.alloc(1024)); finish(state);
    assert.equal(existsSync(state.run), false);
  }
  const index = read(join(root, 'target/storage-reports/index.json'));
  assert.equal(index.ids.length, 31);
  assert.equal(index.ids.map((id) => read(join(root, 'target/storage-reports', `build-${id}.json`))).filter((item) => item.status === 'passed').length, 30);
  assert.deepEqual(readdirSync(join(root, 'tmp/storage-runs')), [failed.id]);
  assert.ok(existsSync(failed.report)); assert.ok(existsSync(unknown));
});

test('20GB增长与临时量阈值使用mock，无需实际占用磁盘', (t) => {
  const root = fixture(t);
  const state = beginStorageRun(root, 'build', { measure: () => sample(10 * GB) });
  const result = finish(state, 1, { measure: () => sample(31 * GB, 90 * GB), measureDirectory: () => 21 * GB });
  assert.equal(result.directoryGrowthBytes, 21 * GB); assert.equal(result.dailyGrowthBytes, 21 * GB);
  assert.ok(result.warnings.includes('登记的临时数据超过 20 GB')); assert.ok(result.warnings.includes('今日构建目录增长超过 20 GB'));
  assert.ok(result.warnings.includes('磁盘可用空间低于 100 GB'));
  const second = beginStorageRun(root, 'build', { measure: () => sample(31 * GB) });
  const next = finish(second, 1, { measure: () => sample(32 * GB), measureDirectory: () => 20 * GB });
  assert.equal(next.dailyGrowthBytes, 22 * GB); assert.ok(!next.warnings.includes('登记的临时数据超过 20 GB'));
});

test('lsof退出状态、告警及进程号严格验证，Windows明确缺少句柄检查', () => {
  const path = '/controlled/run';
  assert.deepEqual(openProcesses(path, (cmd, args) => { assert.equal(cmd, 'lsof'); assert.deepEqual(args, ['-nP', '-t', '+D', path]); return { status: 1, stdout: '', stderr: '' }; }, 'darwin'), []);
  assert.deepEqual(openProcesses(path, () => ({ status: 0, stdout: '12\n12\n34\n', stderr: '' }), 'linux'), [12, 34]);
  for (const result of [{ status: 1, stdout: '', stderr: 'permission denied' }, { status: 2, stdout: '', stderr: '' }, { status: 0, stdout: 'not-a-pid', stderr: '' }, { status: 0, stdout: '0', stderr: '' }, { error: new Error('missing') }]) assert.throws(() => openProcesses(path, () => result, 'darwin'), /检查|进程/);
  assert.throws(() => openProcesses(path, () => assert.fail('不得将进程命令行当句柄检查'), 'win32'), /Windows.*句柄检查/);
});

test('Windows检查不足在报告中显示needs-review，构建结果与清理状态分开', (t) => {
  const root = fixture(t), state = begin(root);
  const result = finish(state, 0, { inspect: (path) => openProcesses(path, () => {}, 'win32') });
  assert.equal(result.status, 'passed'); assert.equal(result.cleanup, 'needs-review'); assert.ok(existsSync(state.run));
});

test('环境变量仅把本轮临时数据导向受控目录，缓存路径仍指向长期构建根', (t) => {
  const state = begin(fixture(t));
  const env = storageEnvironment(state, { PRESERVED: 'yes', CARGO_HOME: '/cache/cargo' });
  assert.equal(env.PRESERVED, 'yes'); assert.equal(env.CARGO_HOME, '/cache/cargo');
  assert.equal(env.TMPDIR, state.temporary); assert.equal(env.TEMP, state.temporary);
  assert.equal(env.CARGO_TARGET_DIR, join(state.root, 'target')); assert.equal(env.CARGO_INCREMENTAL, '0');
});

test('硬链接只计一次，目录大小检查不跟随符号链接', (t) => {
  const root = fixture(t); writeFileSync(join(root, 'a'), '1234'); linkSync(join(root, 'a'), join(root, 'b')); symlinkSync('a', join(root, 'c'));
  assert.equal(directoryBytes(root), 4);
});

test('常驻应用恢复构建前临时环境，区分未设置和空值且不改变构建环境', (t) => {
  const state = begin(fixture(t));
  const original = { TMPDIR: '/original path/带空格', TMP: '', CARGO_HOME: '/cache/cargo', CILAI_BUILD_LOCK_TOKEN: 'build-token' };
  const build = storageEnvironment(state, original), launch = launchEnvironment(build);
  assert.equal(launch.TMPDIR, original.TMPDIR); assert.equal(launch.TMP, '');
  assert.equal(Object.hasOwn(launch, 'TEMP'), false);
  assert.equal(launch.CARGO_HOME, original.CARGO_HOME);
  assert.equal(Object.hasOwn(launch, 'CILAI_BUILD_RUN_TMP'), false);
  assert.equal(Object.hasOwn(launch, 'CILAI_BUILD_ORIGINAL_TEMP_ENV'), false);
  assert.equal(Object.hasOwn(launch, 'CILAI_BUILD_LOCK_TOKEN'), false);
  assert.equal(build.CILAI_BUILD_LOCK_TOKEN, 'build-token');
  assert.equal(build.TMPDIR, state.temporary);
  const nested = storageEnvironment({ ...state, temporary: join(state.temporary, 'nested') }, build);
  assert.deepEqual(launchEnvironment(nested), launch);
});

test('临时环境记录缺失或损坏时拒绝将本轮路径交给常驻应用', () => {
  assert.throws(() => launchEnvironment({ CILAI_BUILD_RUN_TMP: '/owned/tmp' }), /缺少/);
  for (const value of ['not-json', '{}', 'null', '{"TMPDIR":null,"TMP":null,"TEMP":12}']) {
    assert.throws(() => launchEnvironment({ CILAI_BUILD_ORIGINAL_TEMP_ENV: value }), /无效/);
  }
});

test('macOS open仅收到恢复后的临时环境，构建目录清理不改变其启动参数', (t) => {
  const state = begin(fixture(t)), build = storageEnvironment(state, { TMPDIR: '/stable/system/tmp', PATH: '/usr/bin' });
  let called = false;
  launchPackagedMacApp('/Applications/词来个人工作台.app', build, (command, args, options) => {
    called = true; assert.equal(command, '/usr/bin/open');
    assert.deepEqual(args, ['/Applications/词来个人工作台.app']);
    assert.equal(options.env.TMPDIR, '/stable/system/tmp');
    assert.equal(Object.hasOwn(options.env, 'TEMP'), false);
    assert.equal(Object.hasOwn(options.env, 'CILAI_BUILD_RUN_TMP'), false);
    return { status: 0 };
  });
  assert.ok(called); finish(state); assert.equal(existsSync(state.temporary), false);
  assert.throws(() => launchPackagedMacApp('/Applications/test.app', build, () => ({ status: 1 })), /无法打开/);
});

test('Windows启动环境JSON可跨进程恢复原TMP和TEMP，正常启动入口先恢复再Start-Process', (t) => {
  const state = begin(fixture(t));
  const environment = storageEnvironment(state, { ...launchEnvironment(process.env), TMP: 'D:\\personal temp', TEMP: 'D:\\personal temp' });
  const result = spawnSync(process.execPath, [fileURLToPath(new URL('./storage-lifecycle.mjs', import.meta.url)), '--launch-temp-environment-json'], { env: environment, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(result.stdout).TMP, 'D:\\personal temp');
  assert.equal(JSON.parse(result.stdout).TEMP, 'D:\\personal temp');
  const script = readFileSync(new URL('../script/package.ps1', import.meta.url), 'utf8');
  const launch = script.slice(script.indexOf('if ($Launch) {', script.indexOf('$ExitDeadline')));
  assert.ok(launch.indexOf('SetEnvironmentVariable($Name') >= 0);
  assert.ok(launch.indexOf('SetEnvironmentVariable($Name') < launch.indexOf('Start-Process -FilePath $Executable'));
  assert.ok(launch.indexOf('if ($LASTEXITCODE -ne 0)') < launch.indexOf('ConvertFrom-Json'));
});


test('滚动删除旧报告后仍保留当日最早空间基线', (t) => {
  const root = fixture(t);
  let result;
  for (let i = 0; i < 32; i++) {
    const state = beginStorageRun(root, 'build', { measure: () => sample(i * GB) });
    result = finish(state, 0, { measure: () => sample((i + 1) * GB) });
  }
  assert.equal(result.dailyGrowthBytes, 32 * GB);
  assert.ok(result.warnings.includes('今日构建目录增长超过 20 GB'));
});

test('目录统计失败时已保存构建结果并保留待检查状态', (t) => {
  const state = begin(fixture(t));
  assert.throws(() => finish(state, 7, { measureDirectory: () => { throw new Error('cannot measure'); } }), /cannot measure/);
  assert.ok(existsSync(state.run)); assert.equal(read(state.report).status, 'failed');
  assert.equal(read(state.report).cleanup, 'needs-review'); assert.equal(read(state.report).exitCode, 7);
});


test('例行采样首次扫描后复用缓存，两次复用均不扫描目录', (t) => {
  const root = fixture(t); let scans = 0;
  const options = { statfs: () => ({ bavail: 200 * GB, bsize: 1 }), now: () => new Date('2026-10-25T12:00:00+08:00'), measureDirectory: () => { scans++; return 5; } };
  const first = storageSnapshot(root, options);
  assert.equal(first.inventoryFresh, true); assert.equal(first.inventoryReason, 'first-sample'); assert.equal(scans, 8);
  const cache = join(root, 'target/storage-reports/storage-inventory.json'), original = readFileSync(cache, 'utf8');
  const second = storageSnapshot(root, options), third = storageSnapshot(root, options);
  assert.equal(second.inventoryFresh, false); assert.equal(third.inventoryFresh, false); assert.equal(scans, 8);
  assert.equal(second.inventoryAt, first.inventoryAt); assert.equal(readFileSync(cache, 'utf8'), original);
});

test('日期变化或距上次扫描可用空间下降至少20GB触发全目录扫描', (t) => {
  const root = fixture(t); let now = new Date('2026-10-25T12:00:00+08:00'), available = 200 * GB, scans = 0;
  const options = { statfs: () => ({ bavail: available, bsize: 1 }), now: () => now, measureDirectory: () => { scans++; return 7; } };
  storageSnapshot(root, options);
  now = new Date('2026-10-26T12:00:00+08:00'); assert.equal(storageSnapshot(root, options).inventoryReason, 'date-changed'); assert.equal(scans, 16);
  available -= 20 * GB - 1; assert.equal(storageSnapshot(root, options).inventoryFresh, false); assert.equal(scans, 16);
  available--; assert.equal(storageSnapshot(root, options).inventoryReason, 'available-space-drop'); assert.equal(scans, 24);
});

test('未知采样缓存不解析为本工具数据且不覆盖，链接缓存也拒绝', (t) => {
  const root = fixture(t), folder = join(root, 'target/storage-reports'); mkdirSync(folder, { recursive: true });
  const cache = join(folder, 'storage-inventory.json'), text = JSON.stringify({ schema: 'another-tool', important: 'keep' }); writeFileSync(cache, text);
  assert.throws(() => storageSnapshot(root), /归属或结构无效/); assert.equal(readFileSync(cache, 'utf8'), text);
  rmSync(cache); const outside = join(root, 'unmanaged'); writeFileSync(outside, 'keep'); symlinkSync(outside, cache);
  assert.throws(() => storageSnapshot(root), /符号链接/); assert.equal(readFileSync(outside, 'utf8'), 'keep');
});

test('扫描失败不产生新缓存，也不会用不完整数字覆盖旧缓存', (t) => {
  const root = fixture(t), cache = join(root, 'target/storage-reports/storage-inventory.json');
  const options = { statfs: () => ({ bavail: 200 * GB, bsize: 1 }), now: () => new Date('2026-10-25T12:00:00+08:00'), measureDirectory: () => { throw new Error('cannot scan'); } };
  assert.throws(() => storageSnapshot(root, options), /cannot scan/); assert.equal(existsSync(cache), false);
  storageSnapshot(root, { ...options, measureDirectory: () => 5 }); const original = readFileSync(cache, 'utf8');
  assert.throws(() => storageSnapshot(root, { ...options, now: () => new Date('2026-10-26T12:00:00+08:00') }), /cannot scan/);
  assert.equal(readFileSync(cache, 'utf8'), original);
});

test('缓存目录值不冒充本轮精确增量，每日最近实测值附带采样时间', (t) => {
  const root = fixture(t), at = new Date().toISOString();
  const before = { ...sample(10 * GB), inventoryAt: at, inventoryFresh: true };
  const state = beginStorageRun(root, 'build', { measure: () => before });
  const result = finish(state, 0, { measure: () => ({ ...before, at: new Date().toISOString(), inventoryFresh: false, freeBytes: 190 * GB }) });
  assert.equal(result.directoryGrowthBytes, null); assert.equal(result.directoryGrowthBasis, 'inventory-reused');
  assert.equal(result.dailyGrowthBytes, 0); assert.equal(result.dailyGrowthInventoryAt, at); assert.equal(result.dailyGrowthFresh, false);
  assert.equal(result.diskFreeChangeBytes, -10 * GB);
});

test('完整采样跳过顶层链接但不跟随祖先链接逃逸', (t) => {
  const root = fixture(t), external = join(root, 'external'); mkdirSync(external); writeFileSync(join(external, 'keep'), 'private');
  symlinkSync(external, join(root, 'node_modules'));
  const result = storageSnapshot(root); assert.equal(result.directoryLogicalBytes.node_modules, 0);
  const other = fixture(t); symlinkSync(external, join(other, 'apps'));
  assert.throws(() => storageSnapshot(other), /符号链接/);
});


test('合法v1采样显式升级刷新，旧schema字段异常则保留原文件并拒绝覆盖', (t) => {
  const root = fixture(t), folder = join(root, 'target/storage-reports'); mkdirSync(folder, { recursive: true });
  const cache = join(folder, 'storage-inventory.json');
  const names = ['target', 'tmp', 'verification', 'cache', 'toolchains', 'node_modules', 'apps/desktop/node_modules'];
  const old = { schema: 'cilai-build-storage-inventory-v1', root, at: '2026-10-25T04:00:00.000Z', availableBytes: 200 * GB, directoryLogicalBytes: Object.fromEntries(names.map((name) => [name, 1])), totalLogicalBytes: 7 };
  writeFileSync(cache, JSON.stringify(old)); let scans = 0;
  const options = { now: () => new Date(old.at), statfs: () => ({ bavail: 200 * GB, bsize: 1 }), measureDirectory: () => { scans++; return 2; } };
  const result = storageSnapshot(root, options);
  assert.equal(result.inventoryFresh, true); assert.equal(result.inventoryReason, 'schema-upgrade'); assert.equal(scans, 8);
  assert.equal(result.directoryLogicalBytes['target-runtime-audit'], 2); assert.equal(read(cache).schema, 'cilai-build-storage-inventory-v2');
  delete old.directoryLogicalBytes.tmp; const invalid = JSON.stringify(old); writeFileSync(cache, invalid);
  assert.throws(() => storageSnapshot(root, options), /归属或结构无效/); assert.equal(readFileSync(cache, 'utf8'), invalid);
});

test('运行时审查根纳入完整采样，根内cache、sidecar和fixture均不删除', (t) => {
  const root = fixture(t), audit = join(root, 'target-runtime-audit');
  for (const name of ['cargo-home-runtime', 'sidecar', 'fixtures/official-responses-keep']) {
    mkdirSync(join(audit, name), { recursive: true }); writeFileSync(join(audit, name, 'keep'), 'keep');
  }
  const snapshot = storageSnapshot(root); assert.equal(snapshot.directoryLogicalBytes['target-runtime-audit'], 12);
  const state = begin(root); const result = finish(state);
  assert.equal(result.runtimeAuditFixtureBytes, 4);
  for (const name of ['cargo-home-runtime', 'sidecar', 'fixtures/official-responses-keep']) assert.equal(readFileSync(join(audit, name, 'keep'), 'utf8'), 'keep');
  const absent = fixture(t); assert.equal(storageSnapshot(absent).directoryLogicalBytes['target-runtime-audit'], 0);
});

test('运行时fixture超过20GB仅报告提醒，未知或活跃fixture保持原状', (t) => {
  const root = fixture(t), path = join(root, 'target-runtime-audit/fixtures/official-responses-active');
  mkdirSync(path, { recursive: true }); writeFileSync(join(path, 'keep'), 'evidence');
  const state = begin(root);
  const result = finish(state, 0, { measureDirectory: (candidate) => candidate === join(root, 'target-runtime-audit/fixtures') ? 21 * GB : directoryBytes(candidate) });
  assert.equal(result.runtimeAuditFixtureBytes, 21 * GB); assert.ok(result.runtimeAuditFixtureMeasuredAt);
  assert.ok(result.warnings.includes('运行时验证夹具超过 20 GB，需核对活动任务与失败证据'));
  assert.equal(readFileSync(join(path, 'keep'), 'utf8'), 'evidence');
});
