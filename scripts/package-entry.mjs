import { personalBuildChannel } from "./personal-build.mjs";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
personalBuildChannel();
if (args.some(arg => ["--release", "--portable", "--online-test"].includes(arg))) throw new Error("个人版尚未配置发布身份和更新源；当前仅允许本地测试包");
if (args.some((arg) => arg !== "--launch")) {
  throw new Error(`不支持的打包参数：${args.join(" ")}`);
}
if (args.length > 1) {
  throw new Error("打包模式不可组合");
}
const command = process.platform === "darwin" ? "bash" : process.platform === "win32" ? "powershell.exe" : null;
if (!command) throw new Error(`桌面打包暂不支持当前平台：${process.platform}`);
const script = join(root, "script", process.platform === "darwin" ? "package.sh" : "package.ps1");
const result = spawnSync(command, process.platform === "darwin" ? [script, ...args] : [
  "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script,
  ...(args.includes("--launch") ? ["-Launch"] : []),
], { cwd: root, env: process.env, stdio: "inherit" });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
