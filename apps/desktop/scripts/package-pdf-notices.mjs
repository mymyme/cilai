import { join, resolve } from 'node:path';
import { installNoticeFiles, noticeDigest, readNoticeFile, verifyNoticeFiles } from './package-runtime-notices.mjs';

const names = ['LICENSE', 'wasm/LICENSE_OPENJPEG', 'wasm/LICENSE_PDFJS_OPENJPEG', 'wasm/LICENSE_JBIG2', 'wasm/LICENSE_PDFJS_JBIG2'];
const fallbackNames = ['openjpeg_nowasm_fallback.js', 'jbig2_nowasm_fallback.js'];

function inputs({ desktopDir, resources }) {
  const root = resolve(desktopDir, '../..');
  const declared = JSON.parse(readNoticeFile(join(root, 'apps/desktop-ui/package.json')));
  const lockedBytes = readNoticeFile(join(root, 'package-lock.json'));
  const locked = JSON.parse(lockedBytes);
  const source = join(root, 'node_modules/pdfjs-dist');
  const installedBytes = readNoticeFile(join(source, 'package.json'));
  const installed = JSON.parse(installedBytes);
  const version = declared.dependencies?.['pdfjs-dist'];
  if (!/^\d+\.\d+\.\d+$/.test(version ?? '') || locked.packages?.['node_modules/pdfjs-dist']?.version !== version || installed.version !== version) {
    throw new Error('PDF.js许可来源、锁定包与实际资源版本不符');
  }
  const assets = {};
  for (const name of fallbackNames) {
    const original = readNoticeFile(join(source, 'wasm', name));
    if (!original.equals(readNoticeFile(join(resources, 'frontend/pdfjs/wasm', name)))) throw new Error('PDF no-WASM资源与许可来源不符');
    assets[`pdfjs/wasm/${name}`] = noticeDigest(original);
  }
  const files = new Map(names.map(name => [name.replace('wasm/', ''), readNoticeFile(join(source, name))]));
  const provenance = { schemaVersion: 1, component: 'pdfjs-dist', version,
    acquisition: '个人锁定npm包；实际随包no-WASM资源与同包原文件逐字核对；未复制未使用的WASM程序',
    npmPackageSha256: noticeDigest(installedBytes), lockSha256: noticeDigest(lockedBytes), assets,
    files: Object.fromEntries(names.map(name => [name.replace('wasm/', ''), { source: name,
      sha256: noticeDigest(files.get(name.replace('wasm/', ''))), bytes: files.get(name.replace('wasm/', '')).length }])) };
  files.set('provenance.json', Buffer.from(JSON.stringify(provenance, null, 2) + '\n'));
  return { files, provenance };
}

export function verifyPdfNotices(options) {
  const { files, provenance } = inputs(options);
  verifyNoticeFiles(join(options.resources, 'licenses/pdfjs-dist'), files);
  return { directory: 'licenses/pdfjs-dist', version: provenance.version, assets: provenance.assets,
    files: provenance.files, provenanceSha256: noticeDigest(files.get('provenance.json')) };
}

export function installPdfNotices(options) {
  const { files } = inputs(options);
  installNoticeFiles(join(options.resources, 'licenses/pdfjs-dist'), files);
  return verifyPdfNotices(options);
}
