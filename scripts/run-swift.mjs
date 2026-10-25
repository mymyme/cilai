import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { closeSync, lstatSync, mkdtempSync, openSync, readFileSync, readdirSync, readlinkSync, realpathSync, rmdirSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertBuildLock } from './build-lock.mjs';
import { assertManagedPath } from './storage-lifecycle.mjs';

const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const identity = stat => `${stat.dev}:${stat.ino}`;
const plain = path => {
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 1024 ** 2) throw new Error('Swift 构建登记必须为普通文件');
  return stat;
};

function registeredTemporary(environment) {
  const suppliedRoot = environment.CILAI_BUILD_ROOT, suppliedTemporary = environment.CILAI_BUILD_RUN_TMP;
  if (!suppliedRoot || !suppliedTemporary || !isAbsolute(suppliedRoot) || !isAbsolute(suppliedTemporary)) throw new Error('Swift 编译缺少本轮构建登记');
  const root = realpathSync(suppliedRoot);
  if (root !== resolve(suppliedRoot)) throw new Error('Swift 构建根不能经过符号链接');
  const temporary = assertManagedPath(root, resolve(suppliedTemporary)), run = dirname(temporary), id = basename(run);
  if (!uuid.test(id) || temporary !== join(root, 'tmp/storage-runs', id, 'tmp') || !lstatSync(temporary).isDirectory()) throw new Error('Swift 临时目录登记路径无效');
  const read = path => { assertManagedPath(root, path); plain(path); return JSON.parse(readFileSync(path, 'utf8')); };
  plain(join(root, '.cilai-build-lock.json'));
  const lock = assertBuildLock(root, environment.CILAI_BUILD_LOCK_TOKEN);
  const owner = read(join(run, 'owner.json'));
  const index = read(join(root, 'target/storage-reports/index.json'));
  const reportPath = join(root, 'target/storage-reports', `build-${id}.json`), report = read(reportPath);
  if (!Number.isSafeInteger(lock.pid) || lock.pid < 1 || !Number.isFinite(Date.parse(lock.startedAt))
    || owner.schema !== 'cilai-build-storage-v1' || owner.root !== root || owner.id !== id || owner.pid !== lock.pid || !Number.isFinite(Date.parse(owner.startedAt))
    || index.schema !== 'cilai-build-storage-index-v1' || index.root !== root || !Array.isArray(index.ids) || index.ids.filter(value => value === id).length !== 1
    || report.schema !== owner.schema || report.root !== root || report.id !== id || report.run !== run || report.temporary !== temporary
    || report.report !== reportPath || report.pid !== lock.pid || report.startedAt !== owner.startedAt || report.status !== 'running') throw new Error('Swift 构建锁与本轮临时目录归属不一致');
  try { process.kill(lock.pid, 0); } catch { throw new Error('Swift 构建锁进程已结束或无法确认'); }
  return { root, run, id, temporary, temporaryIdentity: identity(lstatSync(temporary)), lockPid: lock.pid };
}

function assertTemporary(state) {
  assertManagedPath(state.root, state.temporary);
  const stat = lstatSync(state.temporary);
  if (!stat.isDirectory() || identity(stat) !== state.temporaryIdentity) throw new Error('Swift 真实临时目录已被替换');
}

function cleanupAlias(alias, state) {
  assertTemporary(state);
  const directory = lstatSync(alias.directory), marker = lstatSync(alias.marker), link = lstatSync(alias.link);
  if (!directory.isDirectory() || directory.isSymbolicLink() || identity(directory) !== alias.directoryIdentity
    || !marker.isFile() || marker.isSymbolicLink() || identity(marker) !== alias.markerIdentity || marker.size !== Buffer.byteLength(alias.markerText)
    || !link.isSymbolicLink() || identity(link) !== alias.linkIdentity
    || readFileSync(alias.marker, 'utf8') !== alias.markerText || readlinkSync(alias.link) !== state.temporary
    || realpathSync(alias.link) !== state.temporary) throw new Error('Swift 临时别名归属已改变，保留待核实');
  if (readdirSync(alias.directory).sort().join(',') !== 'owner.json,tmp') throw new Error('Swift 临时别名包含未知条目，保留待核实');
  unlinkSync(alias.link);
  unlinkSync(alias.marker);
  rmdirSync(alias.directory);
}

export async function runSwift(args, { environment = process.env, start = spawn, stdout = process.stdout, stderr = process.stderr } = {}) {
  if (!Array.isArray(args) || !args.length || args.some(value => typeof value !== 'string' || value.includes('\0'))) throw new Error('Swift 编译参数无效');
  const inherited = { ...environment }, state = registeredTemporary(inherited), invocation = randomUUID();
  const prefix = join(state.temporary, `swift-${invocation}`), reportPath = `${prefix}.result.json`;
  const result = { schema: 'cilai-swift-invocation-v1', invocation, root: state.root, runId: state.id, temporary: state.temporary,
    lockPid: state.lockPid, wrapperPid: process.pid, startedAt: new Date().toISOString(), stdout: `${prefix}.stdout.log`, stderr: `${prefix}.stderr.log`, cleanup: 'not-created' };
  let alias, child, outputFd, errorFd, failure, interrupted = false, streamError;
  const forward = signal => { interrupted = true; child?.kill(signal); };
  const interrupt = () => forward('SIGINT'), terminate = () => forward('SIGTERM');
  try {
    alias = { directory: mkdtempSync('/private/tmp/cilai-swift-') };
    result.aliasDirectory = alias.directory; result.cleanup = 'retained';
    alias.directoryIdentity = identity(lstatSync(alias.directory));
    alias.marker = join(alias.directory, 'owner.json'); alias.link = join(alias.directory, 'tmp');
    alias.markerText = JSON.stringify({ schema: result.schema, invocation, root: state.root, runId: state.id, temporary: state.temporary,
      directory: alias.directory, lockPid: state.lockPid, wrapperPid: process.pid,
      lockTokenHash: createHash('sha256').update(inherited.CILAI_BUILD_LOCK_TOKEN).digest('hex') }) + '\n';
    writeFileSync(alias.marker, alias.markerText, { flag: 'wx', mode: 0o600 }); alias.markerIdentity = identity(lstatSync(alias.marker));
    symlinkSync(state.temporary, alias.link); alias.linkIdentity = identity(lstatSync(alias.link));
    assertTemporary(state);
    writeFileSync(`${prefix}.owner.json`, JSON.stringify(result, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
    outputFd = openSync(result.stdout, 'wx', 0o600); errorFd = openSync(result.stderr, 'wx', 0o600);
    process.on('SIGINT', interrupt); process.on('SIGTERM', terminate);
    const childEnvironment = { ...inherited, TMPDIR: alias.link, TMP: alias.link, TEMP: alias.link };
    const outcome = await new Promise(complete => {
      child = start('/usr/bin/xcrun', ['swiftc', ...args], { env: childEnvironment, stdio: ['ignore', 'pipe', 'pipe'] });
      result.childPid = child.pid;
      let spawnError;
      child.once('error', error => { spawnError = error; });
      child.once('close', (code, signal) => complete({ code, signal, spawnError }));
      const recordError = error => { streamError ??= error; child.kill('SIGTERM'); };
      const record = (fd, output) => bytes => { try { writeFileSync(fd, bytes); output.write(bytes); } catch (error) { recordError(error); } };
      if (!child.stdout || !child.stderr) { recordError(new Error('Swift 编译输出管道不可用')); return; }
      child.stdout.on('data', record(outputFd, stdout)); child.stderr.on('data', record(errorFd, stderr));
      child.stdout.once('error', recordError); child.stderr.once('error', recordError);
    });
    result.exitCode = outcome.code; result.signal = outcome.signal; result.interrupted = interrupted;
    if (outcome.spawnError) throw outcome.spawnError;
    if (streamError) throw streamError;
    if (interrupted || outcome.signal || outcome.code !== 0) throw new Error(`Swift 编译未成功退出：${outcome.signal ?? outcome.code}${interrupted ? '（构建已中断）' : ''}`);
    registeredTemporary(inherited);
    cleanupAlias(alias, state); result.cleanup = 'removed';
  } catch (error) {
    failure = error; result.error = error.message;
  } finally {
    process.off('SIGINT', interrupt); process.off('SIGTERM', terminate);
    if (outputFd !== undefined) closeSync(outputFd);
    if (errorFd !== undefined) closeSync(errorFd);
  }
  result.status = failure ? 'failed' : 'passed'; result.completedAt = new Date().toISOString();
  assertTemporary(state);
  writeFileSync(reportPath, JSON.stringify(result, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
  if (failure) {
    const error = new Error(`${failure.message}；Swift 诊断保留于 ${reportPath}${result.cleanup === 'retained' ? `；临时别名保留于 ${result.aliasDirectory}` : ''}`, { cause: failure });
    error.exitCode = Number.isInteger(result.exitCode) && result.exitCode > 0 ? result.exitCode : 1;
    error.reportPath = reportPath; throw error;
  }
  return { ...result, reportPath };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { await runSwift(process.argv.slice(2)); }
  catch (error) { console.error(error.message); process.exitCode = error.exitCode ?? 1; }
}
