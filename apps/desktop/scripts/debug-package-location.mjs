import { posix, win32 } from 'node:path';
import { assertPersonalBuildRoot, personalBuildRoot } from '../../../scripts/personal-build.mjs';
export function canonicalDebugBuildRoot(options = {}) { return personalBuildRoot(options); }
export function assertDebugPackageLocation({ profile, projectRoot, buildRoot, platform = process.platform, project }) {
  if (!['debug', 'release'].includes(profile)) throw new Error('构建配置必须为 debug 或 release');
  const paths = platform === 'win32' ? win32 : posix;
  const normalize = value => platform === 'win32' ? paths.resolve(value).toLowerCase() : paths.resolve(value);
  const canonical = canonicalDebugBuildRoot({ platform, ...(project ? { project } : {}) });
  if (normalize(projectRoot) !== normalize(canonical) || normalize(buildRoot) !== normalize(canonical)) {
    throw new Error(`测试包只能从个人构建目录生成：${canonical}；请使用个人版打包入口`);
  }
  if (platform === process.platform && !project) assertPersonalBuildRoot(buildRoot);
}
