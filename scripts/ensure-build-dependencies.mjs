import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { existsSync, lstatSync, readFileSync, realpathSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { assertBuildLock } from './build-lock.mjs';
import { assertDebugPackageLocation, canonicalDebugBuildRoot } from '../apps/desktop/scripts/debug-package-location.mjs';

const stateName = '.cilai-dependency-state.json';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const contains = (root, file) => file === root || file.startsWith(root + sep);
const groups = [
  { name: 'workspace', directory: '', inputs: ['package.json', 'package-lock.json', 'apps/desktop-ui/package.json'] },
  { name: 'desktop', directory: 'apps/desktop', inputs: ['apps/desktop/package.json', 'apps/desktop/package-lock.json'] },
];

function boundaryError(message) {
  const error = new Error(message); error.code = 'DEPENDENCY_PATH_BOUNDARY'; return error;
}

function assertRoot(root, expected) {
  if (resolve(root) !== resolve(expected)) throw boundaryError('依赖安装只允许当前版本的标准构建根');
  if (realpathSync(root) !== resolve(root)) throw boundaryError('构建根不能经过符号链接');
  for (let current = resolve(root); ;) {
    if (lstatSync(current).isSymbolicLink()) throw boundaryError(`构建根路径存在符号链接：${current}`);
    const parent = dirname(current); if (parent === current) break; current = parent;
  }
}

function assertContainedPath(root, file) {
  const absolute = resolve(file);
  if (!contains(root, absolute)) throw boundaryError(`依赖路径越过构建根：${file}`);
  for (let current = absolute; contains(root, current); current = dirname(current)) {
    try {
      lstatSync(current);
      if (!contains(root, realpathSync(current))) throw boundaryError(`依赖链接越过构建根：${current}`);
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
    if (current === root) break;
  }
}

function lockPath(directory, name) {
  if (!name || isAbsolute(name) || name.includes('\\') || name.split('/').some(part => !part || part === '.' || part === '..')) {
    throw boundaryError(`依赖锁包含非法包路径：${name}`);
  }
  return join(directory, ...name.split('/'));
}

function readJson(root, file) {
  assertContainedPath(root, file);
  if (!lstatSync(file).isFile()) throw new Error(`依赖清单不是普通文件：${file}`);
  return JSON.parse(readFileSync(file, 'utf8'));
}

function inputDigest(root, group, runtime) {
  const h = createHash('sha256'), archives = new Set();
  const includeArchive = (specification, directory) => {
    if (typeof specification !== 'string' || !specification.startsWith('file:')) return;
    const local = specification.startsWith('file://') ? fileURLToPath(specification) : decodeURIComponent(specification.slice(5));
    if (!/\.(tgz|tar\.gz)$/i.test(local)) return;
    const archive = resolve(directory, local); assertContainedPath(root, archive);
    if (!lstatSync(archive).isFile()) throw new Error(`本地依赖归档不是普通文件：${archive}`);
    archives.add(archive);
  };
  const includeDependencies = (manifest, directory) => {
    for (const field of ['dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies']) {
      for (const specification of Object.values(manifest[field] ?? {})) includeArchive(specification, directory);
    }
  };
  h.update(JSON.stringify(runtime));
  for (const name of group.inputs) {
    const file = join(root, name); assertContainedPath(root, file);
    const bytes = readFileSync(file), manifest = JSON.parse(bytes);
    h.update(name); h.update('\0'); h.update(bytes); h.update('\0');
    if (name.endsWith('package-lock.json')) {
      for (const [packageName, entry] of Object.entries(manifest.packages ?? {})) {
        includeArchive(entry.resolved, dirname(file));
        if (!packageName.split('/').includes('node_modules')) includeDependencies(entry, resolve(dirname(file), packageName));
      }
    } else includeDependencies(manifest, dirname(file));
  }
  for (const archive of [...archives].sort()) {
    h.update(relative(root, archive)); h.update('\0'); h.update(readFileSync(archive)); h.update('\0');
  }
  return h.digest('hex');
}

function accepts(values, value) {
  if (!values) return true;
  if (!Array.isArray(values) || values.some(item => typeof item !== 'string')) throw new Error('依赖平台约束格式错误');
  if (values.includes(`!${value}`)) return false;
  const allowed = values.filter(item => !item.startsWith('!'));
  return allowed.length === 0 || allowed.includes(value) || allowed.includes('any');
}

export function inspectInstalledDependencies(root, group, runtime) {
  const directory = join(root, group.directory);
  const modules = join(directory, 'node_modules');
  assertContainedPath(root, modules);
  if (existsSync(modules) && lstatSync(modules).isSymbolicLink()) throw boundaryError('依赖安装目录不能是目录联接或符号链接');
  const lock = readJson(root, join(directory, 'package-lock.json'));
  if (![2, 3].includes(lock.lockfileVersion) || !lock.packages || typeof lock.packages !== 'object' || Array.isArray(lock.packages)) {
    throw new Error('依赖锁缺少可校验的packages清单');
  }
  for (const name of Object.keys(lock.packages)) if (name) assertContainedPath(root, lockPath(directory, name));
  const hiddenPath = join(modules, '.package-lock.json');
  assertContainedPath(root, hiddenPath);
  if (!existsSync(hiddenPath)) return { valid: false, reason: 'installed-lock-missing' };
  try {
    const hiddenBytes = readFileSync(hiddenPath);
    const hidden = JSON.parse(hiddenBytes);
    if (![2, 3].includes(hidden.lockfileVersion) || !hidden.packages || Array.isArray(hidden.packages)) throw new Error('installed-lock-invalid');
    for (const [name, entry] of Object.entries(lock.packages)) {
      if (!name.startsWith('node_modules/') || entry.optional || !accepts(entry.os, runtime.platform) || !accepts(entry.cpu, runtime.arch)) continue;
      if (!Object.hasOwn(hidden.packages, name)) throw new Error(`required-package-missing:${name}`);
    }
    let checked = 0;
    for (const [name, entry] of Object.entries(hidden.packages)) {
      if (!name) continue;
      const locked = lock.packages[name];
      if (!locked) throw new Error(`installed-package-unlocked:${name}`);
      const packageRoot = lockPath(directory, name);
      assertContainedPath(root, packageRoot);
      let expected = locked;
      if (locked.link) {
        if (!entry.link || typeof locked.resolved !== 'string') throw new Error(`workspace-link-invalid:${name}`);
        const workspace = lockPath(directory, locked.resolved);
        assertContainedPath(root, workspace);
        if (realpathSync(packageRoot) !== realpathSync(workspace)) throw new Error(`workspace-link-mismatch:${name}`);
        expected = lock.packages[locked.resolved];
      } else if (entry.version !== locked.version) throw new Error(`installed-lock-version-mismatch:${name}`);
      if (!expected || typeof expected.version !== 'string') throw new Error(`locked-package-version-missing:${name}`);
      const manifest = readJson(root, join(packageRoot, 'package.json'));
      if (manifest.version !== expected.version) throw new Error(`installed-package-version-mismatch:${name}`);
      checked += 1;
    }
    return { valid: true, hiddenLockSha256: hash(hiddenBytes), checkedPackages: checked };
  } catch (error) {
    if (error.code === 'DEPENDENCY_PATH_BOUNDARY') throw error;
    return { valid: false, reason: error.message };
  }
}

function readState(root) {
  const file = join(root, stateName); assertContainedPath(root, file);
  if (!existsSync(file)) return { version: 1, groups: {} };
  const state = readJson(root, file);
  if (state.version !== 1 || !state.groups || typeof state.groups !== 'object' || Array.isArray(state.groups)) throw new Error('依赖复用记录格式错误');
  return state;
}

function writeState(root, state, token) {
  assertBuildLock(root, token);
  const file = join(root, stateName); assertContainedPath(root, file);
  if (existsSync(file) && lstatSync(file).isSymbolicLink()) throw boundaryError('依赖复用记录不能是符号链接');
  const pending = join(root, `.cilai-dependency-state-${randomUUID()}.json`);
  writeFileSync(pending, JSON.stringify(state, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
  renameSync(pending, file);
}

async function npmCi(directory, environment, runtime) {
  const command = runtime.platform === 'win32' ? 'cmd.exe' : 'npm';
  const args = runtime.platform === 'win32' ? ['/d', '/s', '/c', 'npm.cmd ci --silent --no-audit'] : ['ci', '--silent', '--no-audit'];
  await new Promise((done, fail) => {
    const child = spawn(command, args, { cwd: directory, env: environment, stdio: 'inherit' });
    child.once('error', fail);
    child.once('exit', (code, signal) => code === 0 ? done() : fail(new Error(`npm ci失败（${signal ?? code}）`)));
  });
}

export async function ensureBuildDependencies({
  buildRoot = canonicalDebugBuildRoot(), expectedRoot = canonicalDebugBuildRoot(), environment = process.env,
  runtime = { node: process.version, platform: process.platform, arch: process.arch }, installer = npmCi,
}) {
  if (runtime.node !== 'v24.18.0') throw new Error(`依赖构建需要Node 24.18.0，当前为${runtime.node}`);
  if (!['darwin', 'win32'].includes(runtime.platform)) throw new Error(`不支持的构建平台：${runtime.platform}`);
  assertRoot(buildRoot, expectedRoot);
  const token = environment.CILAI_BUILD_LOCK_TOKEN; assertBuildLock(buildRoot, token);
  const env = { ...environment, CILAI_BUILD_ROOT: buildRoot,
    PATH: `${dirname(process.execPath)}${runtime.platform === 'win32' ? ';' : ':'}${environment.PATH ?? ''}`,
    npm_config_cache: join(buildRoot, 'cache/npm'), npm_config_userconfig: join(buildRoot, 'config/npmrc'),
    npm_config_globalconfig: join(buildRoot, 'config/npmrc-global'), ELECTRON_CACHE: join(buildRoot, 'cache/electron'),
    electron_config_cache: join(buildRoot, 'cache/electron/Cache'), ELECTRON_BUILDER_CACHE: join(buildRoot, 'cache/electron-builder'),
  };
  for (const name of ['npm_config_cache', 'npm_config_userconfig', 'npm_config_globalconfig', 'ELECTRON_CACHE', 'electron_config_cache', 'ELECTRON_BUILDER_CACHE']) {
    assertContainedPath(buildRoot, env[name]);
  }
  const state = readState(buildRoot), result = [];
  for (const group of groups) {
    assertBuildLock(buildRoot, token);
    const inputsSha256 = inputDigest(buildRoot, group, runtime);
    const installed = inspectInstalledDependencies(buildRoot, group, runtime);
    const saved = state.groups[group.name];
    if (installed.valid && saved?.inputsSha256 === inputsSha256 && saved.hiddenLockSha256 === installed.hiddenLockSha256 && saved.npmCiCompleted === true) {
      result.push({ group: group.name, action: 'reused', checkedPackages: installed.checkedPackages });
      continue;
    }
    const reason = !installed.valid ? installed.reason : saved?.inputsSha256 !== inputsSha256 ? 'inputs-changed-or-unrecorded' : 'installed-lock-changed';
    await installer(join(buildRoot, group.directory), env, runtime);
    assertBuildLock(buildRoot, token);
    if (inputDigest(buildRoot, group, runtime) !== inputsSha256) throw new Error(`安装期间${group.name}的依赖输入发生变化，未记录成功`);
    const checked = inspectInstalledDependencies(buildRoot, group, runtime);
    if (!checked.valid) throw new Error(`npm ci后${group.name}依赖校验失败：${checked.reason}`);
    state.groups[group.name] = { inputsSha256, hiddenLockSha256: checked.hiddenLockSha256,
      checkedPackages: checked.checkedPackages, runtime, npmCiCompleted: true, completedAt: new Date().toISOString() };
    writeState(buildRoot, state, token);
    result.push({ group: group.name, action: 'installed', reason, checkedPackages: checked.checkedPackages });
  }
  return result;
}

async function main() {
  if (process.argv.length !== 3 || process.argv[2] !== '--ci') throw new Error('依赖复用入口只接受 --ci');
  const root = canonicalDebugBuildRoot();
  assertDebugPackageLocation({ profile: 'debug', projectRoot: root, buildRoot: root });
  const personal = join(dirname(fileURLToPath(import.meta.url)), 'personal-build.mjs');
  if (!existsSync(personal)) throw new Error('此依赖复用入口仅用于个人版，不改变企业版安装流程');
  (await import(pathToFileURL(personal).href)).assertPersonalBuildRoot(root);
  console.log(JSON.stringify({ dependencies: await ensureBuildDependencies({ buildRoot: root }) }, null, 2));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
