import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { closeSync, copyFileSync, mkdtempSync, openSync, readSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export function isMachO(file) {
  const descriptor = openSync(file, 'r');
  try {
    const magic = Buffer.alloc(4);
    if (readSync(descriptor, magic, 0, 4, 0) !== 4) return false;
    return ['feedface', 'cefaedfe', 'feedfacf', 'cffaedfe', 'cafebabe', 'bebafeca', 'cafebabf', 'bfbafeca'].includes(magic.toString('hex'));
  } finally {
    closeSync(descriptor);
  }
}

export function verifyMacSignature(file) {
  try {
    execFileSync('/usr/bin/codesign', ['--verify', '--strict', file], { stdio: 'pipe' });
  } catch (error) {
    throw new Error(`macOS 随包程序签名无效：${file}`, { cause: error });
  }
}

function payloadDigest(file, copy) {
  copyFileSync(file, copy);
  execFileSync('/usr/bin/codesign', ['--force', '--sign', '-', copy], { stdio: 'pipe' });
  execFileSync('/usr/bin/codesign', ['--remove-signature', copy], { stdio: 'pipe' });
  const hash = createHash('sha256');
  const descriptor = openSync(copy, 'r');
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  try {
    let size;
    while ((size = readSync(descriptor, buffer, 0, buffer.length, null)) > 0) hash.update(buffer.subarray(0, size));
    return hash.digest('hex');
  } finally {
    closeSync(descriptor);
  }
}

export function verifySignedMacPayload(source, packaged) {
  if (!isMachO(source) || !isMachO(packaged)) throw new Error(`macOS 随包程序格式改变：${packaged}`);
  verifyMacSignature(packaged);
  const temporary = mkdtempSync(join(tmpdir(), 'cilai-signature-payload-'));
  try {
    if (payloadDigest(source, join(temporary, 'source')) !== payloadDigest(packaged, join(temporary, 'packaged'))) {
      throw new Error(`macOS 随包程序内容与当前构建结果不一致：${packaged}`);
    }
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}
