import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve, sep } from "node:path";
import test from "node:test";
import { gitRuntime, prepareGitRuntime, verifyGitArchive } from "./prepare-git-runtime.mjs";

test("校验失败的 Git 归档不能进入随包运行时", () => {
  assert.throws(() => verifyGitArchive(Buffer.from("incomplete or modified archive")), /SHA-256 校验失败/);
});

test("并发准备 Git 在下载前明确失败，不重复写入归档", { skip: process.platform !== "win32" || process.arch !== "x64" }, async () => {
  const cache = mkdtempSync(join(tmpdir(), "cilai-git-lock-"));
  assert.ok(resolve(cache).startsWith(`${resolve(tmpdir())}${sep}`));
  try {
    writeFileSync(join(cache, `${gitRuntime.archive}.lock`), "other preparation");
    await assert.rejects(prepareGitRuntime(join(cache, "sidecar"), cache), /无法独占准备 Git 运行时/);
  } finally {
    rmSync(cache, { recursive: true, force: true });
  }
});
