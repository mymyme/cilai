import { spawnSync } from "node:child_process";
import { dirname, join, posix, win32 } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { assertDebugPackageLocation, canonicalDebugBuildRoot } from "./debug-package-location.mjs";

const here = dirname(fileURLToPath(import.meta.url));

export function prepareInvocation({ platform, profile, projectRoot }) {
  if (profile !== "debug" && profile !== "release") {
    throw new Error(`构建配置只接受 debug 或 release，收到：${profile}`);
  }
  if (platform === "darwin") {
    return {
      command: "bash",
      args: [posix.join(projectRoot, "script", "prepare_sidecar.sh"), profile],
    };
  }
  if (platform === "win32") {
    return {
      command: "powershell.exe",
      args: [
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", win32.join(projectRoot, "script", "prepare_sidecar.ps1"),
        "-BuildProfile", profile,
      ],
    };
  }
  throw new Error(`桌面打包暂不支持当前平台：${platform}`);
}

export function frontendBuildInvocation({ platform, projectRoot }) {
  return {
    command: platform === "win32" ? "cmd.exe" : "npm",
    args: platform === "win32" ? ["/d", "/s", "/c", "npm.cmd run build"] : ["run", "build"],
    cwd: projectRoot,
  };
}

function runChecked(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, env: process.env, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function main() {
  const profileArgument = process.argv.indexOf("--profile");
  const profile = profileArgument === -1 ? "debug" : process.argv[profileArgument + 1];
  const projectRoot = join(here, "..", "..", "..");
  const buildRoot = process.env.CILAI_BUILD_ROOT?.trim() || canonicalDebugBuildRoot();
  assertDebugPackageLocation({ profile, projectRoot, buildRoot });
  runChecked(process.execPath, [join(here, "prepare-electron-runtime.mjs")], projectRoot);
  const frontend = frontendBuildInvocation({ platform: process.platform, projectRoot });
  const prepare = prepareInvocation({ platform: process.platform, profile, projectRoot });

  runChecked(frontend.command, frontend.args, frontend.cwd);
  runChecked(prepare.command, prepare.args, projectRoot);
  runChecked(
    process.execPath,
    [join(here, "stage-binaries.mjs"), "--profile", profile],
    projectRoot,
  );
}

const entry = process.argv[1] ? pathToFileURL(process.argv[1]).href : null;
if (entry === import.meta.url) main();
