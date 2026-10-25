import { existsSync, realpathSync } from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import location from '../apps/desktop/shell/personal-build-location.js';

export const personalApplication = Object.freeze({
  appId: 'cn.cilai.personal.workbench', productName: '词来个人工作台',
  executableName: 'cilai-personal-workbench', credentialNamespace: 'cn.cilai.personal.workbench',
});
const here = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const context = location.sourceContext(here);
export const projectRoot = context.projectRoot;
export const personalSourceRoot = context.sourceRoot;
export const personalBuildRoot = location.personalBuildRoot;
export const assertNoSymlinkPath = location.rejectLinkedAncestors;
export const registerPersonalBuild = () => location.registerPersonalBuild(here);

export function assertPersonalBuildRoot(root = personalBuildRoot()) {
  location.sourceContext(here);
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
