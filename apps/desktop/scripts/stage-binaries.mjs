// 打包前把随包二进制收集到暂存目录。
//
// 随包组件由 prepare_sidecar 产出在统一构建根内，打包配置无法以相对路径引用，因此先
// 集中到一个暂存目录再由打包配置纳入。暂存目录同样落在构建根：源码树在 iCloud 上，
// 几百兆的二进制放进去会被持续同步，而它们每次构建都会重新生成。
//
// 取件位置是 `target/sidecar/<配置>`：那是 script/prepare_sidecar.{sh,ps1} 的唯一产出口，
// 文件名带目标三元组后缀。业务侧按不带后缀的固定名在自身同目录查找，故落地时去掉后缀。
// 按配置分目录，正式暂存不会取到调试配置留下的同名产物。
// 不从 `target/<profile>` 取——应用服务恰好也落在那里，但 Codex Harness 与 PDF 渲染器
// 不会，按那里取件会在干净的构建根上取不到件。
import { cpSync, mkdirSync, existsSync, readdirSync, rmSync, statSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join, dirname, posix } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { assertNoSymlinkPath, assertPersonalBuildRoot } from "../../../scripts/personal-build.mjs";
import { canonicalDebugBuildRoot } from "./debug-package-location.mjs";
import { codexRuntimeTarget, verifyCodexRuntime } from '../../../scripts/prepare-codex-runtime.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const buildRoot = process.env.CILAI_BUILD_ROOT ?? canonicalDebugBuildRoot();

/** 暂存目录。打包配置经同名环境变量取到同一个位置，两处不各自推断。 */
export function stagingDirectory(root) {
  return join(root, "target", "desktop-sidecar");
}

/** 目标三元组取自编译器本身，与 prepare_sidecar 的取法一致，避免两处各自推断。 */
function hostTriple() {
  const version = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const line = version.split("\n").find((entry) => entry.startsWith("host: "));
  if (!line) throw new Error("rustc 未报告目标三元组");
  return line.slice("host: ".length).trim();
}

/**
 * 本平台的完整清单。
 *
 * 平台差异体现在清单本身，不体现在缺件时的处置：PDF 渲染器是 macOS 原生实现，另一端
 * 不存在，因此它根本不出现在该端的清单里；而清单内的每一项都必收。原先按"必收/缺省"
 * 分类，缺省项缺失就静默跳过——那会让平台差异与构建缺件混为一谈，装好之后才发现预览
 * 失效，构建期没有任何信号。
 */
export function buildManifest({ platform, profile, triple, buildRoot }) {
  const windows = platform === "win32";
  const suffix = windows ? ".exe" : "";
  const sidecarRoot = join(buildRoot, "target", "sidecar", profile);
  // 应用服务与 Codex Harness 由业务服务在自身同目录查找，缺任一项则服务拉不起来、对话开不了
  const items = [
    { from: join(sidecarRoot, `cilai-node-${triple}${suffix}`), to: `cilai-node${suffix}` },
    { from: join(sidecarRoot, "node-LICENSE.txt"), to: "node-LICENSE.txt" },
    {
      // 宿主服务由 cargo 直接产出。Windows 上 prepare_sidecar 传了 --target，产物多一层目录
      from: windows
        ? join(buildRoot, "target", triple, profile, `cilai-host-service${suffix}`)
        : join(buildRoot, "target", profile, `cilai-host-service${suffix}`),
      to: `cilai-host-service${suffix}`,
    },
    {
      from: join(sidecarRoot, `cilai-app-server-${triple}${suffix}`),
      to: `cilai-app-server${suffix}`,
    },
    {
      from: join(sidecarRoot, "codex-runtime"),
      to: "codex-runtime",
    },
  ];
  if (windows) {
    items.push({ from: join(sidecarRoot, "git"), to: "git" });
  } else {
    items.push({
      from: join(sidecarRoot, `cilai-pdf-renderer-${triple}`),
      to: "cilai-pdf-renderer",
    });
  }
  return items;
}

/** 拦截旧 Harness 和目录合并遗留；不删除现场，也不让残留进入新包。 */
export function assertStagingInventory(items, target) {
  const expected = new Set();
  function collect(directory, prefix) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const name = posix.join(prefix, entry.name);
      expected.add(name);
      if (entry.isDirectory()) collect(join(directory, entry.name), name);
      else if (!entry.isFile()) throw new Error(`随包来源包含非普通文件：${name}`);
    }
  }
  for (const item of items) {
    expected.add(item.to);
    if (statSync(item.from).isDirectory()) collect(item.from, item.to);
  }
  if (!existsSync(target)) return;
  const strays = [];
  function inspect(directory, prefix = "") {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (prefix === "" && entry.isDirectory() && entry.name.endsWith(".dSYM")) continue;
      const name = posix.join(prefix, entry.name);
      if (!expected.has(name)) strays.push(name);
      else if (entry.isDirectory()) inspect(join(directory, entry.name), name);
      else if (!entry.isFile()) strays.push(name);
    }
  }
  inspect(target);
  if (strays.length > 0) {
    throw new Error(`随包暂存目录存在清单外残留，请检查并移出后重试：\n${strays.join("\n")}`);
  }
}

/**
 * 被测试引入时不产生副作用：清单本身要能单独断言，而暂存会写文件。
 */
function main() {
  // 配置以命令行参数给出而不只靠环境变量：环境变量的写法在两端的命令行里不同，
  // 放进 npm 脚本会让其中一端失效。
  const profileArgument = process.argv.indexOf("--profile");
  const profile = profileArgument === -1
    ? (process.env.CILAI_PROFILE ?? "debug")
    : process.argv[profileArgument + 1];
  if (profile !== "debug" && profile !== "release") {
    console.error(`构建配置只接受 debug 或 release，收到：${profile}`);
    process.exit(1);
  }

  assertPersonalBuildRoot(buildRoot);
  const target = stagingDirectory(buildRoot);
  assertNoSymlinkPath(target);

  const triple = hostTriple();
  const windows = process.platform === "win32";
  const items = buildManifest({ platform: process.platform, profile, triple, buildRoot });
  const missing = items.filter((item) => !existsSync(item.from));
  if (missing.length > 0) {
    for (const item of missing) console.error(`缺少二进制：${item.from}`);
    console.error(
      `\n随包组件由 script/prepare_sidecar.${windows ? "ps1" : "sh"} 产出，且只能在统一`
      + `构建根 ${buildRoot} 内执行。先在那里以 ${profile} 配置构建，再重试本步骤。`);
    process.exit(1);
  }
  if (windows) {
    const git = items.find((item) => item.to === "git").from;
    for (const name of ["cmd/git.exe", "LICENSE.txt", "cilai-runtime.json"]) {
      const file = join(git, name);
      if (!existsSync(file) || !statSync(file).isFile() || statSync(file).size === 0) {
        throw new Error(`Git 运行时缺件：${file}`);
      }
    }
  }

  verifyCodexRuntime(items.find(item => item.to === 'codex-runtime').from, codexRuntimeTarget());
  assertStagingInventory(items, target);
  for (const item of items) {
    assertNoSymlinkPath(item.from);
    const destination = join(target, item.to);
    mkdirSync(dirname(destination), { recursive: true });
    cpSync(item.from, destination, { recursive: true, force: true });
  }

  for (const item of items) {
    const size = (statSync(join(target, item.to)).size / 1024 / 1024).toFixed(1);
    console.log(`  ${item.to.padEnd(34)} ${size.padStart(7)} MB`);
  }
  console.log(`已按 ${profile} 配置暂存 ${items.length} 个随包组件到 ${target}`);
}

// argv[1] 在以 -e 求值时不存在，取值前先确认，否则引入本模块本身就会抛错
const entry = process.argv[1] ? pathToFileURL(process.argv[1]).href : null;
if (entry === import.meta.url) main();
