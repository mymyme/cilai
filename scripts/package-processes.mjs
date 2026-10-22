import { spawnSync } from "node:child_process";
import path from "node:path";
import { pathToFileURL } from "node:url";

function packageProcesses(bundlePath, inspect) {
  const result = inspect("/bin/ps", ["-ax", "-o", "pid=,comm="], { encoding: "utf8" });
  if (result.error || result.status !== 0) throw new Error("无法检查目标包的运行进程，已停止打包");
  // `ps` 输出 POSIX 路径；用 posix 解析，Windows 上运行本脚本的测试时也不会被改写成盘符路径。
  const prefix = path.posix.resolve(bundlePath) + "/Contents/";
  const running = result.stdout.split("\n").flatMap(line => {
    const match = line.match(/^\s*(\d+)\s+(.+)$/);
    return match?.[2].startsWith(prefix) ? [Number(match[1])] : [];
  });
  return running;
}

export function assertPackageNotInUse(bundlePath, inspect = spawnSync) {
  const running = packageProcesses(bundlePath, inspect);
  if (running.length) throw new Error(`目标包仍有 ${running.length} 个运行进程，请完成正常退出后重新打包`);
  return running;
}

export async function waitForPackageExit(bundlePath, timeoutMs, inspect = spawnSync) {
  const deadline = Date.now() + timeoutMs;
  while (true) {
    const running = packageProcesses(bundlePath, inspect);
    if (!running.length) return;
    if (Date.now() >= deadline) throw new Error(`启动自检结束后仍有 ${running.length} 个随包进程未完成收尾`);
    await new Promise(resolve => setTimeout(resolve, 100));
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (!process.argv[2]) throw new Error("缺少目标包目录");
  if (process.argv[3]) {
    const value = process.argv[3].match(/^--wait-ms=(\d+)$/);
    if (!value || Number(value[1]) > 60000) throw new Error("退出等待参数无效");
    await waitForPackageExit(process.argv[2], Number(value[1]));
  } else assertPackageNotInUse(process.argv[2]);
}
