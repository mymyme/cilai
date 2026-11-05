import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync, mkdirSync, readdirSync, mkdtempSync, cpSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { spawnSync } from "node:child_process";

const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
function hash(bytes) { return createHash("sha256").update(bytes).digest("hex"); }
const sharedExports = {
  "./conversation": "./src/conversation/index.tsx",
  "./page-notice": "./src/page-notice/index.tsx",
};
const sharedDirectories = ["src/conversation", "src/page-notice"];
function files(root, prefix) {
  return readdirSync(join(root, prefix), { withFileTypes: true }).flatMap(entry => entry.isDirectory()
    ? files(root, `${prefix}/${entry.name}`) : [`${prefix}/${entry.name}`]).sort();
}
function sourceHashes(root) {
  // Git can check out the same text as CRLF on Windows. Archive integrity stays
  // byte-exact; source identity uses the repository's canonical LF text.
  return Object.fromEntries(["package.json", "LICENSE", "NOTICE", ...sharedDirectories.flatMap(directory => files(root, directory))].map(file => [file, hash(readFileSync(join(root, file), "utf8").replace(/\r\n/g, "\n"))]));
}
export function verifyConversationPackage(root = sourceRoot) {
  const manifest = JSON.parse(readFileSync(join(root, "packages/conversation-package.json"), "utf8"));
  if (manifest.name !== "@cilai/app-ui" || !/^cilai-app-ui-[0-9.]+\.tgz$/.test(manifest.archive)
    || hash(readFileSync(join(root, "packages", manifest.archive))) !== manifest.sha256) throw new Error("共享对话组件包校验失败");
  const shared = join(root, "packages/app-ui");
  if (!existsSync(join(shared, "package.json")) || JSON.stringify(sourceHashes(shared)) !== JSON.stringify(manifest.sources)) {
    throw new Error("共享组件源码已改变，请先运行 npm run sync:conversation-package");
  }
  return manifest;
}
function sync() {
  const shared = join(sourceRoot, "packages/app-ui");
  const directory = join(sourceRoot, "packages"); mkdirSync(directory, { recursive: true });
  const npm = process.platform === "win32" ? "npm.cmd" : "npm";
  const temporary = mkdtempSync(join(tmpdir(), "cilai-conversation-package-"));
  let packed;
  try {
    const metadata = JSON.parse(readFileSync(join(shared, "package.json"), "utf8"));
    for (const [name, entry] of Object.entries(sharedExports)) {
      if (metadata.exports?.[name] !== entry) throw new Error(`桌面共享组件入口无效：${name}`);
    }
    writeFileSync(join(temporary, "package.json"), JSON.stringify({ ...metadata, exports: sharedExports, files: [...sharedDirectories, "LICENSE", "NOTICE"] }, null, 2) + "\n");
    for (const directory of sharedDirectories) cpSync(join(shared, directory), join(temporary, directory), { recursive: true });
    for (const name of ["LICENSE", "NOTICE"]) cpSync(join(shared, name), join(temporary, name));
    const result = spawnSync(npm, ["pack", "--json", "--ignore-scripts", "--pack-destination", directory], { cwd: temporary, encoding: "utf8", shell: process.platform === "win32" });
    if (result.status !== 0) throw new Error(result.stderr || "共享对话组件打包失败");
    [packed] = JSON.parse(result.stdout);
  } finally { rmSync(temporary, { recursive: true, force: true }); }
  writeFileSync(join(directory, "conversation-package.json"), JSON.stringify({ name: packed.name, version: packed.version, archive: packed.filename,
    sha256: hash(readFileSync(join(directory, packed.filename))), sources: sourceHashes(shared) }, null, 2) + "\n");
  const lockFile=join(sourceRoot,"package-lock.json");
  if(existsSync(lockFile)){
    const lock=JSON.parse(readFileSync(lockFile,"utf8"));
    const record=lock.packages?.["node_modules/@cilai/app-ui"];
    if(record){record.integrity=`sha512-${createHash("sha512").update(readFileSync(join(directory,packed.filename))).digest("base64")}`;record.license="Apache-2.0";writeFileSync(lockFile,JSON.stringify(lock,null,2)+"\n");}
  }
  verifyConversationPackage();
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv[2] === "--sync") sync(); else verifyConversationPackage();
}
