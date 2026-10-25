import { assertBuildLock, recordStagedBuild, runLockedBuild } from "./build-lock.mjs";
import { verifyConversationPackage } from "./conversation-package.mjs";
import { assertNoSymlinkPath, assertPersonalBuildRoot, personalSourceRoot, registerPersonalBuild } from "./personal-build.mjs";
import { createHash } from "node:crypto";
import {
  chmodSync, copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, realpathSync,
  readlinkSync, readdirSync, renameSync, rmSync, symlinkSync, writeFileSync,
} from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { canonicalDebugBuildRoot } from "../apps/desktop/scripts/debug-package-location.mjs";

const manifestName = ".cilai-staged-files.json";
const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function contentId(file) {
  const info = lstatSync(file);
  if (info.isSymbolicLink()) return `link:${readlinkSync(file)}`;
  if (!info.isFile()) throw new Error(`暂存来源不是普通文件：${file}`);
  return `file:${createHash("sha256").update(readFileSync(file)).digest("hex")}`;
}

function pathExists(file) {
  try { lstatSync(file); return true; } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

function assertContainedSourceLink(root, file) {
  const target = readlinkSync(file);
  if (target.startsWith('/') || /^[A-Za-z]:/.test(target)) throw new Error(`暂存源码不能包含绝对符号链接：${file}`);
  const resolved = realpathSync(file);
  if (!resolved.startsWith(`${realpathSync(root)}${sep}`) || !lstatSync(resolved).isFile()) throw new Error(`暂存源码符号链接越界或不是文件：${file}`);
}

function validName(name) {
  return typeof name === "string" && name.length > 0 && name !== manifestName
    && !name.startsWith("/") && !name.includes("\\")
    && name.split("/").every((part) => part !== "" && part !== "." && part !== "..");
}

const historicalRuntimeInputs = new Set([
  'third_party/openai-codex/LICENSE',
  'third_party/openai-codex/NOTICE',
  'third_party/openai-codex/codex-rs/models-manager/models.json',
]);

export function sourceFiles(root, { profile = 'desktop' } = {}) {
  if (!['desktop', 'native-extensions'].includes(profile)) throw new Error(`未知暂存用途：${profile}`);
  const ignored = new Set(['.git', '.venv', 'node_modules', 'target', 'dist', '.vite', '.personal-build', '.codex', '.aws', '.agents', '__pycache__']);
  function scan(directory, prefix = '') {
    return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
      const name = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (ignored.has(entry.name) || entry.name === '.DS_Store' || entry.name === '.env' || entry.name.startsWith('.env.') || name === '.cargo/config.toml') return [];
      if (profile === 'desktop' && name.startsWith('third_party/openai-codex/')
        && !historicalRuntimeInputs.has(name)
        && ![...historicalRuntimeInputs].some(input => input.startsWith(`${name}/`))) return [];
      if (entry.isSymbolicLink()) { assertContainedSourceLink(root, join(directory, entry.name)); return [name]; }
      if (entry.isDirectory()) return scan(join(directory, entry.name), name);
      if (!entry.isFile()) throw new Error(`暂存来源不是普通文件：${name}`);
      return [name];
    });
  }
  assertNoSymlinkPath(root);
  return scan(root).sort();
}

export function assertPrimarySource(root = sourceRoot) {
  assertNoSymlinkPath(root);
  if (realpathSync(root) !== realpathSync(personalSourceRoot)) throw new Error('暂存来源必须是当前个人版源码目录');
}

export function stageBuild({ source = sourceRoot, destination, profile = 'desktop', files = sourceFiles(source, { profile }) }) {
  const from = resolve(source);
  const to = resolve(destination);
  if (from === to || to.startsWith(`${from}${sep}`) || from.startsWith(`${to}${sep}`)) {
    throw new Error("源码目录与构建目录不能相同或互相嵌套");
  }
  assertNoSymlinkPath(from);
  assertNoSymlinkPath(to);
  mkdirSync(to, { recursive: true });
  const manifestPath = join(to, manifestName);
  assertNoSymlinkPath(manifestPath);
  const previous = existsSync(manifestPath)
    ? JSON.parse(readFileSync(manifestPath, "utf8"))
    : { version: 1, files: {} };
  if (previous.version !== 1 || !previous.files || typeof previous.files !== "object"
    || Array.isArray(previous.files)
    || Object.entries(previous.files).some(([name, digest]) => !validName(name) || typeof digest !== "string")) {
    throw new Error("统一构建目录的源码清单无法识别，请先检查清单内容");
  }

  const next = Object.create(null);
  let copied = 0;
  let removed = 0;
  for (const name of files) {
    if (!validName(name)) {
      throw new Error(`源码清单包含非法路径：${name}`);
    }
    const input = join(from, name);
    const output = join(to, name);
    assertNoSymlinkPath(dirname(input));
    if (lstatSync(input).isSymbolicLink()) assertContainedSourceLink(from, input);
    assertNoSymlinkPath(dirname(output));
    if (pathExists(output) && lstatSync(output).isSymbolicLink()) assertContainedSourceLink(to, output);
    const digest = contentId(input);
    next[name] = digest;
    if (pathExists(output) && contentId(output) === digest) continue;
    mkdirSync(dirname(output), { recursive: true });
    if (pathExists(output)) rmSync(output);
    if (digest.startsWith("link:")) {
      symlinkSync(readlinkSync(input), output);
    } else {
      copyFileSync(input, output);
      chmodSync(output, lstatSync(input).mode);
    }
    copied += 1;
  }
  for (const [name, oldDigest] of Object.entries(previous.files)) {
    if (Object.hasOwn(next, name)) continue;
    const output = join(to, name);
    assertNoSymlinkPath(dirname(output));
    if (!pathExists(output)) continue;
    if (contentId(output) !== oldDigest) {
      throw new Error(`构建目录中的已移除源码被单独修改，需人工检查：${output}`);
    }
    rmSync(output);
    removed += 1;
  }
  const pending = `${manifestPath}.tmp`;
  assertNoSymlinkPath(pending);
  writeFileSync(pending, `${JSON.stringify({ version: 1, files: next })}\n`);
  renameSync(pending, manifestPath);
  return { copied, removed, total: files.length };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (!process.env.CILAI_BUILD_LOCK_TOKEN) {
    process.exitCode = await runLockedBuild([process.execPath, ...process.argv.slice(1)]);
  } else {
  const canonical = assertPersonalBuildRoot(canonicalDebugBuildRoot());
  assertBuildLock(canonical);
  registerPersonalBuild();
  const args = process.argv.slice(2);
  let profile = 'desktop';
  const profileIndex = args.indexOf('--profile');
  if (profileIndex !== -1) {
    profile = args[profileIndex + 1];
    if (!['desktop', 'native-extensions'].includes(profile)) throw new Error('暂存用途必须为 desktop 或 native-extensions');
    args.splice(profileIndex, 2);
  }
  if (args.length > 1 || args.some(arg => arg.startsWith('--'))) throw new Error('暂存参数无法识别');
  const destination = args[0] ? resolve(args[0]) : canonical;
  if (destination !== resolve(canonical)) throw new Error(`只能暂存到统一构建目录：${canonical}`);
  if (process.version !== "v24.18.0") throw new Error(`构建需要 Node 24.18.0，当前为 ${process.version}`);
  if (resolve(sourceRoot) === destination) {
    console.log("源码已在统一构建目录");
  } else {
    assertPrimarySource();
    verifyConversationPackage();
    const result = stageBuild({ destination, profile });
    console.log(`源码暂存完成：更新 ${result.copied} 项，移除 ${result.removed} 项，共 ${result.total} 项`);
  }
  recordStagedBuild(canonical);
  }
}
