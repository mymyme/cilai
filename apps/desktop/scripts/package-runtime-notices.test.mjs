import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { installRuntimeNotices, verifyRuntimeNotices } from './package-runtime-notices.mjs';

function fixture() {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'cilai-runtime-notices-'));
  const desktopDir = join(root, 'desktop');
  const resources = join(root, 'resources');
  const write = (relative, value) => { const file = join(desktopDir, relative); mkdirSync(dirname(file), { recursive: true }); writeFileSync(file, value); };
  write('package.json', JSON.stringify({ devDependencies: { electron: '44.4.4' } }));
  write('package-lock.json', JSON.stringify({ packages: { 'node_modules/electron': { version: '44.4.4' } } }));
  write('node_modules/electron/package.json', JSON.stringify({ version: '44.4.4' }));
  write('node_modules/electron/dist/version', '44.4.4\n');
  write('node_modules/electron/dist/LICENSE', 'original electron license\r\n');
  write('node_modules/electron/dist/LICENSES.chromium.html', '<html>original chromium notices</html>\n');
  return { root, desktopDir, resources, write, platform: 'darwin', arch: 'arm64' };
}

test('同版Electron和Chromium原文完整随包，来源可核且重复核对不改写', () => {
  const f = fixture();
  try {
    const receipt = installRuntimeNotices(f);
    assert.equal(receipt.version, '44.4.4');
    for (const name of ['LICENSE', 'LICENSES.chromium.html']) assert.deepEqual(readFileSync(join(f.resources, 'licenses/electron', name)), readFileSync(join(f.desktopDir, 'node_modules/electron/dist', name)));
    assert.deepEqual(verifyRuntimeNotices(f), receipt);
    assert.deepEqual(installRuntimeNotices(f), receipt);
    writeFileSync(join(f.resources, 'licenses/electron/LICENSE'), 'different');
    assert.throws(() => installRuntimeNotices(f), /不符/);
    assert.equal(readFileSync(join(f.resources, 'licenses/electron/LICENSE'), 'utf8'), 'different');
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});

for (const relative of ['package.json', 'package-lock.json', 'node_modules/electron/package.json', 'node_modules/electron/dist/version']) {
  test(`版本来源不一致时拒绝且不产生许可目录：${relative}`, () => {
    const f = fixture();
    try {
      f.write(relative, readFileSync(join(f.desktopDir, relative), 'utf8').replaceAll('44.4.4', '44.4.1'));
      assert.throws(() => installRuntimeNotices(f), /版本不符/);
      assert.equal(existsSync(join(f.resources, 'licenses/electron')), false);
    } finally { rmSync(f.root, { recursive: true, force: true }); }
  });
}

test('缺失、空文和链接来源均拒绝，不用许可元字段代替原文', () => {
  for (const kind of ['missing', 'empty', 'link']) {
    const f = fixture();
    try {
      const source = join(f.desktopDir, 'node_modules/electron/dist/LICENSES.chromium.html');
      rmSync(source);
      if (kind === 'empty') writeFileSync(source, '');
      if (kind === 'link') symlinkSync(join(f.desktopDir, 'node_modules/electron/dist/LICENSE'), source);
      assert.throws(() => installRuntimeNotices(f), /缺少|为空|链接/);
      assert.equal(existsSync(join(f.resources, 'licenses/electron')), false);
    } finally { rmSync(f.root, { recursive: true, force: true }); }
  }
});

test('签名后的最终门禁拒绝缺失、被改写或多出的通知', () => {
  const f = fixture();
  try {
    assert.throws(() => verifyRuntimeNotices(f), /缺少/);
    installRuntimeNotices(f);
    const target = join(f.resources, 'licenses/electron/provenance.json');
    const original = readFileSync(target);
    writeFileSync(target, '{}');
    assert.throws(() => verifyRuntimeNotices(f), /不符/);
    writeFileSync(target, original);
    writeFileSync(join(f.resources, 'licenses/electron/unregistered'), 'unexpected');
    assert.throws(() => verifyRuntimeNotices(f), /清单不符/);
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});
