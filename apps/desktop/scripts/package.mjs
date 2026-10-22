import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

import { stagingDirectory } from "./stage-binaries.mjs";
import { assertDebugPackageLocation, canonicalDebugBuildRoot } from "./debug-package-location.mjs";
import { verifyPackageLayout } from "./verify-package-layout.mjs";
import { windowsElectronDistribution } from "./windows-electron-distribution.mjs";
import { packageOutputDirectory } from "./package-output.mjs";
import { personalApplication, personalBuildChannel } from "../../../scripts/personal-build.mjs";
import { codexRuntimeTarget, verifyCodexRuntime } from '../../../scripts/prepare-codex-runtime.mjs';
import updateChannels from '../shell/update-channels.js';
import { verifyRuntimeNotices } from './package-runtime-notices.mjs';
import { verifyPdfNotices } from './package-pdf-notices.mjs';
import { verifyWorkbenchNotices } from './package-workbench-notices.mjs';
import { verifyDependencyNotices } from './package-dependency-notices.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const [config, ...requestedArgs] = process.argv.slice(2);
const localTest = requestedArgs.includes("--local");
if (!config || !config.endsWith('electron-builder.yml') || !localTest || requestedArgs.some(arg => arg !== '--local')) {
  throw new Error('个人版当前只允许 electron-builder.yml --local 本地测试构建');
}
const builderArgs = [];
const buildChannel = personalBuildChannel();
const localSigningArgs = process.platform === 'darwin'
  ? ['--dir', '--config.forceCodeSigning=false', '--config.mac.identity=-', '--config.mac.hardenedRuntime=false', '--config.mac.notarize=false'] : [];

const buildRoot = process.env.CILAI_BUILD_ROOT?.trim()
  || canonicalDebugBuildRoot();
const projectRoot = join(here, "..", "..", "..");
assertDebugPackageLocation({ profile: "debug", projectRoot, buildRoot });
const desktopPackage = JSON.parse(readFileSync(join(here, "..", "package.json"), "utf8"));
const version = desktopPackage.version;
const output = packageOutputDirectory({ buildRoot, channel: buildChannel });
mkdirSync(output, { recursive: true });
// 暂存目录与产物目录同源：两者都不进源码树，源码树在 iCloud 上。
// 打包工具把 extraResources 的来源一律拼在项目目录之后，绝对路径会被拼成
// `<项目目录>/<绝对路径>` 而指向不存在的位置，且只记一条警告照常出包——
// 随包组件因此会静默缺失。这里换算成相对路径交给它。
const projectDir = join(here, "..");
const sidecarStage = relative(projectDir, stagingDirectory(buildRoot));

const binary = join(here, "..", "node_modules", "electron-builder", "cli.js");
const platformBuilderArgs = [];
platformBuilderArgs.push('--config.artifactName=cilai-personal-workbench-${version}-${os}-${arch}-testing.${ext}');
platformBuilderArgs.push(...localSigningArgs);
if (process.platform === 'darwin') {
  const installedElectron = join(projectDir, 'node_modules/electron/dist');
  const installedVersion = readFileSync(join(installedElectron, 'version'), 'utf8').trim().replace(/^v/, '');
  if (installedVersion !== desktopPackage.devDependencies.electron) throw new Error('Electron 运行时与锁定版本不符');
  platformBuilderArgs.push(`--config.electronDist=${installedElectron}`);
}
if (process.platform === "win32") {
  // npm 安装器与 Builder 共用个人构建根中的下载缓存。
  const electronCache = process.env.electron_config_cache?.trim();
  if (!electronCache) {
    console.error("Windows 打包缺少 Electron 缓存配置，请通过项目统一构建入口运行");
    process.exit(1);
  }
  platformBuilderArgs.push(`--config.electronDownload.cache=${electronCache}`);
  const distribution = windowsElectronDistribution(projectDir, desktopPackage.devDependencies.electron);
  platformBuilderArgs.push(`--config.electronDist=${distribution}`);
  console.log(`Windows Electron 运行时：${distribution}（版本 ${desktopPackage.devDependencies.electron} 已核对）`);
}
console.log(`Electron 产物目录：${output}`);
const result = spawnSync(process.execPath, [binary, "--config", config, ...builderArgs, ...platformBuilderArgs, '--publish', 'never'], {
  cwd: join(here, ".."),
  env: { ...process.env, CILAI_DESKTOP_OUTPUT: output, CILAI_SIDECAR_STAGE: sidecarStage,
    CILAI_PACKAGE_ARCH: process.arch, CILAI_PACKAGE_PROFILE: 'debug',
    CILAI_UPDATE_CHANNEL: buildChannel, CILAI_UPDATE_FEED: '', CILAI_PACKAGE_DISTRIBUTION: 'local',
    CSC_IDENTITY_AUTO_DISCOVERY: 'false' },
  stdio: 'inherit',
});
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);

/**
 * 核验随包组件确实进了产物。
 *
 * 打包工具对取不到的资源只记一条警告就照常出包：组件缺失不会让构建失败，只会在员工
 * 装好之后以"业务服务拉不起来"的形式暴露。这里在出包之后当场比对一次。
 */
let packaged;
try {
  packaged = verifyPackageLayout({
    output,
    platform: process.platform,
    arch: process.arch,
    productName: personalApplication.productName,
    executableName: personalApplication.executableName,
    stagedDir: join(projectDir, sidecarStage),
    frontendDir: join(projectDir, "..", "desktop-ui", "dist"),
    signedSidecars: process.platform === 'darwin',
  });
  verifyCodexRuntime(join(packaged.resources, 'sidecar', 'codex-runtime'), codexRuntimeTarget());
  verifyRuntimeNotices({ desktopDir: projectDir, resources: packaged.resources, platform: process.platform, arch: process.arch });
  verifyPdfNotices({ desktopDir: projectDir, resources: packaged.resources });
  verifyWorkbenchNotices({ desktopDir: projectDir, resources: packaged.resources });
  verifyDependencyNotices({ desktopDir: projectDir, resources: packaged.resources, platform: process.platform, arch: process.arch });
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
const updaterPath = process.platform === "win32"
  ? "./resources/app.asar/node_modules/electron-updater"
  : "./Contents/Resources/app.asar/node_modules/electron-updater";
const runtimeCheck = spawnSync(packaged.executable, ["-e", `require(${JSON.stringify(updaterPath)})`], {
  cwd: packaged.bundle,
  env: { ...process.env, ELECTRON_RUN_AS_NODE: "1" },
  encoding: "utf8",
});
if (runtimeCheck.status !== 0) {
  console.error("产物主进程运行依赖不完整");
  console.error((runtimeCheck.stderr || runtimeCheck.stdout || runtimeCheck.error?.message || "未返回错误详情").trim());
  process.exit(1);
}
console.log(`随包组件核验通过：${packaged.sidecarCount} 项`);
console.log(`随包前端核验通过：${packaged.frontendCount} 项`);
console.log("随包主进程依赖核验通过");
const packagedUpdateConfiguration = updateChannels.validateConfiguration(JSON.parse(readFileSync(join(packaged.resources, 'update-channels.json'), 'utf8')), process.platform, process.arch);
if ((packagedUpdateConfiguration.distribution === 'local') !== localTest
  || packagedUpdateConfiguration.buildChannel !== buildChannel
  || packagedUpdateConfiguration.buildProfile !== 'debug') {
  throw new Error('随包更新通道或构建配置与本次构建不一致');
}
console.log("本地测试包已生成：在线更新已禁用，不生成发布归档或更新元数据");
process.exit(0);
