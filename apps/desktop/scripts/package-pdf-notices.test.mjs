import assert from 'node:assert/strict';
import { cpSync, existsSync, mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { installPdfNotices, verifyPdfNotices } from './package-pdf-notices.mjs';

function fixture() {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'cilai-pdf-notices-'));
  const desktopDir = join(root, 'apps/desktop');
  const resources = join(root, 'resources');
  const write = (relative, value) => { const file = join(root, relative); mkdirSync(dirname(file), { recursive: true }); writeFileSync(file, value); };
  write('apps/desktop-ui/package.json', JSON.stringify({ dependencies: { 'pdfjs-dist': '6.3.289' } }));
  write('package-lock.json', JSON.stringify({ packages: { 'node_modules/pdfjs-dist': { version: '6.3.289' } } }));
  write('node_modules/pdfjs-dist/package.json', JSON.stringify({ version: '6.3.289' }));
  for (const name of ['LICENSE', 'wasm/LICENSE_OPENJPEG', 'wasm/LICENSE_PDFJS_OPENJPEG', 'wasm/LICENSE_JBIG2', 'wasm/LICENSE_PDFJS_JBIG2']) write('node_modules/pdfjs-dist/' + name, 'original ' + name + '\r\n');
  for (const name of ['openjpeg_nowasm_fallback.js', 'jbig2_nowasm_fallback.js']) {
    write('node_modules/pdfjs-dist/wasm/' + name, 'original ' + name);
    write('resources/frontend/pdfjs/wasm/' + name, 'original ' + name);
  }
  return { root, desktopDir, resources, write };
}

test('实际两个no-WASM资源和锁版相符时，PDF与依赖五份许可逐字随包且可核来源', () => {
  const f = fixture();
  try {
    const receipt = installPdfNotices(f);
    assert.equal(receipt.version, '6.3.289');
    assert.equal(Object.keys(receipt.files).length, 5);
    assert.equal(Object.keys(receipt.assets).length, 2);
    assert.deepEqual(verifyPdfNotices(f), receipt);
    assert.deepEqual(installPdfNotices(f), receipt);
    for (const [name, value] of Object.entries(receipt.files)) assert.deepEqual(readFileSync(join(f.resources, 'licenses/pdfjs-dist', name)), readFileSync(join(f.root, 'node_modules/pdfjs-dist', value.source)));
    f.write('resources/licenses/pdfjs-dist/LICENSE_OPENJPEG', 'modified');
    assert.throws(() => installPdfNotices(f), /不符/);
    assert.equal(readFileSync(join(f.resources, 'licenses/pdfjs-dist/LICENSE_OPENJPEG'), 'utf8'), 'modified');
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});

test('版本错、资源错、许可缺失和链接来源明确拒绝，不创建候选目录', () => {
  for (const kind of ['version', 'asset', 'missing', 'link']) {
    const f = fixture();
    try {
      if (kind === 'version') f.write('node_modules/pdfjs-dist/package.json', '{"version":"6.0.0"}');
      if (kind === 'asset') f.write('resources/frontend/pdfjs/wasm/openjpeg_nowasm_fallback.js', 'different');
      const license = join(f.root, 'node_modules/pdfjs-dist/wasm/LICENSE_JBIG2');
      if (['missing', 'link'].includes(kind)) rmSync(license);
      if (kind === 'link') symlinkSync(join(f.root, 'node_modules/pdfjs-dist/LICENSE'), license);
      assert.throws(() => installPdfNotices(f), /不符|缺少|链接/);
      assert.equal(existsSync(join(f.resources, 'licenses/pdfjs-dist')), false);
    } finally { rmSync(f.root, { recursive: true, force: true }); }
  }
});

test('最终门禁拒绝缺失、来源被改写或额外文件，不复制缺少的资源', () => {
  const f = fixture();
  try {
    assert.throws(() => verifyPdfNotices(f), /缺少/);
    installPdfNotices(f);
    cpSync(join(f.resources, 'licenses/pdfjs-dist/provenance.json'), join(f.root, 'saved-provenance'));
    f.write('resources/licenses/pdfjs-dist/provenance.json', '{}');
    assert.throws(() => verifyPdfNotices(f), /不符/);
    cpSync(join(f.root, 'saved-provenance'), join(f.resources, 'licenses/pdfjs-dist/provenance.json'));
    f.write('resources/licenses/pdfjs-dist/unregistered', 'unexpected');
    assert.throws(() => verifyPdfNotices(f), /清单不符/);
    rmSync(join(f.resources, 'frontend/pdfjs/wasm/jbig2_nowasm_fallback.js'));
    assert.throws(() => verifyPdfNotices(f), /缺少/);
    assert.equal(existsSync(join(f.resources, 'frontend/pdfjs/wasm/jbig2_nowasm_fallback.js')), false);
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});
