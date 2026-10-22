import { posix, resolve, win32 } from 'node:path';
import { fileURLToPath } from 'node:url';
import { canonicalDebugBuildRoot } from './debug-package-location.mjs';
import channels from '../shell/update-channels.js';

export function packageOutputDirectory({
  buildRoot,
  platform = process.platform,
  channel,
  separateChannelOutput = process.env.CILAI_SEPARATE_CHANNEL_OUTPUT?.trim(),
  backgroundOutput = process.env.CILAI_BACKGROUND_OUTPUT?.trim(),
}) {
  const selected = channels.validateChannel(channel);
  if (separateChannelOutput && separateChannelOutput !== '1') throw new Error('独立通道输出开关只接受 1');
  if (backgroundOutput && (backgroundOutput !== '1' || platform !== 'win32')) throw new Error('后台独立输出仅支持 Windows，开关只接受 1');
  const paths = platform === 'win32' ? win32 : platform === 'darwin' ? posix : null;
  if (!paths) throw new Error(`不支持的桌面构建平台：${platform}`);
  if (typeof buildRoot !== 'string' || !paths.isAbsolute(buildRoot)) throw new Error('构建根目录必须是绝对路径');
  return paths.join(buildRoot, 'target', `desktop-electron-${selected}`);
}

export function packageBundlePaths({ output, platform, arch, productName, executableName = productName }) {
  const paths = platform === 'win32' ? win32 : platform === 'darwin' ? posix : null;
  if (!paths || typeof output !== 'string' || (!win32.isAbsolute(output) && !posix.isAbsolute(output))) throw new Error('产物目录必须是绝对路径');
  const joinPath = win32.isAbsolute(output) && !posix.isAbsolute(output) ? win32.join : posix.join;
  if (!productName || ['.', '..'].includes(productName) || paths.basename(productName) !== productName || /[\\/]/.test(productName)) throw new Error('桌面产品名无效');
  if (!executableName || ['.', '..'].includes(executableName) || /[\\/]/.test(executableName)) throw new Error('桌面可执行名称无效');
  if (platform === 'darwin' && ['arm64', 'x64'].includes(arch)) {
    const bundle = joinPath(output, arch === 'arm64' ? 'mac-arm64' : 'mac', `${executableName}.app`);
    return { bundle, resources: joinPath(bundle, 'Contents', 'Resources'),
      executable: joinPath(bundle, 'Contents', 'MacOS', executableName) };
  }
  if (platform === 'win32' && arch === 'x64') {
    const bundle = joinPath(output, 'win-unpacked');
    return { bundle, resources: joinPath(bundle, 'resources'), executable: joinPath(bundle, `${executableName}.exe`) };
  }
  throw new Error(`桌面产物暂不支持当前平台和架构：${platform}/${arch}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length > 4 || (process.argv[3] && !['--json', '--bundle'].includes(process.argv[3]))) throw new Error('不支持的输出格式');
  const output = packageOutputDirectory({
    buildRoot: process.env.CILAI_BUILD_ROOT?.trim() || canonicalDebugBuildRoot(),
    channel: process.argv[2],
  });
  // PowerShell 5.1 的本机管道解码可能仍是 OEM；ASCII JSON 避免中文地址损坏。
  const selected = process.argv[3] === '--bundle'
    ? packageBundlePaths({ output, platform: process.platform, arch: process.arch, productName: '词来个人工作台', executableName: 'cilai-personal-workbench' }).bundle : output;
  console.log(process.argv[3] === '--json'
    ? JSON.stringify(selected).replace(/[\u007f-\uffff]/g, character => `\\u${character.charCodeAt(0).toString(16).padStart(4, '0')}`)
    : selected);
}
