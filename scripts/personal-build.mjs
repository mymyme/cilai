import { existsSync, lstatSync, realpathSync } from 'node:fs';
import { basename, dirname, join, posix, resolve, sep, win32 } from 'node:path';
import { fileURLToPath } from 'node:url';

export const personalApplication = Object.freeze({
  appId: 'cn.cilai.personal.workbench', productName: '词来个人工作台',
  executableName: 'cilai-personal-workbench', credentialNamespace: 'cn.cilai.personal.workbench',
});
const here = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const projectRoot = basename(here) === '.personal-build' ? dirname(here) : resolve(here, '../..');
export const personalSourceRoot = join(projectRoot, '06代码索引', 'cilai-workbench');

export function personalBuildRoot({ platform = process.platform, project = projectRoot } = {}) {
  const paths = platform === 'win32' ? win32 : platform === 'darwin' ? posix : null;
  if (!paths) throw new Error(`个人桌面构建尚未支持 ${platform}`);
  return paths.resolve(project, '.personal-build');
}

export function assertNoSymlinkPath(path) {
  const absolute = resolve(path);
  const parts = absolute.split(sep);
  let cursor = absolute.startsWith(sep) ? sep : parts.shift();
  for (const part of parts.filter(Boolean)) {
    cursor = join(cursor, part);
    try {
      if (lstatSync(cursor).isSymbolicLink()) throw new Error(`个人工程路径不能经过符号链接：${cursor}`);
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
  }
  return absolute;
}

export function assertPersonalBuildRoot(root = personalBuildRoot()) {
  const expected = resolve(personalBuildRoot());
  if (resolve(root) !== expected) throw new Error(`只能写入已登记的个人构建根：${expected}`);
  assertNoSymlinkPath(expected);
  assertNoSymlinkPath(personalSourceRoot);
  if (existsSync(expected) && realpathSync(expected) !== expected) throw new Error('个人构建根的规范路径不一致');
  return expected;
}

export function assertPersonalBuildPath(path) {
  const root = assertPersonalBuildRoot();
  const absolute = resolve(path);
  if (absolute !== root && !absolute.startsWith(root + sep)) throw new Error(`路径必须位于个人构建根：${root}`);
  assertNoSymlinkPath(absolute);
  return absolute;
}

export function personalBuildChannel(environment = process.env) {
  const selected = environment.CILAI_PERSONAL_UPDATE_CHANNEL?.trim() || 'testing';
  if (selected !== 'testing') throw new Error('个人版当前只支持 testing 本地包，请将 CILAI_PERSONAL_UPDATE_CHANNEL 设为 testing 或取消设置');
  return selected;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (!args.length) console.log(assertPersonalBuildRoot());
  else if (args.length === 1 && args[0] === '--channel') console.log(personalBuildChannel());
  else throw new Error('个人构建参数无效');
}
