import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { installWindowsResourceRetry } from "./windows-resource-retry.mjs";
import { writeUpdateConfiguration } from "./update-configuration.mjs";
import { verifyPackageLayout } from "./verify-package-layout.mjs";
import { codexRuntimeTarget, verifyCodexRuntime } from '../../../scripts/prepare-codex-runtime.mjs';
import { restoreEmptyDirectories } from './package-empty-directories.mjs';
import { installRuntimeNotices } from './package-runtime-notices.mjs';
import { installPdfNotices } from './package-pdf-notices.mjs';
import { installWorkbenchNotices } from './package-workbench-notices.mjs';
import { installDependencyNotices } from './package-dependency-notices.mjs';
import { installToolchainNotices } from './package-toolchain-notices.mjs';

/**
 * 打包后修正应用元信息。
 *
 * 打包工具的配置对元信息是逐键深度合并：顶层的键可以用空值移除，嵌套字典里的键不行。
 * 传输安全的默认值正落在嵌套层，只能在产物上直接删。
 *
 * 保留的只有"网页内容允许明文加载"一项，与现有壳一致——容器里的业务页面需要它；
 * 业务服务自己的请求不经这套机制，不需要整体放开。
 */
const REMOVE = [
  "NSAppTransportSecurity.NSAllowsArbitraryLoads",
  "NSAppTransportSecurity.NSAllowsLocalNetworking",
  "NSAppTransportSecurity.NSExceptionDomains",
];

export default async function afterPack(context) {
  const platform = context.electronPlatformName;
  if (!['win32', 'darwin'].includes(platform)) return;
  const projectDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  if (!process.env.CILAI_SIDECAR_STAGE) throw new Error('缺少随包组件暂存目录');
  const stagedDir = resolve(projectDir, process.env.CILAI_SIDECAR_STAGE);
  const resources = platform === 'win32' ? join(context.appOutDir, 'resources')
    : join(context.appOutDir, `${context.packager.appInfo.productFilename}.app`, 'Contents', 'Resources');
  installRuntimeNotices({ desktopDir: projectDir, resources, platform, arch: process.env.CILAI_PACKAGE_ARCH });
  installPdfNotices({ desktopDir: projectDir, resources });
  installWorkbenchNotices({ desktopDir: projectDir, resources });
  const dependencies = installDependencyNotices({ desktopDir: projectDir, resources, platform, arch: process.env.CILAI_PACKAGE_ARCH });
  console.log(`依赖许可通知已复制：${dependencies.components} 个组件，${dependencies.files} 个文件`);
  const toolchain = installToolchainNotices({ desktopDir: projectDir, resources });
  console.log(`Rust工具链许可通知已复制：${toolchain.version}，${toolchain.files} 个文件`);
  const emptyDirectories = restoreEmptyDirectories(join(stagedDir, 'codex-runtime'), join(resources, 'sidecar', 'codex-runtime'));
  console.log(`官方 Codex 空目录结构已复制：${emptyDirectories} 项`);
  if (platform === 'darwin') {
    const packaged = verifyPackageLayout({
      output: dirname(context.appOutDir), platform,
      arch: process.env.CILAI_PACKAGE_ARCH,
      productName: context.packager.appInfo.productFilename,
      executableName: "cilai-personal-workbench",
      stagedDir,
      frontendDir: resolve(projectDir, '../desktop-ui/dist'),
    });
    verifyCodexRuntime(join(packaged.resources, 'sidecar', 'codex-runtime'),
      codexRuntimeTarget({ platform, arch: process.env.CILAI_PACKAGE_ARCH }));
    console.log('签名前随包组件与前端逐文件校验通过');
  }
  await writeUpdateConfiguration({ resources, platform,
    arch: process.env.CILAI_PACKAGE_ARCH,
    profile: process.env.CILAI_PACKAGE_PROFILE,
    channel: process.env.CILAI_UPDATE_CHANNEL,
    distribution: process.env.CILAI_PACKAGE_DISTRIBUTION });
  if (context.electronPlatformName === "win32") {
    installWindowsResourceRetry(context);
    return;
  }
  if (context.electronPlatformName !== "darwin") return;
  const plist = join(
    context.appOutDir,
    `${context.packager.appInfo.productFilename}.app`,
    "Contents",
    "Info.plist",
  );
  if (!existsSync(plist)) throw new Error(`找不到应用元信息：${plist}`);
  for (const key of REMOVE) {
    // 键不存在时 plutil 以非零退出，这不是错误——它可能已被配置移除
    try {
      execFileSync("/usr/bin/plutil", ["-remove", key, plist], { stdio: "pipe" });
    } catch {
      continue;
    }
  }
  console.log(`已收敛应用元信息：${REMOVE.length} 项传输安全默认值`);
}
