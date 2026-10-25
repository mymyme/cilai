import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, realpathSync, existsSync, symlinkSync, lstatSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { beginStorageRun, storageEnvironment } from '../../../scripts/storage-lifecycle.mjs';
import { verifyPersonalSmoke } from './verify-personal-smoke.mjs';

const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==', 'base64');
function fixture(t) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'cilai-personal-smoke-')));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const state = beginStorageRun(root, 'package', { measure: () => ({ totalLogicalBytes: 0, freeBytes: 1000 }) });
  const output = join(root, 'target/desktop-electron-testing'); mkdirSync(output, { recursive: true });
  const screenshot = join(state.temporary, 'startup.png'), log = join(output, 'headless-check.stdout.log');
  writeFileSync(screenshot, png);
  const value = { result: 'passed', platform: process.platform, arch: process.arch, memoryEnabled: false,
    ui: { navigation: '工作台,对话,项目,文件,自动化,技能,应用'.split(','), login: false, alerts: [] }, screenshot };
  const writeLog = () => writeFileSync(log, 'before\r\n[personal-check] ' + JSON.stringify(value) + '\r\nafter\r\n');
  writeLog();
  return { root, state, screenshot, log, value, writeLog, target: join(output, 'headless-check.png'), environment: storageEnvironment(state, {}) };
}
function unchanged(f, operation, match) {
  const before = readFileSync(f.log, 'utf8');
  assert.throws(operation, match);
  assert.equal(readFileSync(f.log, 'utf8'), before);
  assert.equal(existsSync(f.target), false);
  assert.ok(existsSync(f.screenshot));
}

test('已登记构建保留截图并更新唯一回执，回收临时目录后可以再次验证', t => {
  const f = fixture(t), result = verifyPersonalSmoke(f.log, { environment: f.environment });
  assert.equal(result.archived, true); assert.equal(result.screenshot, f.target);
  assert.equal(result.screenshotSha256, createHash('sha256').update(png).digest('hex'));
  assert.deepEqual(readFileSync(f.target), png); assert.ok(existsSync(f.screenshot));
  const log = readFileSync(f.log, 'utf8');
  assert.ok(log.startsWith('before\r\n[personal-check] ')); assert.ok(log.endsWith('\r\nafter\r\n'));
  assert.equal(log.split('[personal-check] ').length, 2);
  rmSync(f.state.run, { recursive: true });
  assert.equal(verifyPersonalSmoke(f.log, { environment: {} }).archived, true);
  assert.equal(verifyPersonalSmoke(f.log, { environment: f.environment }).archived, true);
  assert.equal(readFileSync(f.log, 'utf8'), log);
});

test('没有构建环境时只验证，不复制或改写日志', t => {
  const f = fixture(t), original = readFileSync(f.log, 'utf8');
  assert.equal(verifyPersonalSmoke(f.log, { environment: {} }).archived, false);
  assert.equal(readFileSync(f.log, 'utf8'), original); assert.equal(existsSync(f.target), false);
});

test('截图越出本轮临时目录时拒绝归档', t => {
  const f = fixture(t), outside = join(f.root, 'outside.png'); writeFileSync(outside, png);
  f.value.screenshot = outside; f.writeLog();
  unchanged(f, () => verifyPersonalSmoke(f.log, { environment: f.environment }), /越界/);
});

test('登记元数据不匹配时拒绝归档', t => {
  const f = fixture(t), owner = join(f.state.run, 'owner.json');
  const value = JSON.parse(readFileSync(owner, 'utf8')); value.root = '/unrelated'; writeFileSync(owner, JSON.stringify(value));
  unchanged(f, () => verifyPersonalSmoke(f.log, { environment: f.environment }), /有效登记/);
});

test('损坏截图和摘要不符均保留源日志和截图', t => {
  const f = fixture(t), broken = Buffer.from(png); broken[29] ^= 1; writeFileSync(f.screenshot, broken);
  unchanged(f, () => verifyPersonalSmoke(f.log, { environment: f.environment }), /截图校验失败/);
  writeFileSync(f.screenshot, png); f.value.screenshotSha256 = '0'.repeat(64); f.writeLog();
  unchanged(f, () => verifyPersonalSmoke(f.log, { environment: f.environment }), /摘要不一致/);
});

test('业务结果失败或进程退出失败时不修改源日志', t => {
  const f = fixture(t);
  unchanged(f, () => verifyPersonalSmoke(f.log, { environment: f.environment, exitCode: 1 }), /未成功退出/);
  f.value.result = 'failed'; f.writeLog();
  unchanged(f, () => verifyPersonalSmoke(f.log, { environment: f.environment }), /结果不完整/);
});

test('多个回执不归档且保持原始日志', t => {
  const f = fixture(t); writeFileSync(f.log, readFileSync(f.log, 'utf8') + '[personal-check] ' + JSON.stringify(f.value) + '\n');
  unchanged(f, () => verifyPersonalSmoke(f.log, { environment: f.environment }), /不唯一/);
});

test('截图链接或归档目标链接均拒绝处理', { skip: process.platform === 'win32' }, t => {
  const f = fixture(t), outside = join(f.root, 'outside.png'); writeFileSync(outside, png);
  rmSync(f.screenshot); symlinkSync(outside, f.screenshot);
  unchanged(f, () => verifyPersonalSmoke(f.log, { environment: f.environment }), /普通文件/);
  rmSync(f.screenshot); writeFileSync(f.screenshot, png); symlinkSync(outside, f.target);
  const before = readFileSync(f.log, 'utf8');
  assert.throws(() => verifyPersonalSmoke(f.log, { environment: f.environment }), /普通文件/);
  assert.equal(readFileSync(f.log, 'utf8'), before); assert.equal(lstatSync(f.target).isSymbolicLink(), true);
  assert.deepEqual(readFileSync(outside), png);
});

test('已归档截图被损坏时再次验证明确失败且不改日志', t => {
  const f = fixture(t); verifyPersonalSmoke(f.log, { environment: f.environment });
  const before = readFileSync(f.log, 'utf8'); writeFileSync(f.target, 'damaged');
  assert.throws(() => verifyPersonalSmoke(f.log, { environment: {} }), /不是 PNG/);
  assert.equal(readFileSync(f.log, 'utf8'), before);
});
