import { join, resolve } from 'node:path';
import { installNoticeFiles, noticeDigest, readNoticeFile, verifyNoticeFiles } from './package-runtime-notices.mjs';

const apacheSha256 = 'cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30';
const packageNames = ['package.json', 'apps/desktop/package.json', 'apps/desktop-ui/package.json', 'packages/app-ui/package.json'];

function inputs({ desktopDir }) {
  const root = resolve(desktopDir, '../..');
  const files = new Map(['LICENSE', 'NOTICE'].map(name => [name, readNoticeFile(join(root, name))]));
  if (noticeDigest(files.get('LICENSE')) !== apacheSha256) throw new Error('第一方 Apache-2.0 许可全文与已核对的标准文本不符');
  const manifests = Object.fromEntries(packageNames.map(name => {
    const original = readNoticeFile(join(root, name));
    if (JSON.parse(original).license !== 'Apache-2.0') throw new Error('第一方 npm 许可声明不一致');
    return [name, noticeDigest(original)];
  }));
  const cargo = readNoticeFile(join(root, 'Cargo.toml'));
  const workspace = cargo.toString('utf8').replaceAll('\r\n', '\n').split('[workspace.package]')[1]?.split('\n[')[0];
  if (!/^license = "Apache-2.0"$/m.test(workspace ?? '')) throw new Error('第一方 Rust 许可声明不一致');
  for (const name of ['LICENSE', 'NOTICE']) {
    if (!files.get(name).equals(readNoticeFile(join(root, 'packages/app-ui', name)))) throw new Error('共享组件许可与第一方许可不一致');
  }
  const provenance = { schemaVersion: 1, component: 'Cilai Personal Workbench', license: 'Apache-2.0',
    licenseSource: 'https://www.apache.org/licenses/LICENSE-2.0.txt', manifests, cargoManifestSha256: noticeDigest(cargo),
    files: Object.fromEntries([...files].map(([name, value]) => [name, { sha256: noticeDigest(value), bytes: value.length }])) };
  files.set('provenance.json', Buffer.from(JSON.stringify(provenance, null, 2) + '\n'));
  return { files, provenance };
}

export function verifyWorkbenchNotices(options) {
  const { files, provenance } = inputs(options);
  verifyNoticeFiles(join(options.resources, 'licenses/workbench'), files);
  return { directory: 'licenses/workbench', license: provenance.license, files: provenance.files,
    provenanceSha256: noticeDigest(files.get('provenance.json')) };
}

export function installWorkbenchNotices(options) {
  const { files } = inputs(options);
  installNoticeFiles(join(options.resources, 'licenses/workbench'), files);
  return verifyWorkbenchNotices(options);
}
