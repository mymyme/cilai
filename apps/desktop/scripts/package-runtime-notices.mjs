import { createHash } from 'node:crypto';
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';

const noticeNames = ['LICENSE', 'LICENSES.chromium.html'];
const digest = bytes => createHash('sha256').update(bytes).digest('hex');

function noLinks(file) {
  for (let current = resolve(file); ; current = dirname(current)) {
    if (existsSync(current) && lstatSync(current).isSymbolicLink()) throw new Error('许可来源或目标不可使用符号链接');
    if (dirname(current) === current) break;
  }
}

function bytes(file) {
  noLinks(file);
  if (!existsSync(file) || !lstatSync(file).isFile()) throw new Error(`缺少普通许可来源或目标文件：${file}`);
  const value = readFileSync(file);
  if (!value.length) throw new Error(`许可来源或目标文件为空：${file}`);
  return value;
}

export { bytes as readNoticeFile, digest as noticeDigest };

export function verifyNoticeFiles(directory, files) {
  noLinks(directory);
  if (!existsSync(directory) || !lstatSync(directory).isDirectory()) throw new Error('产物缺少许可目录');
  if (JSON.stringify(readdirSync(directory).sort()) !== JSON.stringify([...files.keys()].sort())) throw new Error('许可目录清单不符');
  for (const [name, value] of files) {
    if (!bytes(join(directory, name)).equals(value)) throw new Error(`随包许可或来源与当前发行件不符：${name}`);
  }
}

export function installNoticeFiles(directory, files) {
  noLinks(directory);
  if (!existsSync(directory)) {
    mkdirSync(directory, { recursive: true });
    for (const [name, value] of files) writeFileSync(join(directory, name), value, { flag: 'wx', mode: 0o644 });
  }
  verifyNoticeFiles(directory, files);
}

function inputs({ desktopDir, platform, arch }) {
  if (!['darwin', 'win32'].includes(platform) || !['arm64', 'x64'].includes(arch)) throw new Error('未登记许可目标平台');
  const declared = JSON.parse(bytes(join(desktopDir, 'package.json')));
  const lockedBytes = bytes(join(desktopDir, 'package-lock.json'));
  const locked = JSON.parse(lockedBytes);
  const runtime = join(desktopDir, 'node_modules/electron');
  const installedBytes = bytes(join(runtime, 'package.json'));
  const installed = JSON.parse(installedBytes);
  const version = declared.devDependencies?.electron;
  if (!/^\d+\.\d+\.\d+$/.test(version ?? '') || locked.packages?.['node_modules/electron']?.version !== version
    || installed.version !== version || bytes(join(runtime, 'dist/version')).toString('utf8').trim().replace(/^v/, '') !== version) {
    throw new Error('Electron许可来源、锁定包与实际运行时版本不符');
  }
  const files = new Map(noticeNames.map(name => [name, bytes(join(runtime, 'dist', name))]));
  const provenance = { schemaVersion: 1, component: 'Electron', version, platform, arch,
    acquisition: '个人锁定npm包附属的已安装官方发行目录；未重新下载、修改或从旧应用包取通知',
    distribution: `electron-v${version}-${platform}-${arch}`,
    npmPackageSha256: digest(installedBytes), lockSha256: digest(lockedBytes),
    files: Object.fromEntries([...files].map(([name, value]) => [name, { sha256: digest(value), bytes: value.length }])) };
  files.set('provenance.json', Buffer.from(JSON.stringify(provenance, null, 2) + '\n'));
  return { files, provenance };
}

export function verifyRuntimeNotices(options) {
  const { files, provenance } = inputs(options);
  const directory = join(options.resources, 'licenses/electron');
  verifyNoticeFiles(directory, files);
  return { directory: 'licenses/electron', version: provenance.version, files: provenance.files, provenanceSha256: digest(files.get('provenance.json')) };
}

export function installRuntimeNotices(options) {
  const { files } = inputs(options);
  const directory = join(options.resources, 'licenses/electron');
  installNoticeFiles(directory, files);
  return verifyRuntimeNotices(options);
}
