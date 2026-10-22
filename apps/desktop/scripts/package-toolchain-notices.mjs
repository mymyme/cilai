import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readNoticeFile, noticeDigest } from './package-runtime-notices.mjs';
import { verifyDependencyNoticeFiles } from './package-dependency-notices.mjs';

export function toolchainNoticeInputs({ desktopDir, rustcInfo, sysroot }) {
  const root = resolve(desktopDir, '../..');
  const pinned = JSON.parse(readNoticeFile(join(root, 'third_party/rust-toolchain-notices/provenance.json')));
  const channel = readNoticeFile(join(root, 'rust-toolchain.toml')).toString().match(/^channel\s*=\s*"([^"]+)"/m)?.[1];
  if (pinned.schemaVersion !== 1 || channel !== pinned.version) throw new Error('Rust工具链许可与锁定版本不符');
  const run = args => {
    const result = spawnSync(process.env.RUSTC || 'rustc', args, { cwd: root, env: process.env, encoding: 'utf8' });
    if (result.status !== 0 || !result.stdout.trim()) throw new Error('无法核对Rust工具链许可来源');
    return result.stdout.trim();
  };
  const description = rustcInfo ?? run(['-vV']);
  const fields = Object.fromEntries(description.split(/\r?\n/).flatMap(line => {
    const at = line.indexOf(': '); return at < 0 ? [] : [[line.slice(0, at), line.slice(at + 2)]];
  }));
  if (fields.release !== pinned.version || fields['commit-hash'] !== pinned.commit) throw new Error('Rust编译器版本或提交与许可原文不符');
  const installation = sysroot ?? run(['--print', 'sysroot']);
  const source = join(installation, 'share/doc/rust');
  const expectedLicenses = Object.keys(pinned.installedDocuments).filter(name => name.startsWith('licenses/')).map(name => name.slice('licenses/'.length)).sort();
  if (JSON.stringify(readdirSync(join(source, 'licenses')).sort()) !== JSON.stringify(expectedLicenses)) throw new Error('Rust工具链许可原文目录与锁定发行件不符');
  const files = new Map();
  const verify = (name, record, bytes) => {
    if (bytes.length !== record.bytes || noticeDigest(bytes) !== record.sha256) throw new Error(`Rust工具链许可原文摘要不符：${name}`);
    files.set(name, bytes);
  };
  for (const [name, record] of Object.entries(pinned.installedDocuments)) verify(name, record, readNoticeFile(join(source, name)));
  for (const [name, record] of Object.entries(pinned.supplements)) verify(name, record, readNoticeFile(join(root, 'third_party/rust-toolchain-notices', name)));
  const provenance = { schemaVersion: 1, version: pinned.version, commit: pinned.commit, compilerHost: fields.host,
    scope: 'Complete official Rust toolchain and standard-library notice collection, including all installed license texts; not a pruned list of code linked for this target.',
    installedSource: 'share/doc/rust within the verified rustc sysroot', officialSource: pinned.officialSource,
    installedDocuments: pinned.installedDocuments, supplements: pinned.supplements };
  files.set('provenance.json', Buffer.from(JSON.stringify(provenance, null, 2) + '\n'));
  files.set('README.txt', Buffer.from('Rust toolchain and standard-library notices\n\nThis folder preserves both original copyright reports and every original license text from the verified Rust 1.97.1 toolchain. It is a complete official notice collection, including components for other platforms and compiler-only components; it is not an assertion that all listed code is linked into this application.\n\nThe standard library and its runtime support are outside the application Cargo metadata graph. compiler-builtins and libm notices from the exact rustc source commit are also retained here. The original SPDX templates are unchanged; copyright attribution remains in the original reports and notices. Source paths and SHA-256 digests are recorded in provenance.json.\n'));
  return { files, provenance };
}

export function installToolchainNotices(options) {
  const { files, provenance } = toolchainNoticeInputs(options);
  const directory = join(options.resources, 'licenses/rust-toolchain');
  if (!existsSync(directory)) for (const [name, bytes] of files) {
    const file = join(directory, name); mkdirSync(dirname(file), { recursive: true }); writeFileSync(file, bytes, { flag: 'wx', mode: 0o644 });
  }
  verifyDependencyNoticeFiles(directory, files);
  return { directory: 'licenses/rust-toolchain', version: provenance.version, files: files.size, provenanceSha256: noticeDigest(files.get('provenance.json')) };
}

export function verifyToolchainNotices(options) {
  const { files, provenance } = toolchainNoticeInputs(options);
  verifyDependencyNoticeFiles(join(options.resources, 'licenses/rust-toolchain'), files);
  return { directory: 'licenses/rust-toolchain', version: provenance.version, files: files.size, provenanceSha256: noticeDigest(files.get('provenance.json')) };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [resources] = process.argv.slice(2);
  if (!resources) throw new Error('请提供待核对的产物Resources目录');
  console.log(JSON.stringify(verifyToolchainNotices({ desktopDir: resolve(dirname(fileURLToPath(import.meta.url)), '..'), resources })));
}
