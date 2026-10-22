import { spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { readdir, lstat, writeFile, rename, unlink } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { writeUpdateConfiguration } from './update-configuration.mjs';
import channels from '../shell/update-channels.js';

export const PORTABLE_FEED = null;
export async function digestFile(file, algorithm = 'sha256', encoding = 'hex') {
  const hash = createHash(algorithm); let size = 0;
  for await (const bytes of createReadStream(file)) { hash.update(bytes); size += bytes.length; }
  return { digest: hash.digest(encoding), size };
}

export async function createPortableArchive({ bundle, output, version, channel = 'stable', profile = 'release' }) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('免安装构建版本无效');
  channels.validateChannel(channel);
  await writeUpdateConfiguration({ resources: join(bundle, 'resources'), platform: 'win32', arch: 'x64',
    profile, channel, distribution: 'portable' });
  const files = [];
  async function visit(directory, prefix = '') {
    for (const entry of (await readdir(directory)).sort()) {
      const relative = prefix + entry; const file = join(directory, entry); const info = await lstat(file);
      if (info.isSymbolicLink()) throw new Error(`免安装包不允许目录联接：${relative}`);
      if (info.isDirectory()) await visit(file, relative + '/');
      else if (info.isFile() && relative !== 'portable-package.json') {
        const actual = await digestFile(file);
        files.push({ path: relative, size: actual.size, sha256: actual.digest });
      } else if (!info.isFile()) throw new Error(`免安装包含特殊文件：${relative}`);
    }
  }
  await visit(bundle);
  const inventory = JSON.stringify({ schemaVersion: 1, kind: 'cilai-portable', version, channel, files }, null, 2) + '\n';
  await writeFile(join(bundle, 'portable-package.json'), inventory);
  const file = `cilai-personal-workbench-${version}-win-x64-${channel === 'testing' ? 'testing-' : ''}portable.zip`;
  const archive = join(output, file);
  // 本地开发允许重复构建；先完成独立归档再替换输出。远端同版本不可覆盖由发布器强制执行。
  const temporary = join(output, `.${file}.${randomUUID()}.building`);
  let actual;
  try {
    const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File',
      join(dirname(fileURLToPath(import.meta.url)), 'zip-portable.ps1'), '-SourceDirectory', bundle, '-ArchivePath', temporary], { stdio: 'inherit' });
    if (result.error || result.status !== 0) throw result.error ?? new Error('免安装 ZIP 构建失败');
    actual = await digestFile(temporary, 'sha512', 'base64');
    await rename(temporary, archive);
  } catch (error) {
    await unlink(temporary).catch(failure => { if (failure.code !== 'ENOENT') throw failure; });
    throw error;
  }
  const manifest = { schemaVersion: 1, kind: 'cilai-portable', platform: 'win32', arch: 'x64', version, channel,
    archive: { file, size: actual.size, sha512: actual.digest },
    packageManifestSha256: createHash('sha256').update(inventory).digest('hex'),
    releaseName: `词来个人工作台 ${version} ${channel === 'testing' ? '测试版' : '正式版'}`,
    releaseNotes: '个人版更新源尚未配置。' };
  await writeFile(join(output, 'portable.json'), JSON.stringify(manifest, null, 2) + '\n');
  console.log(`免安装更新包：${archive} (${actual.size} bytes)`);
  return manifest;
}
