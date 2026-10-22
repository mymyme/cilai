const { execFileSync } = require("node:child_process");
const { readFileSync } = require("node:fs");
const path = require("node:path");

/** 只配置业务服务的子进程环境；不修改系统或用户 PATH。 */
function gitRuntimeEnvironment(binaryPath, source = process.env, platform = process.platform) {
  const env = { ...source };
  if (platform !== "win32") return env;
  const binaryDirectory = path.win32.dirname(binaryPath);
  const profile = path.win32.basename(binaryDirectory);
  const directory = ["debug", "release"].includes(profile)
    && path.win32.basename(path.win32.dirname(binaryDirectory)) === "x86_64-pc-windows-msvc"
    ? path.win32.join(binaryDirectory, "..", "..", "sidecar", profile, "git")
    : path.win32.join(binaryDirectory, "git");
  let runtime;
  try {
    runtime = JSON.parse(readFileSync(path.win32.join(directory, "cilai-runtime.json"), "utf8"));
    const version = execFileSync(path.win32.join(directory, "cmd", "git.exe"), ["--version"], {
      windowsHide: true, encoding: "utf8", timeout: 10_000,
    }).trim();
    if (typeof runtime.version !== "string" || version !== `git version ${runtime.version}`) {
      throw new Error("随包 Git 版本与清单不一致");
    }
  } catch (error) {
    throw new Error(`工作台 Git 运行时不可用，请修复或重新安装客户端：${error.message}`);
  }
  const key = Object.keys(env).find((name) => name.toLowerCase() === "path");
  const inherited = key ? env[key] : "";
  for (const name of Object.keys(env)) if (name.toLowerCase() === "path") delete env[name];
  env.Path = [path.win32.join(directory, "cmd"), inherited].filter(Boolean).join(";");
  env.CILAI_MEMORY_GIT_RUNTIME = `mingit-${runtime.version}`;
  return env;
}

module.exports = { gitRuntimeEnvironment };
