import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { acquireBuildLock, assertBuildLock, recordStagedBuild, releaseBuildLock, runLockedBuild, verifyStagedBuild } from "./build-lock.mjs";

function fixture(t) { const root = mkdtempSync(join(tmpdir(), "cilai-build-lock-")); t.after(() => rmSync(root, { recursive: true, force: true })); return root; }
test("质量检查和打包不能同时占用目录，错误身份不能释放锁", (t) => {
  const root = fixture(t), token = acquireBuildLock(root, "quality");
  assert.throws(() => acquireBuildLock(root, "package"), /已被占用/);
  assert.throws(() => releaseBuildLock(root, "other"), /另一任务/);
  assert.equal(assertBuildLock(root, token).command, "quality");
  releaseBuildLock(root, token);
  releaseBuildLock(root, acquireBuildLock(root, "package"));
});
test("源码或清单在构建中被覆盖时拒绝通过", (t) => {
  const root = fixture(t), token = acquireBuildLock(root, "quality");
  const manifest = JSON.stringify({ version: 1, files: { "source.ts": `file:${createHash("sha256").update("first").digest("hex")}` } });
  writeFileSync(join(root, "source.ts"), "first"); writeFileSync(join(root, ".cilai-staged-files.json"), manifest);
  recordStagedBuild(root, token); verifyStagedBuild(root, token);
  writeFileSync(join(root, "source.ts"), "second"); assert.throws(() => verifyStagedBuild(root, token), /源码被修改/);
  writeFileSync(join(root, ".cilai-staged-files.json"), "{}"); assert.throws(() => verifyStagedBuild(root, token), /清单被替换/);
  releaseBuildLock(root, token);
});
test("子命令失败和启动失败均释放自己的锁", async (t) => {
  const root = fixture(t);
  const environment = { ...process.env }; delete environment.CILAI_BUILD_LOCK_TOKEN;
  assert.equal(await runLockedBuild([process.execPath, "-e", "process.exit(7)"], root, environment), 7);
  await assert.rejects(runLockedBuild([join(root, "missing")], root, environment), /ENOENT/);
  releaseBuildLock(root, acquireBuildLock(root, "next"));
});
test("两端标准质量和打包入口先取得同一锁再暂存", () => {
  for (const name of ["quality.sh", "package.sh", "quality.ps1", "package.ps1"]) {
    const source = readFileSync(new URL(`../script/${name}`, import.meta.url), "utf8");
    assert.ok(source.indexOf("build-lock.mjs") < source.indexOf("stage-build.mjs"), name);
    assert.match(source, /CILAI_BUILD_LOCK_TOKEN/);
  }
});
