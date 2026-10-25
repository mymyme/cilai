import { assertPersonalBuildPath } from "./personal-build.mjs";
import { cpSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { validateToolchain } from "./check-toolchain.mjs";

export function validatedPersonalNode(runtime = {
  nodeVersion: process.versions.node, platform: process.platform, arch: process.arch, executable: process.execPath,
}) {
  validateToolchain(runtime);
  const triples = { "darwin/arm64": "aarch64-apple-darwin", "darwin/x64": "x86_64-apple-darwin", "win32/x64": "x86_64-pc-windows-msvc" };
  const triple = triples[`${runtime.platform}/${runtime.arch}`];
  const suffix = runtime.platform === "win32" ? ".exe" : "";
  const license = runtime.platform === "win32" ? join(dirname(runtime.executable), "LICENSE") : join(dirname(runtime.executable), "..", "LICENSE");
  if (!existsSync(license) || !readFileSync(license, "utf8").includes("Node.js")) throw new Error("缺少 Node.js 随包许可证");
  return { executable: runtime.executable, license, triple, suffix };
}

export function preparePersonalNode(directory, name) {
  if (!directory) throw new Error("受管 Node 暂存参数无效");
  const { executable, license, triple, suffix } = validatedPersonalNode();
  if (name !== `cilai-node-${triple}${suffix}`) throw new Error("受管 Node 目标平台或架构不匹配");
  assertPersonalBuildPath(directory);
  mkdirSync(resolve(directory), { recursive: true });
  cpSync(executable, join(directory, name));
  cpSync(license, join(directory, "node-LICENSE.txt"));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 4) throw new Error("受管 Node 暂存参数无效");
  preparePersonalNode(...process.argv.slice(2));
}
