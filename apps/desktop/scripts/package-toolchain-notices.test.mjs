import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { installToolchainNotices, verifyToolchainNotices } from './package-toolchain-notices.mjs';
import { noticeDigest } from './package-runtime-notices.mjs';

function fixture(t) {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'cilai-toolchain-notices-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const write = (name, value) => { const file = join(root, name); mkdirSync(dirname(file), { recursive: true }); writeFileSync(file, value); return file; };
  const record = value => ({ bytes: Buffer.byteLength(value), sha256: noticeDigest(Buffer.from(value)) });
  const installedDocuments = {};
  for (const name of ['COPYRIGHT.html', 'COPYRIGHT-library.html', 'licenses/MIT.txt', 'licenses/Apache-2.0.txt']) {
    const value = `Original fixture document: ${name}\n`; write(`sysroot/share/doc/rust/${name}`, value); installedDocuments[name] = record(value);
  }
  const notice = 'Original compiler-builtins fixture notice\n';
  write('third_party/rust-toolchain-notices/compiler-builtins-LICENSE.txt', notice);
  write('third_party/rust-toolchain-notices/provenance.json', JSON.stringify({ schemaVersion: 1, version: '1.97.1', commit: 'fixture-commit', installedDocuments,
    supplements: { 'compiler-builtins-LICENSE.txt': record(notice) } }));
  write('rust-toolchain.toml', '[toolchain]\nchannel = "1.97.1"\n');
  return { write, root, desktopDir: join(root, 'apps/desktop'), resources: join(root, 'output'), sysroot: join(root, 'sysroot'),
    rustcInfo: 'release: 1.97.1\ncommit-hash: fixture-commit\nhost: aarch64-apple-darwin' };
}

test('完整Rust通知原文随包，可重复只读核验且不泄露sysroot路径', t => {
  const f = fixture(t);
  const installed = installToolchainNotices(f);
  assert.equal(installed.files, 7);
  assert.deepEqual(verifyToolchainNotices(f), installed);
  assert.deepEqual(installToolchainNotices(f), installed);
  const provenance = readFileSync(join(f.resources, 'licenses/rust-toolchain/provenance.json'), 'utf8');
  assert.ok(!provenance.includes(f.sysroot));
  assert.deepEqual(readFileSync(join(f.resources, 'licenses/rust-toolchain/COPYRIGHT-library.html')),
    readFileSync(join(f.sysroot, 'share/doc/rust/COPYRIGHT-library.html')));
});

test('编译器版本、commit或锁定channel漂移均停止', t => {
  const f = fixture(t);
  assert.throws(() => installToolchainNotices({ ...f, rustcInfo: f.rustcInfo.replace('1.97.1', '1.98.0') }), /版本或提交/);
  assert.throws(() => installToolchainNotices({ ...f, rustcInfo: f.rustcInfo.replace('fixture-commit', 'other-commit') }), /版本或提交/);
  f.write('rust-toolchain.toml', '[toolchain]\nchannel = "1.98.0"\n');
  assert.throws(() => installToolchainNotices(f), /锁定版本/);
});

test('工具链通知缺失、原文篡改和未登记文本不会被当作完整', t => {
  const f = fixture(t);
  const original = readFileSync(join(f.sysroot, 'share/doc/rust/COPYRIGHT-library.html'));
  f.write('sysroot/share/doc/rust/COPYRIGHT-library.html', 'modified');
  assert.throws(() => installToolchainNotices(f), /摘要不符/);
  f.write('sysroot/share/doc/rust/COPYRIGHT-library.html', original);
  const added = f.write('sysroot/share/doc/rust/licenses/EXTRA.txt', 'extra');
  assert.throws(() => installToolchainNotices(f), /目录与锁定发行件不符/);
  rmSync(added);
  rmSync(join(f.sysroot, 'share/doc/rust/licenses/MIT.txt'));
  assert.throws(() => installToolchainNotices(f), /目录与锁定发行件不符/);
});

test('最终包通知内容被修改或删除时只报错，不覆盖', t => {
  const f = fixture(t);
  installToolchainNotices(f);
  const file = join(f.resources, 'licenses/rust-toolchain/COPYRIGHT.html');
  writeFileSync(file, 'changed');
  assert.throws(() => verifyToolchainNotices(f), /不符/);
  assert.throws(() => installToolchainNotices(f), /不符/);
  assert.equal(readFileSync(file, 'utf8'), 'changed');
  rmSync(file);
  assert.throws(() => verifyToolchainNotices(f), /清单不符/);
});
