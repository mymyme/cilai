import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, readlinkSync, realpathSync, renameSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { acquireBuildLock } from './build-lock.mjs';
import { beginStorageRun, storageEnvironment } from './storage-lifecycle.mjs';
import { runSwift } from './run-swift.mjs';

const macTest = (name, fn) => test(name, { skip: process.platform !== 'darwin' && '仅 macOS Swift 构建入口使用 /private/tmp' }, fn);
const read = path => JSON.parse(readFileSync(path, 'utf8'));
const environmentDigest = value => createHash('sha256').update(JSON.stringify(Object.entries(value).sort(([a], [b]) => a.localeCompare(b)))).digest('hex');
const quiet = { write() {} };
function fixture(t) {
  const container = realpathSync(mkdtempSync('/private/tmp/cilai-swift-test-')), root = join(container, '个人 构建');
  mkdirSync(root); const token = acquireBuildLock(root, 'swift-test');
  const state = beginStorageRun(root, 'swift-test', { measure: () => ({ totalLogicalBytes: 0, freeBytes: 200 * 1000 ** 3 }) });
  const environment = storageEnvironment(state, { ...process.env, CILAI_BUILD_LOCK_TOKEN: token });
  const aliases = new Set();
  t.after(() => { for (const alias of aliases) rmSync(alias, { recursive: true, force: true }); rmSync(container, { recursive: true, force: true }); });
  return { root, state, environment, aliases };
}
function mockStart(f, { inspect = () => {}, code = 0, signal = null, error, interrupt = false } = {}) {
  return (command, args, options) => {
    assert.equal(command, '/usr/bin/xcrun'); assert.equal(args[0], 'swiftc');
    const alias = dirname(options.env.TMPDIR); f.aliases.add(alias);
    assert.match(alias, /^\/private\/tmp\/cilai-swift-[A-Za-z0-9]+$/);
    assert.equal(readlinkSync(options.env.TMPDIR), f.state.temporary);
    assert.equal(options.env.TMP, options.env.TMPDIR); assert.equal(options.env.TEMP, options.env.TMPDIR);
    assert.equal(lstatSync(alias).mode & 0o777, 0o700);
    const child = new EventEmitter(); child.pid = process.pid; child.stdout = new PassThrough(); child.stderr = new PassThrough();
    child.kill = value => { child.killedWith = value; return true; };
    queueMicrotask(() => {
      inspect({ command, args, options, alias, child });
      if (error) child.emit('error', error);
      child.stdout.end('compiler stdout\n'); child.stderr.end('compiler stderr\n');
      if (interrupt) { process.emit('SIGTERM'); assert.equal(child.killedWith, 'SIGTERM'); }
      child.emit('close', code, signal);
    });
    return child;
  };
}
const options = (f, extra = {}) => ({ environment: f.environment, stdout: quiet, stderr: quiet, ...extra });

macTest('成功仅删除已登记ASCII别名，真实临时目录、缓存和父环境保持不变', async t => {
  const f = fixture(t), before = { ...f.environment }, parentBefore = environmentDigest(process.env);
  writeFileSync(join(f.state.temporary, 'keep'), 'real data');
  const cache = join(f.root, 'tmp/swift-module-cache'); mkdirSync(cache); writeFileSync(join(cache, 'keep'), 'cache');
  f.environment.SWIFT_MODULE_CACHE_PATH = cache; f.environment.CLANG_MODULE_CACHE_PATH = cache;
  const result = await runSwift(['--version'], options(f, { start: mockStart(f, { inspect: ({ args, options: spawnOptions, alias }) => {
    assert.deepEqual(args, ['swiftc', '--version']); assert.equal(spawnOptions.env.SWIFT_MODULE_CACHE_PATH, cache);
    assert.ok(!readFileSync(join(alias, 'owner.json'), 'utf8').includes(f.environment.CILAI_BUILD_LOCK_TOKEN));
  } }) }));
  assert.equal(result.cleanup, 'removed'); assert.equal(existsSync(result.aliasDirectory), false);
  assert.equal(readFileSync(join(f.state.temporary, 'keep'), 'utf8'), 'real data');
  assert.equal(readFileSync(join(cache, 'keep'), 'utf8'), 'cache');
  assert.equal(environmentDigest(f.environment), environmentDigest({ ...before, SWIFT_MODULE_CACHE_PATH: cache, CLANG_MODULE_CACHE_PATH: cache })); assert.equal(environmentDigest(process.env), parentBefore);
  assert.equal(readFileSync(result.stdout, 'utf8'), 'compiler stdout\n'); assert.equal(read(result.reportPath).status, 'passed');
});

macTest('缺失令牌、路径越界、错误轮次或归属不符都在启动前拒绝', async t => {
  const mutations = [
    f => { delete f.environment.CILAI_BUILD_LOCK_TOKEN; },
    f => { f.environment.CILAI_BUILD_LOCK_TOKEN = 'other-token'; },
    f => { f.environment.CILAI_BUILD_RUN_TMP = dirname(f.root); },
    f => { f.environment.CILAI_BUILD_RUN_TMP = f.root; },
    f => { const file = join(f.state.run, 'owner.json'); writeFileSync(file, JSON.stringify({ ...read(file), pid: process.pid + 1 })); },
    f => { writeFileSync(f.state.report, JSON.stringify({ ...read(f.state.report), status: 'passed' })); },
    f => { const file = join(f.root, 'target/storage-reports/index.json'); writeFileSync(file, JSON.stringify({ ...read(file), ids: [] })); },
  ];
  for (const mutate of mutations) {
    const f = fixture(t); mutate(f);
    await assert.rejects(runSwift(['--version'], options(f, { start: () => assert.fail('无效登记不能启动编译器') })));
  }
});

macTest('锁PID无效或已结束时拒绝使用旧run', async t => {
  for (const pid of [0, 2147483647]) {
    const f = fixture(t);
    for (const file of [join(f.root, '.cilai-build-lock.json'), join(f.state.run, 'owner.json'), f.state.report]) writeFileSync(file, JSON.stringify({ ...read(file), pid }));
    await assert.rejects(runSwift(['--version'], options(f, { start: () => assert.fail('失效锁不能启动编译器') })), /归属|进程/);
  }
});

macTest('登记路径或owner经过符号链接时拒绝启动', async t => {
  for (const replace of ['tmp', 'owner']) {
    const f = fixture(t), outside = join(f.root, 'outside'); mkdirSync(outside);
    if (replace === 'tmp') { rmSync(f.state.temporary, { recursive: true }); symlinkSync(outside, f.state.temporary); }
    else { const owner = join(f.state.run, 'owner.json'), saved = join(outside, 'owner.json'); renameSync(owner, saved); symlinkSync(saved, owner); }
    await assert.rejects(runSwift(['--version'], options(f, { start: () => assert.fail('链接登记不能启动编译器') })), /符号链接/);
  }
});

macTest('别名链接被替换时不删除链接或其外部目标', async t => {
  const f = fixture(t), outside = join(f.root, 'outside'); mkdirSync(outside); writeFileSync(join(outside, 'keep'), 'private');
  await assert.rejects(runSwift(['--version'], options(f, { start: mockStart(f, { inspect: ({ alias, options: spawnOptions }) => {
    unlinkSync(spawnOptions.env.TMPDIR); symlinkSync(outside, spawnOptions.env.TMPDIR);
    assert.ok(existsSync(join(alias, 'owner.json')));
  } }) })), /别名归属已改变/);
  const [alias] = f.aliases; assert.equal(readlinkSync(join(alias, 'tmp')), outside);
  assert.equal(readFileSync(join(outside, 'keep'), 'utf8'), 'private'); assert.ok(existsSync(join(alias, 'owner.json')));
});

macTest('marker被改写或外层目录被替换时保留现场', async t => {
  for (const replace of ['marker', 'directory']) {
    const f = fixture(t);
    await assert.rejects(runSwift(['--version'], options(f, { start: mockStart(f, { inspect: ({ alias }) => {
      if (replace === 'marker') writeFileSync(join(alias, 'owner.json'), '{}');
      else { const moved = `${alias}-moved`; renameSync(alias, moved); f.aliases.add(moved); mkdirSync(alias); writeFileSync(join(alias, 'keep'), 'unknown'); }
    } }) })));
    const [alias] = f.aliases; assert.ok(existsSync(alias));
    if (replace === 'marker') assert.equal(readFileSync(join(alias, 'owner.json'), 'utf8'), '{}');
    else assert.equal(readFileSync(join(alias, 'keep'), 'utf8'), 'unknown');
  }
});

macTest('别名目录出现未知条目时所有条目均保留', async t => {
  const f = fixture(t);
  await assert.rejects(runSwift(['--version'], options(f, { start: mockStart(f, { inspect: ({ alias }) => writeFileSync(join(alias, 'unknown'), 'keep') }) })), /未知条目/);
  const [alias] = f.aliases; assert.deepEqual(readdirSync(alias).sort(), ['owner.json', 'tmp', 'unknown']);
  assert.equal(readFileSync(join(alias, 'unknown'), 'utf8'), 'keep');
});

macTest('命令失败保留真实tmp与诊断，返回实际退出码', async t => {
  const f = fixture(t); writeFileSync(join(f.state.temporary, 'keep'), 'diagnostic');
  await assert.rejects(runSwift(['--version'], options(f, { start: mockStart(f, { code: 7 }) })), error => {
    assert.equal(error.exitCode, 7); const report = read(error.reportPath);
    assert.equal(report.status, 'failed'); assert.equal(report.exitCode, 7); assert.equal(report.cleanup, 'retained');
    assert.equal(readFileSync(report.stderr, 'utf8'), 'compiler stderr\n'); return true;
  });
  assert.equal(readFileSync(join(f.state.temporary, 'keep'), 'utf8'), 'diagnostic');
});

macTest('spawn错误等待close后记录失败，清除本轮信号监听器', async t => {
  const f = fixture(t), listeners = process.listenerCount('SIGTERM');
  await assert.rejects(runSwift(['--version'], options(f, { start: mockStart(f, { code: -2, error: new Error('spawn failed') }) })), error => {
    assert.equal(read(error.reportPath).error, 'spawn failed'); return true;
  });
  assert.equal(process.listenerCount('SIGTERM'), listeners);
});

macTest('中断即使随后退出0也保留别名和原run', async t => {
  const f = fixture(t);
  await assert.rejects(runSwift(['--version'], options(f, { start: mockStart(f, { interrupt: true }) })), error => {
    const report = read(error.reportPath); assert.equal(report.interrupted, true); assert.equal(report.cleanup, 'retained');
    assert.ok(existsSync(report.aliasDirectory)); return true;
  });
  assert.ok(existsSync(f.state.temporary));
});

macTest('有效中文受管目录通过真实Swift版本检查和最小编译', async t => {
  const f = fixture(t), cache = join(f.root, 'tmp/中文模块缓存'); mkdirSync(cache);
  f.environment.CLANG_MODULE_CACHE_PATH = cache; f.environment.SWIFT_MODULE_CACHE_PATH = cache;
  const version = await runSwift(['--version'], options(f)); f.aliases.add(version.aliasDirectory);
  assert.match(readFileSync(version.stdout, 'utf8'), /Swift version/);
  const source = join(f.state.temporary, 'probe.swift'), output = join(f.state.temporary, 'probe');
  writeFileSync(source, 'print("managed swift probe")\n');
  const compiled = await runSwift([source, '-o', output], options(f)); f.aliases.add(compiled.aliasDirectory);
  assert.equal(compiled.status, 'passed'); assert.ok(lstatSync(output).isFile()); assert.ok(existsSync(source));
  assert.equal(existsSync(compiled.aliasDirectory), false); assert.ok(existsSync(cache));
});
