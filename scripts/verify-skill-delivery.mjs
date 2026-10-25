import { delegateManagedScript } from './managed-verification.mjs';
import { verificationEnvironment } from "./verification-environment.mjs";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { mkdtemp, mkdir, readFile, writeFile, lstat, realpath, rm } from "node:fs/promises";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { tmpdir } from "node:os";
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";

async function assertReportOutside(reportPath, temporaryRoot) {
  const target = resolve(reportPath);
  let canonical;
  try {
    await lstat(target);
    canonical = await realpath(target);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
    // A dangling report symlink must not bypass the destination check.
    const entry = await lstat(target).catch((missing) => {
      if (missing.code !== "ENOENT") throw missing;
      return null;
    });
    if (entry) throw new Error("验收报告目标不能是失效的符号链接");
    canonical = join(await realpath(dirname(target)), basename(target));
  }
  const offset = relative(temporaryRoot, canonical);
  if (!offset || (!offset.startsWith(`..${sep}`) && offset !== ".." && !isAbsolute(offset))) {
    throw new Error("验收报告必须位于本轮临时目录之外");
  }
}

function startService({ executable, servicePrefixArgs, root, requestTimeoutMs, stopTimeoutMs }) {
  const child = spawn(resolve(executable), [...servicePrefixArgs, "serve", "--transport", "stdio", "--enable-local-ipc", "--environment", "dev", "--data-dir", root, "--runtime-dir", join(root, "run")], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true, env: verificationEnvironment(root) });
  let id = 0, stderr = "", fatal = null, stopPromise, exitResult, stopping = false;
  const pending = new Map();
  const rejectPending = (error, permanent = true) => {
    if (permanent) fatal ??= error;
    for (const request of pending.values()) {
      clearTimeout(request.timer);
      request.reject(error);
    }
    pending.clear();
  };
  const closed = new Promise((resolveClosed) => {
    child.once("close", (code, signal) => {
      exitResult = { code, signal };
      rejectPending(new Error(`服务退出 ${code ?? signal}: ${stderr}`), !stopping);
      resolveClosed(exitResult);
    });
  });
  child.on("error", (error) => rejectPending(new Error(`服务启动失败: ${error.message}`)));
  child.stdin.on("error", (error) => rejectPending(new Error(`服务输入失败: ${error.message}`)));
  child.stderr.on("data", (chunk) => { stderr = (stderr + chunk).slice(-4000); });
  const lines = createInterface({ input: child.stdout });
  lines.on("line", (line) => {
    let message;
    try { message = JSON.parse(line); }
    catch { rejectPending(new Error(`服务输出不是有效 JSON: ${line.slice(0, 200)}`)); return; }
    if (!message || typeof message !== "object") {
      rejectPending(new Error("服务输出不是有效 JSON-RPC 消息")); return;
    }
    const request = pending.get(message.id);
    if (!request) return;
    clearTimeout(request.timer);
    pending.delete(message.id);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  });
  const rpc = (method, params) => new Promise((resolveRequest, reject) => {
    if (fatal) { reject(fatal); return; }
    const requestId = ++id;
    const timer = setTimeout(() => rejectPending(new Error(`请求超时 ${method}: ${stderr}`)), requestTimeoutMs);
    pending.set(requestId, { resolve: resolveRequest, reject, timer });
    child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: requestId, method, params }) + "\n");
  });
  const waitForClose = async () => {
    let timer;
    try {
      return await Promise.race([closed, new Promise((resolveTimeout) => {
        timer = setTimeout(() => resolveTimeout(null), stopTimeoutMs);
      })]);
    } finally { clearTimeout(timer); }
  };
  const stop = () => stopPromise ??= (async () => {
    stopping = true;
    child.stdin.end();
    let result = exitResult ?? await waitForClose();
    if (!result) {
      rejectPending(new Error("服务未在期限内正常退出"));
      for (const signal of ["SIGTERM", "SIGKILL"]) {
        if (child.exitCode === null && child.signalCode === null && child.pid) child.kill(signal);
        result = exitResult ?? await waitForClose();
        if (result) break;
      }
      if (!result) {
        lines.close(); child.stdin.destroy(); child.stdout.destroy(); child.stderr.destroy(); child.unref();
      }
      throw new Error(`服务未在期限内正常退出，保留诊断目录（本轮 PID ${child.pid ?? "未启动"}）`);
    }
    if (result.code !== 0) throw new Error(`服务未正常退出: ${result.code ?? result.signal}`);
    if (fatal) throw fatal;
  })();
  return {
    rpc, stop,
    initialize: async (platform, arch) => {
      await rpc("initialize", { client: { name: "contexthub-desktop", version: "1", platform, arch }, protocol: { minVersion: "1.0", maxVersion: "1.0" } });
      child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "initialized", params: {} }) + "\n");
    },
    diagnostics: () => ({ pid: child.pid ?? null, exit: exitResult ?? null, stderr }),
  };
}

export async function verifySkillDelivery({ executable, responsePath, reportPath, servicePrefixArgs = [], requestTimeoutMs = 15000, stopTimeoutMs = 5000 }) {
  if (!executable || !responsePath || !reportPath) throw new Error("需要本地服务路径、原始回复路径和验收报告路径");
  for (const value of [requestTimeoutMs, stopTimeoutMs]) {
    if (!Number.isFinite(value) || value <= 0) throw new Error("验收超时必须为正数");
  }
  const platform = { darwin: "macos", win32: "windows" }[process.platform];
  const arch = { arm64: "arm64", x64: "x86_64" }[process.arch];
  if (!platform || !arch) throw new Error(`不支持的验收平台：${process.platform}/${process.arch}`);
  const content = await readFile(responsePath, "utf8");
  const match = /^```cilai-skill[ \t]*\r?\n([\s\S]*?)^```[ \t]*$/m.exec(content);
  assert.ok(match, "原始回复应包含完整技能包");
  const expected = JSON.parse(match[1]);
  const temporaryRoot = await realpath(await mkdtemp(join(tmpdir(), "cilai-skill-delivery-")));
  const root = join(temporaryRoot, "dev");
  const services = [];
  const start = async () => {
    const service = startService({ executable, servicePrefixArgs, root, requestTimeoutMs, stopTimeoutMs });
    services.push(service);
    await service.initialize(platform, arch);
    return service;
  };
  let report, failure;
  try {
    await assertReportOutside(reportPath, temporaryRoot);
    await mkdir(root);
    await mkdir(join(root, "run"));
    await mkdir(join(root, "workspace"));
    let server = await start();
    const before = await server.rpc("aiState/get", {});
    const state = { conversations: [{ conversationId: "verification", context: { workspaceRoot: join(root, "workspace") } }],
      runs: [{ runId: "verification", conversationId: "verification", status: "completed", superseded: false, artifacts: [] }],
      messages: [{ messageId: "verification-reply", role: "assistant", runId: "verification", content }] };
    await server.rpc("aiState/replace", { expectedRevision: before.stateRevision, state });
    let installation;
    for (let attempt = 0; attempt < 30; attempt += 1) {
      const result = await server.rpc("skill/runList", { runIds: ["verification"] });
      installation = result.installations[0];
      if (installation?.status !== "pending") break;
      await new Promise((resolveWait) => setTimeout(resolveWait, 250));
    }
    assert.equal(installation.status, "installed", JSON.stringify(installation));
    const saved = (await server.rpc("skill/get", { skillId: installation.skillId })).skill;
    assert.equal(saved.name, expected.name.trim());
    assert.equal(saved.description, expected.description.trim());
    assert.equal(saved.instructions, expected.instructions.trim());
    assert.deepEqual(saved.files.map(({ path, content: text }) => ({ path, content: text })).sort((a, b) => a.path.localeCompare(b.path)), [...expected.files].sort((a, b) => a.path.localeCompare(b.path)));
    assert.equal((await server.rpc("skill/list", {})).skills.filter(skill => skill.source === "user").length, 1);
    await server.stop(); server = await start();
    const restarted = (await server.rpc("skill/runList", { runIds: ["verification"] })).installations[0];
    assert.equal(restarted.skillId, saved.skillId);
    const retried = await server.rpc("skill/runRetry", { runId: "verification" });
    assert.equal(retried.skillId, saved.skillId);
    assert.equal((await server.rpc("skill/list", {})).skills.filter(skill => skill.source === "user").length, 1);
    report = { status: "passed", name: saved.name, resourceCount: saved.files.length,
      responseSha256: createHash("sha256").update(content).digest("hex"),
      resources: saved.files.map((file) => ({ path: file.path, bytes: file.sizeBytes, sha256: createHash("sha256").update(file.content).digest("hex") })),
      backgroundWithoutUi: true, contentPreserved: true, restartPreserved: true, duplicateRetryPrevented: true, modelCalls: 0, isolatedDataDirectory: root };
  } catch (error) { failure = error; }
  const shutdownErrors = [];
  for (const service of services) {
    try { await service.stop(); } catch (error) { shutdownErrors.push(error.message); }
  }
  if (shutdownErrors.length) failure ??= new Error(shutdownErrors.join("; "));
  if (!failure) {
    try {
      await assertReportOutside(reportPath, temporaryRoot);
      await writeFile(reportPath, JSON.stringify(report, null, 2) + "\n");
      if ((await lstat(temporaryRoot)).isSymbolicLink() || await realpath(temporaryRoot) !== temporaryRoot) {
        throw new Error("本轮临时目录归属发生变化，停止清理");
      }
      await rm(temporaryRoot, { recursive: true });
      console.log(JSON.stringify({ ...report, temporaryDataRemoved: true }));
      return report;
    } catch (error) { failure = error; }
  }
  const diagnostic = { status: "failed", message: failure.message, shutdownErrors, temporaryRoot,
    reportPath: resolve(reportPath), services: services.map((service) => service.diagnostics()) };
  try { await writeFile(join(temporaryRoot, "failure.json"), JSON.stringify(diagnostic, null, 2) + "\n", { mode: 0o600 }); }
  catch (error) { diagnostic.diagnosticWriteError = error.message; }
  console.error(`验收失败，诊断目录已保留：${temporaryRoot}`);
  if (diagnostic.diagnosticWriteError) console.error(`诊断写入失败：${diagnostic.diagnosticWriteError}`);
  failure.diagnosticDirectory = temporaryRoot;
  throw failure;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const delegatedStatus = await delegateManagedScript(import.meta.url, { pathArguments: [0, 1, 2] });
  if (delegatedStatus !== null) process.exit(delegatedStatus);
  const [executable, responsePath, reportPath] = process.argv.slice(2);
  try { await verifySkillDelivery({ executable, responsePath, reportPath }); }
  catch (error) {
    console.error(JSON.stringify({ status: "failed", message: error.message, diagnosticDirectory: error.diagnosticDirectory ?? null }));
    process.exitCode = 1;
  }
}
