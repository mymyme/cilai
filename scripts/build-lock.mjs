import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { closeSync, lstatSync, mkdirSync, openSync, readFileSync, readlinkSync, unlinkSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { assertPersonalBuildRoot } from "./personal-build.mjs";
import { canonicalDebugBuildRoot } from "../apps/desktop/scripts/debug-package-location.mjs";

const lockName = ".cilai-build-lock.json";
const manifestName = ".cilai-staged-files.json";
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

export function assertBuildLock(root, token = process.env.CILAI_BUILD_LOCK_TOKEN) {
  const file = join(root, lockName);
  if (!token || !lstatSync(file).isFile() || lstatSync(file).isSymbolicLink()) throw new Error("构建锁无效，已停止操作");
  const owner = JSON.parse(readFileSync(file, "utf8"));
  if (owner.version !== 1 || owner.token !== token) throw new Error("构建目录正由另一任务使用，已停止操作");
  return owner;
}

export function acquireBuildLock(root, command) {
  mkdirSync(root, { recursive: true });
  const token = randomUUID();
  let fd;
  try {
    fd = openSync(join(root, lockName), "wx", 0o600);
  } catch (error) {
    if (error.code === "EEXIST") throw new Error("共用构建目录已被占用；请等待当前构建结束。异常退出留下的锁须核对进程后手动处理");
    throw error;
  }
  try {
    writeFileSync(fd, JSON.stringify({ version: 1, token, pid: process.pid, command, startedAt: new Date().toISOString() }));
  } finally { closeSync(fd); }
  return token;
}

export function recordStagedBuild(root, token = process.env.CILAI_BUILD_LOCK_TOKEN) {
  const owner = assertBuildLock(root, token);
  owner.manifestDigest = digest(readFileSync(join(root, manifestName)));
  writeFileSync(join(root, lockName), JSON.stringify(owner));
}

export function verifyStagedBuild(root, token) {
  const owner = assertBuildLock(root, token);
  if (!owner.manifestDigest) return;
  const bytes = readFileSync(join(root, manifestName));
  if (digest(bytes) !== owner.manifestDigest) throw new Error("构建期间源码清单被替换，本轮结果无效");
  for (const [name, expected] of Object.entries(JSON.parse(bytes).files)) {
    if (!name || name.startsWith("/") || name.includes("\\") || name.split("/").some((part) => !part || part === "." || part === "..")) throw new Error("构建源码清单包含非法路径");
    const file = join(root, name), stat = lstatSync(file);
    const actual = stat.isSymbolicLink() ? `link:${readlinkSync(file)}` : stat.isFile() ? `file:${digest(readFileSync(file))}` : "invalid";
    if (actual !== expected) throw new Error(`构建期间源码被修改，本轮结果无效：${name}`);
  }
}

export function releaseBuildLock(root, token) {
  assertBuildLock(root, token);
  unlinkSync(join(root, lockName));
}

export async function runLockedBuild(args, root = canonicalDebugBuildRoot(), environment = process.env) {
  if (!args.length) throw new Error("缺少构建命令");
  if (root === canonicalDebugBuildRoot()) assertPersonalBuildRoot(root);
  const inherited = environment.CILAI_BUILD_LOCK_TOKEN;
  const token = inherited ?? acquireBuildLock(root, args[0]);
  if (inherited) assertBuildLock(root, inherited);
  let child, interrupted = false;
  const forward = (signal) => { interrupted = true; child?.kill(signal); };
  const interrupt = () => forward("SIGINT"), terminate = () => forward("SIGTERM");
  process.on("SIGINT", interrupt); process.on("SIGTERM", terminate);
  try {
    const status = await new Promise((resolveStatus, reject) => {
      child = spawn(args[0], args.slice(1), { stdio: "inherit", env: { ...environment, CILAI_BUILD_LOCK_TOKEN: token, CILAI_BUILD_ROOT: root, CARGO_TARGET_DIR: join(root, "target") } });
      child.once("error", reject);
      child.once("exit", (code) => resolveStatus(code ?? 1));
    });
    if (interrupted) throw new Error("构建被中断；锁已保留，请核对所有构建子进程结束后再处理");
    if (status === 0) verifyStagedBuild(root, token);
    return status;
  } finally {
    process.off("SIGINT", interrupt); process.off("SIGTERM", terminate);
    if (!inherited && !interrupted) releaseBuildLock(root, token);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args[0] === "--check") assertBuildLock(canonicalDebugBuildRoot());
  else {
    if (args.shift() !== "--") throw new Error("构建锁入口需要 -- 命令");
    process.exitCode = await runLockedBuild(args);
  }
}
