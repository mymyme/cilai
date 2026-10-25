import { openProcesses } from './storage-lifecycle.mjs';
import { lstat, mkdtemp, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export async function createVerificationFixture(prefix, { stopTimeoutMs = 5000, inspect = openProcesses } = {}) {
  if (!Number.isFinite(stopTimeoutMs) || stopTimeoutMs <= 0) throw new Error('子进程退出超时必须为正数');
  const root = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  const ownership = await lstat(root);
  const children = new Map();
  const servers = new Set();
  let protocolFailure;
  function track(child) {
    const state = { child, result: null, error: null, promise: null };
    state.closed = new Promise((done) => {
      child.once('error', (error) => { state.error = error; });
      child.once('close', (code, signal) => { state.result = { code, signal }; done(state.result); });
    });
    children.set(child, state);
    return child;
  }
  async function wait(state) {
    let timer;
    try { return await Promise.race([state.closed, new Promise((done) => { timer = setTimeout(() => done(null), stopTimeoutMs); })]); }
    finally { clearTimeout(timer); }
  }
  function stop(child) {
    const state = children.get(child);
    if (!state) throw new Error('只能关闭当前验收创建的子进程');
    return state.promise ??= (async () => {
      if (state.error && !child.pid) { await state.closed; throw state.error; }
      if (!state.result) child.stdin?.end();
      let result = state.result ?? await wait(state);
      if (!result) {
        for (const signal of ['SIGTERM', 'SIGKILL']) {
          if (child.pid && child.exitCode === null && child.signalCode === null) child.kill(signal);
          result = state.result ?? await wait(state);
          if (result) break;
        }
        if (!result) { child.stdin?.destroy(); child.stdout?.destroy(); child.stderr?.destroy(); child.unref(); }
        throw new Error(`本轮子进程未正常退出，保留诊断（PID ${child.pid ?? '未启动'}）`);
      }
      if (state.error) throw state.error;
      if (result.code !== 0) throw new Error(`本轮子进程异常退出：${result.code ?? result.signal}`);
    })();
  }
  async function finish(failure) {
    const shutdownErrors = [];
    for (const child of children.keys()) {
      try { await stop(child); } catch (error) { shutdownErrors.push(error.message); }
    }
    for (const server of servers) {
      server.closeAllConnections();
      if (server.listening) await new Promise((done) => server.close(done));
    }
    let occupancyError, activePids;
    if (!failure && !protocolFailure && !shutdownErrors.length) {
      try {
        activePids = inspect(root);
        if (!Array.isArray(activePids) || activePids.some((pid) => !Number.isSafeInteger(pid) || pid < 1)) throw new Error('占用检查结果无效');
      } catch (error) { occupancyError = error.message; }
    }
    if (occupancyError) {
      const result = { status: 'passed', cleanup: 'needs-review', temporaryRoot: root, message: occupancyError };
      await writeFile(join(root, 'cleanup-needs-review.json'), `${JSON.stringify(result, null, 2)}\n`, { mode: 0o600 });
      console.error(`无法确认本轮目录无占用，保留现场：${root}（${occupancyError}）`);
      return result;
    }
    const error = failure ?? protocolFailure ?? (shutdownErrors.length ? new Error(shutdownErrors.join('; ')) : null)
      ?? (activePids?.length ? new Error(`本轮目录仍有进程占用：${activePids.join(', ')}`) : null);
    if (error) {
      const details = { status: 'failed', message: error.message, shutdownErrors, temporaryRoot: root,
        children: [...children.values()].map((state) => ({ pid: state.child.pid ?? null, exit: state.result, error: state.error?.message ?? null })) };
      await writeFile(join(root, 'failure.json'), `${JSON.stringify(details, null, 2)}\n`, { mode: 0o600 });
      console.error(`验收失败，诊断目录已保留：${root}`);
      error.diagnosticDirectory = root;
      throw error;
    }
    const current = await lstat(root);
    if (!current.isDirectory() || current.isSymbolicLink() || current.dev !== ownership.dev || current.ino !== ownership.ino
      || await realpath(root) !== root) throw new Error('本轮验收目录归属变化，停止清理');
    await rm(root, { recursive: true });
  }
  return { root, track, stop, finish, markFailure: (error) => { protocolFailure ??= error; }, trackServer: (server) => { servers.add(server); return server; } };
}
