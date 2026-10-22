import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { closeSync, existsSync, mkdirSync, openSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve, sep } from "node:path";
import { assertPersonalBuildPath, personalBuildRoot } from "./personal-build.mjs";
import { fileURLToPath } from "node:url";

// 固定官方 MinGit 版本与发布页校验值，完整保留 DLL、许可证及第三方声明。
export const gitRuntime = Object.freeze({
  version: "2.56.0.windows.2",
  archive: "MinGit-2.56.0.2-64-bit.zip",
  download: "https://api.github.com/repos/git-for-windows/git/releases/assets/613673731?download=1",
  sha256: "da35e72aa21c005a5a0d298cfbae110bc1609a815730ea0dde84b01a1b3cd3be",
  release: "https://github.com/git-for-windows/git/releases/tag/v2.56.0.windows.2",
  source: "https://github.com/git-for-windows/git/archive/refs/tags/v2.56.0.windows.2.tar.gz",
});

export function verifyGitArchive(bytes) {
  if (createHash("sha256").update(bytes).digest("hex") !== gitRuntime.sha256) {
    throw new Error("MinGit 官方归档 SHA-256 校验失败");
  }
}

export async function prepareGitRuntime(sidecar, cache = join(personalBuildRoot(), "target/downloads/git")) {
  if (process.platform !== "win32" || process.arch !== "x64") throw new Error("MinGit 必须在 Windows x64 原生暂存");
  assertPersonalBuildPath(sidecar);
  assertPersonalBuildPath(cache);
  mkdirSync(cache, { recursive: true });
  const lockPath = join(cache, `${gitRuntime.archive}.lock`);
  let lock;
  try {
    lock = openSync(lockPath, "wx");
  } catch (error) {
    throw new Error(`无法独占准备 Git 运行时，请检查其他构建或遗留锁 ${lockPath}：${error.message}`);
  }
  try {
    const archive = join(cache, gitRuntime.archive);
    if (!existsSync(archive)) {
      // 使用 Windows 自带 curl，兼容本机构建网络；任何下载或校验失败都终止打包。
      console.log(`正在下载固定版本 ${gitRuntime.archive}，支持续传，完成后校验 SHA-256`);
      execFileSync("curl.exe", ["--fail", "--location", "--silent", "--show-error", "--connect-timeout", "20",
        "--retry", "2", "--max-time", "600", "--continue-at", "-",
        "--header", "Accept: application/octet-stream",
        "--output", `${archive}.pending`,
        gitRuntime.download], {
        windowsHide: true, stdio: "inherit",
      });
      verifyGitArchive(readFileSync(`${archive}.pending`));
      renameSync(`${archive}.pending`, archive);
    }
    verifyGitArchive(readFileSync(archive));
    // 每次在新目录解压，防止上次版本的 DLL 混入当前运行时。
    const extracted = join(sidecar, `git-extract-${crypto.randomUUID()}`);
    mkdirSync(extracted, { recursive: true });
    execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
      "$ErrorActionPreference = 'Stop'; [Reflection.Assembly]::LoadWithPartialName('System.IO.Compression.FileSystem') | Out-Null; [IO.Compression.ZipFile]::ExtractToDirectory($env:CILAI_MINGIT_ARCHIVE, $env:CILAI_MINGIT_DESTINATION)"], {
      windowsHide: true, stdio: "inherit",
      env: { ...process.env, CILAI_MINGIT_ARCHIVE: resolve(archive), CILAI_MINGIT_DESTINATION: resolve(extracted) },
    });
    if (!existsSync(join(extracted, "LICENSE.txt"))) throw new Error("MinGit 缺少随包许可证");
    writeFileSync(join(extracted, "cilai-runtime.json"), `${JSON.stringify(gitRuntime, null, 2)}\n`);
    const target = resolve(sidecar, "git");
    if (!target.startsWith(`${resolve(sidecar)}${sep}`)) throw new Error("Git 暂存目标越界");
    if (existsSync(target)) {
      // 解压及运行校验成功后，才替换已核对位于 sidecar 内的旧暂存目录。
      rmSync(target, { recursive: true, force: true });
    }
    renameSync(extracted, target);
    // Windows 上先落定目录，再执行其中的程序，避免映像句柄阻止目录更名。
    const version = execFileSync(join(target, "cmd", "git.exe"), ["--version"], { encoding: "utf8", windowsHide: true }).trim();
    if (version !== `git version ${gitRuntime.version}`) throw new Error(`MinGit 版本不匹配：${version}`);
    console.log(`MinGit ${gitRuntime.version} 已校验并暂存到 ${target}`);
  } finally {
    closeSync(lock);
    rmSync(lockPath);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (!process.argv[2]) throw new Error("缺少 sidecar 暂存目录");
  await prepareGitRuntime(resolve(process.argv[2]));
}
