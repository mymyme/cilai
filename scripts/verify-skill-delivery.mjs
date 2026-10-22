import { verificationEnvironment } from "./verification-environment.mjs";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { createHash } from "node:crypto";

const [executable, responsePath, reportPath] = process.argv.slice(2);
if (!executable || !responsePath || !reportPath) throw new Error("需要本地服务路径、原始回复路径和验收报告路径");
const platform = { darwin: "macos", win32: "windows" }[process.platform];
const arch = { arm64: "arm64", x64: "x86_64" }[process.arch];
if (!platform || !arch) throw new Error(`不支持的验收平台：${process.platform}/${process.arch}`);
const content = await readFile(responsePath, "utf8");
const match = /^```cilai-skill[ \t]*\r?\n([\s\S]*?)^```[ \t]*$/m.exec(content);
assert.ok(match, "原始回复应包含完整技能包");
const expected = JSON.parse(match[1]);
const root = join(await mkdtemp(join(tmpdir(), "cilai-skill-delivery-")), "dev");
await mkdir(root);
await mkdir(join(root, "run"));
await mkdir(join(root, "workspace"));

async function start() {
  const child = spawn(resolve(executable), ["serve", "--transport", "stdio", "--enable-local-ipc", "--environment", "dev", "--data-dir", root, "--runtime-dir", join(root, "run")], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true, env: verificationEnvironment(root) });
  let id = 0;
  let stderr = "";
  const pending = new Map();
  child.stderr.on("data", (chunk) => { stderr = (stderr + chunk).slice(-4000); });
  createInterface({ input: child.stdout }).on("line", (line) => {
    const message = JSON.parse(line);
    const request = pending.get(message.id);
    if (!request) return;
    clearTimeout(request.timer); pending.delete(message.id);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  });
  child.on("exit", (code) => {
    for (const request of pending.values()) { clearTimeout(request.timer); request.reject(new Error(`服务退出 ${code}: ${stderr}`)); }
  });
  const rpc = (method, params) => new Promise((resolveRequest, reject) => {
    const requestId = ++id;
    const timer = setTimeout(() => reject(new Error(`请求超时 ${method}: ${stderr}`)), 15000);
    pending.set(requestId, { resolve: resolveRequest, reject, timer });
    child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: requestId, method, params }) + "\n");
  });
  const stop = () => new Promise((resolveStop) => {
    if (child.exitCode !== null) return resolveStop();
    child.once("exit", resolveStop); child.stdin.end();
  });
  await rpc("initialize", { client: { name: "contexthub-desktop", version: "1", platform, arch }, protocol: { minVersion: "1.0", maxVersion: "1.0" } });
  child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "initialized", params: {} }) + "\n");
  return { rpc, stop };
}

let server = await start();
try {
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
  const report = { status: "passed", name: saved.name, resourceCount: saved.files.length,
    responseSha256: createHash("sha256").update(content).digest("hex"),
    resources: saved.files.map((file) => ({ path: file.path, bytes: file.sizeBytes, sha256: createHash("sha256").update(file.content).digest("hex") })),
    backgroundWithoutUi: true, contentPreserved: true, restartPreserved: true, duplicateRetryPrevented: true, modelCalls: 0, isolatedDataDirectory: root };
  await writeFile(reportPath, JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify(report));
} finally { await server.stop(); }
