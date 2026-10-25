import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, renameSync, statfsSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve, sep } from 'node:path';
import { performance } from 'node:perf_hooks';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { assertBuildLock, runLockedBuild, verifyStagedBuild } from './build-lock.mjs';
import { assertDebugPackageLocation, canonicalDebugBuildRoot } from '../apps/desktop/scripts/debug-package-location.mjs';

const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifestName = '.cilai-staged-files.json';
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const inside = (root, path) => path === root || path.startsWith(root + sep);

function assertPlainPath(path) {
  let cursor = resolve(path);
  while (true) {
    try {
      if (lstatSync(cursor).isSymbolicLink()) throw new Error(`验证路径不能经过符号链接：${cursor}`);
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
    const parent = dirname(cursor);
    if (parent === cursor) break;
    cursor = parent;
  }
}

function safeNumber(value) {
  const number = Number(value);
  if (!Number.isSafeInteger(number)) throw new Error('空间统计超出安全整数范围');
  return number;
}

export function sampleStorage(root, { platform = process.platform } = {}) {
  const paths = { root, target: join(root, 'target'), temporary: join(root, 'tmp') };
  const buckets = Object.fromEntries(Object.entries(paths).map(([name, path]) => [name, {
    path, exists: existsSync(path), logicalBytes: 0, allocatedBytes: platform === 'win32' ? null : 0,
    files: 0, symbolicLinks: 0,
  }]));
  const seen = Object.fromEntries(Object.keys(paths).map(name => [name, new Set()]));
  const stack = [root];
  while (stack.length) {
    const current = stack.pop();
    const info = lstatSync(current, { bigint: true });
    const owners = Object.entries(paths).filter(([, path]) => inside(path, current)).map(([name]) => name);
    if (info.isSymbolicLink()) {
      for (const name of owners) buckets[name].symbolicLinks += 1;
      continue;
    }
    if (info.isDirectory()) {
      for (const entry of readdirSync(current)) stack.push(join(current, entry));
    } else if (info.isFile()) {
      const identity = `${info.dev}:${info.ino}`;
      for (const name of owners) {
        if (seen[name].has(identity)) continue;
        seen[name].add(identity);
        buckets[name].files += 1;
        buckets[name].logicalBytes += safeNumber(info.size);
        if (platform !== 'win32') buckets[name].allocatedBytes += safeNumber(info.blocks * 512n);
      }
    }
  }
  const fs = statfsSync(root, { bigint: true });
  return {
    sampledAt: new Date().toISOString(), directories: buckets,
    filesystem: { path: root, capacityBytes: safeNumber(fs.blocks * fs.bsize),
      freeBytes: safeNumber(fs.bfree * fs.bsize), availableBytes: safeNumber(fs.bavail * fs.bsize) },
    accounting: '目录按文件身份去重，不跟随链接；各目录包含关系不得相加。POSIX分配字节不消除APFS克隆共享块；Windows仅报告逻辑字节。文件系统空闲变化可能包含其他进程活动。',
  };
}

export function assertInstalledDependencies(root, { personal = false } = {}) {
  for (const [folder, packages] of [
    ['apps/desktop-ui', ['typescript', 'vite', 'react', 'pdfjs-dist']],
    ['apps/desktop', ['electron', 'electron-builder']],
  ]) {
    const require = createRequire(join(root, folder, 'package.json'));
    for (const name of packages) {
      let installed;
      try { installed = realpathSync(require.resolve(name)); }
      catch { throw new Error(`缺少已安装依赖 ${name}；请先完成该版本的标准依赖安装，本入口不会安装依赖`); }
      if (personal && !inside(realpathSync(root), installed)) throw new Error(`个人版依赖越过构建根：${name}`);
    }
  }
  const electronRoot = join(root, 'apps/desktop/node_modules/electron');
  const expected = JSON.parse(readFileSync(join(root, 'apps/desktop/package.json'), 'utf8')).devDependencies.electron;
  const versionFile = join(electronRoot, 'dist/version');
  if (!existsSync(versionFile) || readFileSync(versionFile, 'utf8').trim().replace(/^v/, '') !== expected) {
    throw new Error('已安装Electron运行时缺失或版本不匹配；本入口不会下载或安装运行时');
  }
}

async function runProcess(command, args, options) {
  await new Promise((done, fail) => {
    const child = spawn(command, args, { ...options, stdio: 'inherit' });
    child.once('error', fail);
    child.once('exit', (code, signal) => {
      if (code === 0) done();
      else fail(new Error(`验证子命令失败：${args[0]}（${signal ?? code}）`));
    });
  });
}

function writeReport(root, report, mode) {
  const directory = join(root, 'target/storage-reports');
  assertPlainPath(directory);
  mkdirSync(directory, { recursive: true });
  const name = mode === 'package' ? 'verify-package-reuse' : 'verify-build-reuse';
  const output = join(directory, `${name}.json`);
  assertPlainPath(output);
  const pending = join(directory, `.${name}-${randomUUID()}.json`);
  writeFileSync(pending, JSON.stringify(report, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
  renameSync(pending, output);
  return output;
}

export async function verifyBuildReuse({
  buildRoot, source = sourceRoot, environment = process.env, personal = false,
  runner = runProcess, sampler = sampleStorage, dependencies = assertInstalledDependencies,
  verifySource = verifyStagedBuild, clock = () => performance.now(), nodeVersion = process.version,
  mode = 'prepare', platform = process.platform,
}) {
  if (!['prepare', 'package'].includes(mode)) throw new Error(`不支持的重复构建验证模式：${mode}`);
  if (nodeVersion !== 'v24.18.0') throw new Error(`验证需要 Node 24.18.0，当前为 ${nodeVersion}`);
  if (!['darwin', 'win32'].includes(platform)) throw new Error(`不支持的构建平台：${platform}`);
  const completePackage = mode === 'package';
  assertPlainPath(buildRoot);
  assertBuildLock(buildRoot, environment.CILAI_BUILD_LOCK_TOKEN);
  const env = { ...environment, CILAI_BUILD_ROOT: buildRoot, CARGO_TARGET_DIR: join(buildRoot, 'target'), CARGO_INCREMENTAL: '0',
    PATH: `${dirname(process.execPath)}${platform === 'win32' ? ';' : ':'}${environment.PATH ?? ''}` };
  if (personal) Object.assign(env, {
    CARGO_HOME: join(buildRoot, 'toolchains/cargo'), npm_config_cache: join(buildRoot, 'cache/npm'),
    ELECTRON_CACHE: join(buildRoot, 'cache/electron'), electron_config_cache: join(buildRoot, 'cache/electron/Cache'),
    ELECTRON_BUILDER_CACHE: join(buildRoot, 'cache/electron-builder'),
    npm_config_userconfig: join(buildRoot, 'config/npmrc'), npm_config_globalconfig: join(buildRoot, 'config/npmrc-global'),
  });
  const report = {
    version: 1, kind: completePackage ? 'full-package-reuse' : 'build-preparation-reuse', startedAt: new Date().toISOString(),
    buildRoot, source, nodeVersion, platform, passed: false,
    included: ['源码暂存', '工具链检查', '前端编译和资源校验', 'Rust服务编译', '平台原生组件准备', '随包组件暂存',
      ...(completePackage ? ['标准依赖校验及安装', 'Electron最终组包和本地测试签名', '应用启动自检'] : [])],
    excluded: completePackage ? ['正式分发签名与公证', '常驻应用启动', '发布'] : ['npm依赖安装', 'Electron最终组包和签名', '应用启动自检', '发布'],
    cachePolicy: { cargoTarget: env.CARGO_TARGET_DIR, cargoIncremental: env.CARGO_INCREMENTAL, cleaned: false, cacheHitsMeasured: false },
    passes: [],
  };
  let phase = 'stage-source';
  try {
    await runner(process.execPath, [join(source, 'scripts/stage-build.mjs')], { cwd: source, env });
    report.manifestSha256 = digest(readFileSync(join(buildRoot, manifestName)));
    verifySource(buildRoot, env.CILAI_BUILD_LOCK_TOKEN);
    if (!completePackage) {
      phase = 'installed-dependencies';
      dependencies(buildRoot, { personal });
    }
    phase = 'toolchain';
    await runner(process.execPath, [join(buildRoot, 'scripts/check-toolchain.mjs')], { cwd: buildRoot, env });
    phase = 'storage-before';
    report.storageBefore = sampler(buildRoot);
    for (let index = 1; index <= 2; index += 1) {
      phase = `${mode}-${index}`;
      assertBuildLock(buildRoot, env.CILAI_BUILD_LOCK_TOKEN);
      verifySource(buildRoot, env.CILAI_BUILD_LOCK_TOKEN);
      if (digest(readFileSync(join(buildRoot, manifestName))) !== report.manifestSha256) throw new Error('两次构建验证必须使用同一暂存源码清单');
      const pass = { index, passed: false };
      report.passes.push(pass);
      const started = clock();
      try {
        if (!completePackage) {
          await runner(process.execPath, [join(buildRoot, 'apps/desktop/scripts/prepare-package.mjs'), '--profile', 'debug'], { cwd: buildRoot, env });
        } else if (platform === 'darwin') {
          await runner('bash', [join(buildRoot, 'script/package.sh')], { cwd: buildRoot, env });
        } else {
          await runner('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(buildRoot, 'script/package.ps1')], { cwd: buildRoot, env });
        }
      } finally { pass.elapsedMs = Math.round(clock() - started); }
      verifySource(buildRoot, env.CILAI_BUILD_LOCK_TOKEN);
      pass.manifestSha256 = digest(readFileSync(join(buildRoot, manifestName)));
      if (pass.manifestSha256 !== report.manifestSha256) throw new Error('构建验证期间源码清单改变，本轮比较无效');
      phase = `storage-after-${index}`;
      pass.storageAfter = sampler(buildRoot);
      pass.passed = true;
    }
    report.comparison = {
      firstElapsedMs: report.passes[0].elapsedMs, secondElapsedMs: report.passes[1].elapsedMs,
      elapsedDifferenceMs: report.passes[1].elapsedMs - report.passes[0].elapsedMs,
      secondAvailableBytesChange: report.passes[1].storageAfter.filesystem.availableBytes - report.passes[0].storageAfter.filesystem.availableBytes,
      secondTargetLogicalBytesChange: report.passes[1].storageAfter.directories.target.logicalBytes - report.passes[0].storageAfter.directories.target.logicalBytes,
      interpretation: completePackage
        ? '两次均使用现有缓存，计时覆盖标准依赖处理、编译、最终组包及自检；首遍不是冷构建，耗时和空间变化不等于缓存命中率。'
        : '两次均使用现有缓存；首遍不是冷构建。耗时和空间变化不等于缓存命中率，也不代表完整打包耗时。',
    };
    report.passed = true;
  } catch (error) {
    report.failure = { phase, message: error.message };
  }
  report.completedAt = new Date().toISOString();
  assertBuildLock(buildRoot, env.CILAI_BUILD_LOCK_TOKEN);
  const output = writeReport(buildRoot, report, mode);
  if (!report.passed) throw new Error(`重复构建验证失败（${phase}）：${report.failure.message}；报告：${output}`);
  return { output, report };
}

export function parseVerifyArguments(args) {
  if (args.length === 0) return 'prepare';
  if (args.length === 1 && args[0] === '--package') return 'package';
  throw new Error('重复构建验证只接受 --package，不接受目录、启动或清理参数');
}

async function main() {
  const mode = parseVerifyArguments(process.argv.slice(2));
  if (process.version !== 'v24.18.0') throw new Error(`验证需要 Node 24.18.0，当前为 ${process.version}`);
  const buildRoot = canonicalDebugBuildRoot();
  assertDebugPackageLocation({ profile: 'debug', projectRoot: buildRoot, buildRoot });
  assertPlainPath(buildRoot);
  const personalModule = join(sourceRoot, 'scripts/personal-build.mjs');
  const personal = existsSync(personalModule);
  if (personal) (await import(pathToFileURL(personalModule).href)).assertPersonalBuildRoot(buildRoot);
  if (!process.env.CILAI_BUILD_LOCK_TOKEN) {
    process.exitCode = await runLockedBuild([process.execPath, fileURLToPath(import.meta.url), ...process.argv.slice(2)], buildRoot);
    return;
  }
  const { output, report } = await verifyBuildReuse({ buildRoot, personal, mode });
  console.log(JSON.stringify({ passed: report.passed, scope: report.kind, report: output, comparison: report.comparison }, null, 2));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
