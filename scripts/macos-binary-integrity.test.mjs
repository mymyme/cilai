import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { isMachO, verifySignedMacPayload } from './macos-binary-integrity.mjs';

test('macOS 重签名仅允许签名改变，程序内容改变即使重签成功仍拒绝', { skip: process.platform !== 'darwin' }, () => {
  const root = mkdtempSync(join(tmpdir(), 'cilai-macho-integrity-'));
  const source = join(root, 'source');
  const packaged = join(root, 'packaged');
  try {
    const architectures = execFileSync('/usr/bin/lipo', ['-archs', '/usr/bin/true'], { encoding: 'utf8' }).trim().split(/\s+/);
    if (architectures.length > 1) execFileSync('/usr/bin/lipo', ['/usr/bin/true', '-thin', architectures[0], '-output', source]);
    else copyFileSync('/usr/bin/true', source);
    copyFileSync(source, packaged);
    execFileSync('/usr/bin/codesign', ['--force', '--sign', '-', packaged], { stdio: 'pipe' });
    assert.equal(isMachO(source), true);
    assert.doesNotThrow(() => verifySignedMacPayload(source, packaged));
    const changed = readFileSync(packaged);
    const textSection = execFileSync('/usr/bin/otool', ['-l', packaged], { encoding: 'utf8' }).match(/sectname __text[\s\S]*?\boffset (\d+)/);
    assert.ok(textSection);
    changed[Number(textSection[1])] ^= 1;
    writeFileSync(packaged, changed);
    assert.throws(() => verifySignedMacPayload(source, packaged), /签名无效/);
    execFileSync('/usr/bin/codesign', ['--force', '--sign', '-', packaged], { stdio: 'pipe' });
    assert.throws(() => verifySignedMacPayload(source, packaged), /内容与当前构建结果不一致/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
