import { join, resolve } from "node:path";
import { setTimeout } from "node:timers/promises";

const installed = new WeakSet();

// The builder edits PE resources after afterPack. Retrying the whole package
// unpacks the executable again and can repeat the same transient Windows lock.
// Retain the builder's complete resource/signing operation and retry only an
// unsuccessful open of this package's main executable; no file was truncated.
export function installWindowsResourceRetry(context, { wait = setTimeout, warn = console.warn } = {}) {
  if (context.electronPlatformName !== "win32") return;
  const packager = context.packager;
  if (installed.has(packager)) return;
  const edit = packager.signAndEditResources;
  if (typeof edit !== "function") {
    throw new Error("Windows 打包器资源编辑接口不可用，请核对锁定的 electron-builder 版本");
  }
  const executable = resolve(join(context.appOutDir, `${packager.appInfo.productFilename}.exe`)).toLowerCase();
  packager.signAndEditResources = async function (...args) {
    for (let attempt = 1; ; attempt += 1) {
      try {
        return await edit.apply(this, args);
      } catch (error) {
        const locked = error.code === "EBUSY" && error.syscall === "open"
          && typeof error.path === "string" && resolve(error.path).toLowerCase() === executable
          && typeof args[0] === "string" && resolve(args[0]).toLowerCase() === executable;
        if (!locked || attempt >= 5) throw error;
        warn(`Windows 主程序资源写入暂时被占用，${attempt} 秒后重试资源编辑（${attempt}/4）`);
        await wait(attempt * 1000);
      }
    }
  };
  installed.add(packager);
}
