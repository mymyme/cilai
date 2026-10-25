import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, rmSync, symlinkSync, realpathSync, existsSync, chmodSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createVerifiedBackup, verifyBackup, pruneBackups, runCli } from './verified-backups.mjs';

function fixture(t) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'cilai-verified-backup-test-')));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const source = join(root, 'source'), backupRoot = join(root, 'backups');
  mkdirSync(source); mkdirSync(join(source, 'empty')); writeFileSync(join(source, 'a.txt'), 'original');
  const options = { source, backupRoot, dataset: 'test-data', sourceGuard: { assertHeld: () => ({ source, exclusive: true }) } };
  return { root, source, backupRoot, options };
}
function registry(root) { return JSON.parse(readFileSync(join(root, '.verified-backups.json'), 'utf8')); }
function contents(root) {
  return readdirSync(root, { recursive: true }).sort().map((x) => { const path = join(root, x); return [x, statSync(path).isFile() ? readFileSync(path).toString('base64') : 'directory']; });
}
function unpin(root, id) {
  const reg = registry(root); reg.backups.find((x) => x.id === id).pinned = false;
  writeFileSync(join(root, '.verified-backups.json'), JSON.stringify(reg));
  const path = join(root, id, 'receipt.json'), receipt = JSON.parse(readFileSync(path, 'utf8'));
  receipt.pinned = false; writeFileSync(path, JSON.stringify(receipt));
}

test('完整备份核对内容、执行权限、空目录和内部符号链接', (t) => {
  const f = fixture(t); chmodSync(join(f.source, 'a.txt'), 0o700);
  symlinkSync('a.txt', join(f.source, 'internal-link'));
  const result = createVerifiedBackup(f.options);
  assert.equal(result.status, 'verified'); assert.equal(result.files, 1); assert.equal(result.bytes, 8);
  assert.equal(statSync(join(result.path, 'data/a.txt')).mode & 0o777, 0o700);
  assert.ok(existsSync(join(result.path, 'data/empty')));
  assert.equal(verifyBackup({ ...f.options, id: result.id }).status, 'verified');
});

test('同一dataset成功后仅保留最新两份，其他dataset与未登记legacy保留', (t) => {
  const f = fixture(t); mkdirSync(f.backupRoot); mkdirSync(join(f.backupRoot, 'legacy'));
  writeFileSync(join(f.backupRoot, 'legacy/restore.db'), 'keep');
  const other = createVerifiedBackup({ ...f.options, dataset: 'other' });
  const first = createVerifiedBackup(f.options);
  writeFileSync(join(f.source, 'a.txt'), 'second'); const second = createVerifiedBackup(f.options);
  writeFileSync(join(f.source, 'a.txt'), 'third'); const third = createVerifiedBackup(f.options);
  assert.deepEqual(third.retention.deleted, [first.id]);
  assert.equal(existsSync(first.path), false); assert.ok(existsSync(second.path)); assert.ok(existsSync(third.path));
  assert.ok(existsSync(other.path)); assert.equal(readFileSync(join(f.backupRoot, 'legacy/restore.db'), 'utf8'), 'keep');
});

test('缺少或失效的独占写保护拒绝创建，且不建立备份根', (t) => {
  const f = fixture(t);
  assert.throws(() => createVerifiedBackup({ ...f.options, sourceGuard: undefined }), /独占写保护/);
  assert.throws(() => createVerifiedBackup({ ...f.options, sourceGuard: { assertHeld: () => ({ source: f.source, exclusive: false }) } }), /未持有/);
  assert.equal(existsSync(f.backupRoot), false);
});

test('复制过程中源变化导致failed，不影响现有两份完整恢复点', (t) => {
  const f = fixture(t), a = createVerifiedBackup(f.options), b = createVerifiedBackup(f.options);
  let calls = 0;
  const sourceGuard = { assertHeld: () => { if (++calls === 3) writeFileSync(join(f.source, 'a.txt'), 'changed-after-scan'); return { source: f.source, exclusive: true }; } };
  assert.throws(() => createVerifiedBackup({ ...f.options, sourceGuard }), /源目录发生变化/);
  assert.ok(existsSync(a.path)); assert.ok(existsSync(b.path));
  assert.equal(registry(f.backupRoot).backups.filter((x) => x.status === 'failed').length, 1);
  pruneBackups(f.options);
  assert.equal(registry(f.backupRoot).backups.filter((x) => x.status === 'failed').length, 1);
});

test('复制后保护丢失时不产生verified记录', (t) => {
  const f = fixture(t); let calls = 0;
  const sourceGuard = { assertHeld: () => ({ source: f.source, exclusive: ++calls < 4 }) };
  assert.throws(() => createVerifiedBackup({ ...f.options, sourceGuard }), /未持有/);
  assert.equal(registry(f.backupRoot).backups[0].status, 'failed');
});

test('pinned、pendingRecovery和显式保护文件不会被保留策略删除', (t) => {
  const f = fixture(t);
  const pinned = createVerifiedBackup({ ...f.options, pinned: true });
  const pending = createVerifiedBackup({ ...f.options, pendingRecovery: true });
  const marker = createVerifiedBackup(f.options); writeFileSync(join(marker.path, 'PENDING_RECOVERY'), '');
  const marker2 = createVerifiedBackup(f.options); writeFileSync(join(marker2.path, 'PINNED'), '');
  createVerifiedBackup(f.options); createVerifiedBackup(f.options);
  for (const backup of [pinned, pending, marker, marker2]) assert.ok(existsSync(backup.path));
  assert.equal(pruneBackups({ ...f.options, dryRun: true }).protected.length, 4);
});

test('较新保留点被篡改时prune拒绝删除任何旧备份', (t) => {
  const f = fixture(t), old = createVerifiedBackup({ ...f.options, pinned: true });
  createVerifiedBackup(f.options); const newest = createVerifiedBackup(f.options); unpin(f.backupRoot, old.id);
  writeFileSync(join(newest.path, 'data/a.txt'), 'corrupted');
  assert.throws(() => pruneBackups(f.options), /清单不一致/);
  assert.ok(existsSync(old.path));
});

test('dry-run创建和清理不写任何文件，也不删除候选', (t) => {
  const f = fixture(t), before = contents(f.root);
  const preview = createVerifiedBackup({ ...f.options, dryRun: true });
  assert.equal(preview.wouldCreate, true); assert.deepEqual(contents(f.root), before);
  const old = createVerifiedBackup({ ...f.options, pinned: true }); createVerifiedBackup(f.options); createVerifiedBackup(f.options); unpin(f.backupRoot, old.id);
  const beforePrune = contents(f.root);
  assert.deepEqual(pruneBackups({ ...f.options, dryRun: true }).wouldDelete, [old.id]);
  assert.deepEqual(contents(f.root), beforePrune);
});

test('拒绝根目录、重叠目录和符号链接目录参数', (t) => {
  const f = fixture(t);
  for (const backupRoot of ['/', f.source, join(f.source, 'backup'), f.root]) assert.throws(() => createVerifiedBackup({ ...f.options, backupRoot }), /根目录|互相包含/);
  const link = join(f.root, 'alias'); symlinkSync(f.source, link);
  assert.throws(() => createVerifiedBackup({ ...f.options, source: link }), /符号链接/);
  symlinkSync(f.source, f.backupRoot);
  assert.throws(() => createVerifiedBackup(f.options), /符号链接/);
});

test('拒绝外部、绝对和悬空符号链接，不复制链接外文件', (t) => {
  const f = fixture(t); writeFileSync(join(f.root, 'outside'), 'private');
  const path = join(f.source, 'escape');
  for (const target of ['../outside', join(f.source, 'a.txt'), 'missing']) {
    symlinkSync(target, path);
    assert.throws(() => createVerifiedBackup(f.options), /符号链接/);
    rmSync(path);
  }
  assert.equal(registry(f.backupRoot).backups.filter((x) => x.status === 'verified').length, 0);
});

test('verify发现文件缺失、额外文件和内容篡改', (t) => {
  const f = fixture(t), backup = createVerifiedBackup(f.options), data = join(backup.path, 'data');
  writeFileSync(join(data, 'extra'), 'x');
  assert.throws(() => verifyBackup({ ...f.options, id: backup.id }), /清单不一致/);
  rmSync(join(data, 'extra')); rmSync(join(data, 'a.txt'));
  assert.throws(() => verifyBackup({ ...f.options, id: backup.id }), /清单不一致/);
});

test('登记归属不符或备份目录被换成链接时拒绝处理', (t) => {
  const f = fixture(t), backup = createVerifiedBackup(f.options);
  const ownerPath = join(backup.path, '.backup-owner.json');
  const owner = JSON.parse(readFileSync(ownerPath, 'utf8')); owner.rootId = 'other'; writeFileSync(ownerPath, JSON.stringify(owner));
  assert.throws(() => verifyBackup({ ...f.options, id: backup.id }), /归属不匹配/);
  rmSync(backup.path, { recursive: true }); symlinkSync(f.source, backup.path);
  assert.throws(() => pruneBackups(f.options), /符号链接/);
  assert.equal(readFileSync(join(f.source, 'a.txt'), 'utf8'), 'original');
});

test('dataset绑定固定source，并发锁不被自动覆盖或删除', (t) => {
  const f = fixture(t); createVerifiedBackup(f.options);
  const other = join(f.root, 'other'); mkdirSync(other);
  assert.throws(() => createVerifiedBackup({ ...f.options, source: other, sourceGuard: { assertHeld: () => ({ source: other, exclusive: true }) } }), /另一源目录/);
  const lock = join(f.backupRoot, '.verified-backups.lock'); writeFileSync(lock, 'owned-by-other-task');
  assert.throws(() => pruneBackups(f.options), /已被占用/);
  assert.equal(readFileSync(lock, 'utf8'), 'owned-by-other-task');
});

test('CLI要求完整参数和有效外部保护，支持create verify prune', (t) => {
  const f = fixture(t), guardFile = join(f.root, 'guard.json');
  writeFileSync(guardFile, JSON.stringify({ tool: 'cilai-source-write-guard', version: 1, source: f.source, exclusive: true, pid: process.pid, token: 'test-token' }));
  const common = ['--backup-root', f.backupRoot, '--dataset', f.options.dataset];
  const created = runCli(['create', ...common, '--source', f.source, '--guard-file', guardFile, '--guard-token', 'test-token']);
  assert.equal(runCli(['verify', ...common, '--id', created.id]).status, 'verified');
  assert.deepEqual(runCli(['prune', ...common, '--dry-run']).wouldDelete, []);
  assert.throws(() => runCli(['create', ...common, '--source', f.source, '--guard-file', guardFile, '--guard-token', 'wrong']), /保护无效/);
  assert.throws(() => runCli(['prune', '--dataset', 'x', '--dataset', 'y']), /重复参数/);
  assert.ok(runCli(['--help']).usage.includes('--dry-run'));
});


test('备份顶层出现未登记内容时拒绝自动删除', (t) => {
  const f = fixture(t), old = createVerifiedBackup({ ...f.options, pinned: true });
  createVerifiedBackup(f.options); createVerifiedBackup(f.options); unpin(f.backupRoot, old.id);
  writeFileSync(join(old.path, 'unregistered-important-file'), 'keep');
  assert.throws(() => pruneBackups(f.options), /未登记的额外内容/);
  assert.ok(existsSync(join(old.path, 'unregistered-important-file')));
});
