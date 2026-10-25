import { spawn } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertBuildLock, runLockedBuild, verifyStagedBuild } from './build-lock.mjs';
import { canonicalDebugBuildRoot } from '../apps/desktop/scripts/debug-package-location.mjs';

const moduleRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const entry = fileURLToPath(import.meta.url);
export function parseManagedArguments(args) {
  const copy = [...args];
  const desktop = copy[0] === '--desktop';
  if (desktop) copy.shift();
  if (copy.shift() !== '--' || !copy.length) throw new Error('验证入口需要 [--desktop] -- 命令');
  return { desktop, command: copy };
}
async function execute(command, args, options) {
  return new Promise((done, fail) => {
    const child = spawn(command, args, { ...options, stdio: 'inherit' });
    const interrupt = () => child.kill('SIGINT'), terminate = () => child.kill('SIGTERM');
    process.on('SIGINT', interrupt); process.on('SIGTERM', terminate);
    const detach = () => { process.off('SIGINT', interrupt); process.off('SIGTERM', terminate); };
    child.once('error', (error) => { detach(); fail(error); });
    child.once('close', (code) => { detach(); done(code ?? 1); });
  });
}
export async function runManagedVerification({ command, desktop = false, source = moduleRoot,
  buildRoot = canonicalDebugBuildRoot(), environment = process.env, run = execute,
  lock = runLockedBuild, assertLock = assertBuildLock, verify = verifyStagedBuild } = {}) {
  if (!Array.isArray(command) || !command.length) throw new Error('缺少验证命令');
  const root = resolve(buildRoot);
  if (!environment.CILAI_BUILD_LOCK_TOKEN) {
    return lock([process.execPath, entry, ...(desktop ? ['--desktop'] : []), '--', ...command], root, environment);
  }
  const owner = assertLock(root, environment.CILAI_BUILD_LOCK_TOKEN);
  const env = { ...environment, CILAI_BUILD_ROOT: root, CARGO_TARGET_DIR: join(root, 'target'), CARGO_INCREMENTAL: '0' };
  const personal = existsSync(join(source, 'scripts/personal-build.mjs'));
  if (personal) {
    Object.assign(env, { CARGO_HOME: join(root, 'toolchains/cargo'), npm_config_cache: join(root, 'cache/npm'),
      ELECTRON_CACHE: join(root, 'cache/electron'), electron_config_cache: join(root, 'cache/electron/Cache'),
      ELECTRON_BUILDER_CACHE: join(root, 'cache/electron-builder'), npm_config_userconfig: join(root, 'config/npmrc'),
      npm_config_globalconfig: join(root, 'config/npmrc-global') });
    mkdirSync(join(root, 'config'), { recursive: true });
    for (const file of [env.npm_config_userconfig, env.npm_config_globalconfig]) appendFileSync(file, '', { mode: 0o600 });
  }
  if (!owner?.manifestDigest) {
    const staged = await run(process.execPath, [join(source, 'scripts/stage-build.mjs')], { cwd: source, env });
    if (staged !== 0) return staged;
    if (personal) {
      const dependencies = await run(process.execPath, [join(root, 'scripts/ensure-build-dependencies.mjs'), '--ci'], { cwd: root, env });
      if (dependencies !== 0) return dependencies;
    }
  }
  verify(root, env.CILAI_BUILD_LOCK_TOKEN);
  const cwd = desktop ? join(root, 'apps/desktop') : root;
  let [program, ...args] = command;
  if (program === 'npm') {
    if (!env.npm_execpath || !isAbsolute(env.npm_execpath) || !existsSync(env.npm_execpath)) throw new Error('npm 验证必须从标准 npm 入口执行');
    args = [env.npm_execpath, ...args]; program = process.execPath;
  } else if (program === 'electron') {
    if (personal) {
      const runtime = await run(process.execPath, [join(root, 'apps/desktop/scripts/prepare-electron-runtime.mjs')], { cwd: root, env });
      if (runtime !== 0) return runtime;
    }
    args = [join(root, 'apps/desktop/node_modules/electron/cli.js'), ...args]; program = process.execPath;
  } else if (program === 'node') {
    program = process.execPath;
  }
  const status = await run(program, args, { cwd, env });
  if (status === 0) verify(root, env.CILAI_BUILD_LOCK_TOKEN);
  return status;
}
export async function delegateManagedScript(scriptUrl, { pathArguments = [], args = process.argv.slice(2),
  environment = process.env, cwd = process.cwd(), buildRoot = canonicalDebugBuildRoot() } = {}) {
  const script = fileURLToPath(scriptUrl);
  if (!process.argv[1] || resolve(process.argv[1]) !== script) return null;
  const offset = relative(moduleRoot, script);
  if (offset.startsWith(`..${sep}`) || isAbsolute(offset)) throw new Error('验证脚本必须属于当前源码');
  if (resolve(moduleRoot) === resolve(buildRoot) && environment.CILAI_BUILD_LOCK_TOKEN) {
    assertBuildLock(buildRoot, environment.CILAI_BUILD_LOCK_TOKEN);
    verifyStagedBuild(buildRoot, environment.CILAI_BUILD_LOCK_TOKEN);
    return null;
  }
  const absoluteArgs = args.map((value, index) => pathArguments.includes(index) && value ? resolve(cwd, value) : value);
  return runManagedVerification({ command: ['node', offset, ...absoluteArgs], environment, buildRoot });
}
if (process.argv[1] && resolve(process.argv[1]) === entry) {
  process.exitCode = await runManagedVerification(parseManagedArguments(process.argv.slice(2)));
}
