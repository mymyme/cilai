import { delegateManagedScript } from './managed-verification.mjs';
import { createVerificationFixture } from './verification-fixture.mjs';
import { verificationEnvironment } from "./verification-environment.mjs";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, copyFile, readFile, writeFile, rm } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";

const delegatedStatus = await delegateManagedScript(import.meta.url, { pathArguments: [0] });
if (delegatedStatus !== null) process.exit(delegatedStatus);

const executable = process.argv[2];
if (!executable) throw new Error("请传入待验收的 codex-app-server 可执行文件路径");
const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const fixture = await createVerificationFixture("cilai-creator-harness-");
const root = fixture.root;
let fixtureFailure;
try {
const home = join(root, "harness");
const workspace = join(root, "workspace");
const extraRoot = join(root, "managed-skills");
const creatorPath = join(extraRoot, "creator", "SKILL.md");
const bundledPath = join(home, "skills/.system/skill-creator/SKILL.md");
await Promise.all([mkdir(home), mkdir(workspace), mkdir(dirname(creatorPath), { recursive: true })]);
await copyFile(join(sourceRoot, "crates/app-server/skills/skill-creator/SKILL.md"), creatorPath);
await writeFile(join(home, "config.toml"), '[[skills.config]]\nname = "unrelated-disabled-skill"\nenabled = false\n');

function start() {
  const child = fixture.track(spawn(resolve(executable), ["--listen", "stdio://", "-c", "features.plugins=false",
    "-c", "features.memories=false", "-c", "memories.generate_memories=false",
    "-c", "memories.use_memories=false", "-c", "check_for_update_on_startup=false",
    "-c", "cli_auth_credentials_store=\"file\""], {
    cwd: workspace, env: verificationEnvironment(root, { CODEX_HOME: home, CODEX_SQLITE_HOME: home }), stdio: ["pipe", "pipe", "pipe"],
  }));
  const pending = new Map();
  let fatal;
  const fail = (error) => {
    fatal ??= error;
    for (const request of pending.values()) { clearTimeout(request.timer); request.reject(error); }
    pending.clear();
  };
  let id = 0;
  let stderr = "";
  child.stderr.on("data", (chunk) => { stderr = (stderr + chunk).slice(-4000); });
  child.on("error", fail);
  child.stdin.on("error", fail);
  child.on("exit", (code) => {
    fail(new Error(`Harness退出 ${code}: ${stderr}`));
  });
  const lines = createInterface({ input: child.stdout });
  lines.on("line", (line) => {
    let message;
    try { message = JSON.parse(line); }
    catch (error) { fixture.markFailure(error); fail(error); return; }
    if (!message || typeof message !== "object") { const error = new Error("服务输出不是有效消息"); fixture.markFailure(error); fail(error); return; }
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  });
  const rpc = (method, params) => new Promise((resolveRequest, reject) => {
    if (fatal) { reject(fatal); return; }
    const requestId = ++id;
    const timer = setTimeout(() => {
      pending.delete(requestId);
      reject(new Error(`RPC超时: ${method}`));
    }, 20000);
    pending.set(requestId, { resolve: resolveRequest, reject, timer });
    child.stdin.write(JSON.stringify({ id: requestId, method, params }) + "\n");
  });
  return { rpc, notify: (method, params) => child.stdin.write(JSON.stringify({ method, params }) + "\n"),
    stop: async () => {
      for (const request of pending.values()) { clearTimeout(request.timer); request.reject(new Error("Harness已关闭")); }
      pending.clear();
      await fixture.stop(child);
      lines.close();
    } };
}

  for (const restart of [false, true]) {
    const server = start();
    try {
      await server.rpc("initialize", { clientInfo: { name: "cilai-creator-verification", version: "1" }, capabilities: { experimentalApi: true } });
      server.notify("initialized", {});
      const before = await server.rpc("skills/list", { cwds: [workspace], forceReload: true });
      const original = before.data[0].skills.find((skill) => skill.path === bundledPath);
      assert.ok(original, "Harness原版创建器应存在，才可验证定向停用");
      assert.equal(original.enabled, !restart);
      const originalBytes = await readFile(bundledPath);
      const configured = await server.rpc("skills/config/write", { path: bundledPath, enabled: false });
      assert.equal(configured.effectiveEnabled, false);
      await server.rpc("skills/extraRoots/set", { extraRoots: [extraRoot] });
      const after = await server.rpc("skills/list", { cwds: [workspace], forceReload: true });
      assert.deepEqual(after.data[0].errors, []);
      const skills = after.data[0].skills;
      assert.equal(skills.find((skill) => skill.path === bundledPath)?.enabled, false);
      assert.equal(skills.find((skill) => skill.path === creatorPath)?.enabled, true);
      assert.equal(skills.filter((skill) => skill.enabled && ["技能创建器", "skill-creator"].includes(skill.name)).length, 1);
      assert.deepEqual(await readFile(bundledPath), originalBytes, "停用不改写Harness内置内容");
      assert.match(await readFile(join(home, "config.toml"), "utf8"), /unrelated-disabled-skill/);
      console.log(JSON.stringify({ phase: restart ? "rebuilt-cache-restart" : "first-start", customCreatorEnabled: true, bundledCreatorEnabled: false, unrelatedConfigPreserved: true }));
    } finally { await server.stop(); }
    if (!restart) await rm(join(home, "skills/.system/.codex-system-skills.marker"));
  }

} catch (error) { fixtureFailure = error; }
finally { await fixture.finish(fixtureFailure); }
