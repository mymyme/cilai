import { createHash, randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, lstatSync, mkdirSync, readFileSync, readlinkSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { assertPersonalBuildPath, personalBuildRoot } from "./personal-build.mjs";
import { isMachO, verifyMacSignature } from './macos-binary-integrity.mjs';

const project = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const codexRelease = JSON.parse(readFileSync(join(project, "third_party/openai-codex.release.json")));
const noticeManifestName = 'cilai-codex-notices.json';
export function verifyCodexArtifact(bytes, expected) {
  if (!/^[a-f0-9]{64}$/.test(expected) || createHash("sha256").update(bytes).digest("hex") !== expected) {
    throw new Error("Codex 官方发行件 SHA-256 校验失败");
  }
}
export function verifyCodexSource(root, manifest) {
  const expected = new Set(manifest.files.map((file) => file.path));
  function walk(directory, prefix = "") {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const name = prefix + entry.name;
      if (entry.isDirectory()) walk(join(directory, entry.name), name + "/");
      else if (!expected.has(name)) throw new Error("Codex 官方源码出现额外文件：" + name);
    }
  }
  walk(root);
  for (const file of manifest.files) {
    const input = join(root, file.path);
    const link = lstatSync(input).isSymbolicLink();
    if (link !== (file.mode === "120000")) throw new Error("Codex 官方源码链接类型改变：" + file.path);
    const bytes = link ? Buffer.from(readlinkSync(input)) : readFileSync(input);
    const hash = createHash("sha1").update("blob " + bytes.length + "\0").update(bytes).digest("hex");
    if (hash !== file.sha) throw new Error("Codex 官方源码已改变：" + file.path);
  }
  return manifest.files.length;
}

export function codexRuntimeTarget({ platform = process.platform, arch = process.arch } = {}) {
  if (platform === 'win32' && arch === 'x64') return 'x86_64-pc-windows-msvc';
  if (platform === 'darwin' && arch === 'arm64') return 'aarch64-apple-darwin';
  if (platform === 'darwin' && arch === 'x64') return 'x86_64-apple-darwin';
  throw new Error(`当前平台没有登记官方 Codex 发行件：${platform}/${arch}`);
}

function regularBytes(file) {
  if (!lstatSync(file).isFile()) throw new Error('Codex 许可通知必须为普通文件');
  return readFileSync(file);
}

function sourceBytes(relative) {
  let directory = project;
  const parts = relative.split('/');
  for (const part of parts.slice(0, -1)) {
    directory = join(directory, part);
    if (!lstatSync(directory).isDirectory()) throw new Error('Codex 许可通知来源目录不可使用链接');
  }
  return regularBytes(join(project, relative));
}

function pinnedNotices() {
  const manifestPath = `third_party/openai-codex-notices/${codexRelease.tag}/provenance.json`;
  if (codexRelease.notices?.manifest !== manifestPath) throw new Error('Codex 许可通知清单路径不匹配');
  const bytes = sourceBytes(manifestPath);
  verifyCodexArtifact(bytes, codexRelease.notices.sha256);
  const manifest = JSON.parse(bytes);
  if (manifest.version !== codexRelease.version || manifest.tag !== codexRelease.tag
      || manifest.tagObject !== codexRelease.tagObject || !/^[a-f0-9]{40}$/.test(manifest.sourceCommit)
      || !/^[a-f0-9]{40}$/.test(manifest.sourceTree) || !Array.isArray(manifest.files)
      || manifest.files.map(file => file.path).sort().join(',') !== 'LICENSE,NOTICE') {
    throw new Error('Codex 许可通知来源与发行件不匹配');
  }
  for (const file of manifest.files) {
    if (file.sourcePath !== `third_party/openai-codex-notices/${manifest.tag}/${file.path}`
        || file.sourceUrl !== `https://raw.githubusercontent.com/openai/codex/${manifest.sourceCommit}/${file.path}`
        || !/^[a-f0-9]{40}$/.test(file.upstreamBlobSha1) || !/^[a-f0-9]{64}$/.test(file.sha256)) {
      throw new Error('Codex 许可通知文件来源不匹配');
    }
  }
  return { bytes, manifest };
}

function verifyNotice(bytes, file) {
  verifyCodexArtifact(bytes, file.sha256);
  const blob = createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex');
  if (blob !== file.upstreamBlobSha1) throw new Error('Codex 许可通知上游文件摘要不匹配');
}

export function prepareCodexNotices(root) {
  if (!lstatSync(root).isDirectory()) throw new Error('Codex 许可通知目标目录不可使用链接');
  const { bytes, manifest } = pinnedNotices();
  const inputs = manifest.files.map(file => {
    const content = sourceBytes(file.sourcePath);
    verifyNotice(content, file);
    return { name: file.path, content };
  });
  inputs.push({ name: noticeManifestName, content: bytes });
  for (const { name, content } of inputs) {
    const destination = join(root, name);
    if (existsSync(destination)) {
      if (!regularBytes(destination).equals(content)) throw new Error('Codex 许可通知与固定来源不一致');
    } else {
      writeFileSync(destination, content, { flag: 'wx', mode: 0o644 });
    }
  }
  verifyCodexNotices(root);
}

export function verifyCodexNotices(root) {
  if (!lstatSync(root).isDirectory()) throw new Error('Codex 许可通知目标目录不可使用链接');
  const { manifest } = pinnedNotices();
  verifyCodexArtifact(regularBytes(join(root, noticeManifestName)), codexRelease.notices.sha256);
  for (const file of manifest.files) verifyNotice(regularBytes(join(root, file.path)), file);
  return manifest;
}

function verifyCodexPackage(root, target) {
  const artifact = codexRelease.packages[target];
  if (!artifact) throw new Error('未登记的 Codex 官方发行目标');
  const windows = target === 'x86_64-pc-windows-msvc';
  const description = JSON.parse(readFileSync(join(root, 'codex-package.json'), 'utf8'));
  if (description.layoutVersion !== 1 || description.version !== codexRelease.version || description.target !== target
    || description.variant !== 'codex-app-server' || description.entrypoint !== `bin/codex-app-server${windows ? '.exe' : ''}`
    || description.resourcesDir !== 'codex-resources' || description.pathDir !== 'codex-path') {
    throw new Error('Codex 官方包版本、平台或目录清单不匹配');
  }
  for (const directory of ['bin', description.resourcesDir, description.pathDir]) {
    if (!lstatSync(join(root, directory)).isDirectory()) throw new Error(`Codex 官方包缺少目录：${directory}`);
  }
  verifyCodexArtifact(readFileSync(join(root, description.entrypoint)), artifact.executableSha256);
  function inspect(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const file = join(directory, entry.name);
      if (entry.isDirectory()) inspect(file);
      else if (!entry.isFile()) throw new Error(`Codex 官方包包含非普通文件：${file}`);
      else if (!windows && isMachO(file)) verifyMacSignature(file);
    }
  }
  inspect(root);
  return description;
}

export function verifyCodexRuntime(root, target) {
  const description = verifyCodexPackage(root, target);
  verifyCodexNotices(root);
  return description;
}

export function prepareCodexRuntime(sidecar, cache = join(personalBuildRoot(), "target/downloads/codex")) {
  assertPersonalBuildPath(sidecar);
  assertPersonalBuildPath(cache);
  // The released binary is independent of the retained legacy source snapshot.
  // Verify the pinned archive, package metadata and executable below; never modify
  // or compile third_party/openai-codex to prepare the official runtime.
  const target = codexRuntimeTarget();
  const artifact = codexRelease.packages[target];
  const root = resolve(sidecar);
  mkdirSync(root, { recursive: true });
  mkdirSync(cache, { recursive: true });
  const archive = join(cache, artifact.archive);
  if (!existsSync(archive)) {
    execFileSync(process.platform === "win32" ? "curl.exe" : "curl", ["--fail", "--location", "--silent", "--show-error",
      "--connect-timeout", "20", "--max-time", "900", "--retry", "2", "--continue-at", "-", "--header", "Accept: application/octet-stream", "--output", archive + ".pending", artifact.url],
    { windowsHide: true, stdio: "inherit" });
    verifyCodexArtifact(readFileSync(archive + ".pending"), artifact.sha256);
    renameSync(archive + ".pending", archive);
  }
  verifyCodexArtifact(readFileSync(archive), artifact.sha256);
  const extracted = join(root, "codex-extract-" + randomUUID());
  mkdirSync(extracted);
  try {
    const entries = execFileSync("tar", ["-tf", archive], { encoding: "utf8", windowsHide: true }).trim().split(/\r?\n/);
    if (entries.some((entry) => entry.startsWith("/") || entry.includes("\\") || entry.split("/").some((part) => part === ".."))) throw new Error("Codex 官方归档目录越界");
    const metadataEntries = entries.filter((entry) => entry === "codex-package.json" || entry.endsWith("/codex-package.json"));
    if (metadataEntries.length !== 1) throw new Error("Codex 官方归档必须包含唯一包清单");
    execFileSync("tar", ["-xf", archive, "-C", extracted], { windowsHide: true, stdio: "inherit" });
    const packageRoot = dirname(join(extracted, metadataEntries[0]));
    verifyCodexPackage(packageRoot, target);
    prepareCodexNotices(packageRoot);
    verifyCodexRuntime(packageRoot, target);
    // Keep the complete upstream package layout so newly added helpers are included.
    const installed = join(root, "codex-runtime");
    if (!resolve(installed).startsWith(root + sep)) throw new Error("Codex 暂存目标越界");
    if (existsSync(installed)) rmSync(installed, { recursive: true, force: true });
    renameSync(packageRoot, installed);
    console.log("官方 Codex " + codexRelease.version + " 完整发行包已校验并暂存");
  } finally {
    if (!resolve(extracted).startsWith(root + sep)) throw new Error("Codex 解压清理目标越界");
    rmSync(extracted, { recursive: true, force: true });
  }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (!process.argv[2]) throw new Error("缺少 sidecar 暂存目录");
  prepareCodexRuntime(process.argv[2], process.argv[3]);
}
