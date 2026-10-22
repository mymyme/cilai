import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { canonicalDebugBuildRoot } from "../apps/desktop/scripts/debug-package-location.mjs";
import { assertBuildLock, runLockedBuild } from "./build-lock.mjs";

const source = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const root = canonicalDebugBuildRoot();
const native = join(root, "third_party/openai-codex/codex-rs");
const args = process.argv.slice(2);
let filter, report, workspace = false;
while (args.length) {
  const arg = args.shift();
  if (arg === "--filter") filter = args.shift();
  else if (arg === "--report") report = args.shift();
  else if (arg === "--workspace") workspace = true;
  else throw new Error(`未知参数：${arg}`);
  if ((arg === "--filter" && !filter) || (arg === "--report" && !report)) throw new Error(`${arg} 缺少值`);
}

async function run(command, argv, cwd, env = process.env) {
  const status = await new Promise((done, fail) => {
    const child = spawn(command, argv, { cwd, env, stdio: "inherit" });
    child.once("error", fail);
    child.once("exit", (code) => done(code ?? 1));
  });
  if (status !== 0) throw new Error(`${command} 退出码 ${status}`);
}

async function main() {
  if (process.version !== "v24.18.0") throw new Error("原生扩展测试需要 Node 24.18.0");
  assertBuildLock(root);
  const receiptDirectory = resolve(report ?? join(root, "target/native-extension-tests", new Date().toISOString().replaceAll(":", "-")));
  mkdirSync(receiptDirectory, { recursive: true });
  const receipt = { startedAt: new Date().toISOString(), workspace, filter: filter ?? null, nextestProfile: "default", automaticRetries: 0, userConfigurationChanged: false };
  let isolated;
  try {
    await run(process.execPath, [join(source, "scripts/stage-build.mjs")], source);
    receipt.stagedManifestSha256 = createHash("sha256").update(readFileSync(join(root, ".cilai-staged-files.json"))).digest("hex");
    const env = { ...process.env, CARGO_TARGET_DIR: join(root, "target/codex-harness"), CARGO_INCREMENTAL: "0", CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "4" };
    receipt.buildJobs = env.CARGO_BUILD_JOBS;
    if (process.platform === "darwin" && process.arch === "arm64") {
      env.V8_FROM_SOURCE = "1";
      if (!env.LIBCLANG_PATH) throw new Error("V8 源码构建需要显式设置 LIBCLANG_PATH，指向 Clang 19 或更新版本的动态库目录");
      const metadata = spawnSync("cargo", ["metadata", "--locked", "--format-version", "1"], { cwd: native, env, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
      if (metadata.error || metadata.status !== 0) {
        writeFileSync(join(receiptDirectory, "cargo-metadata.log"), metadata.stderr ?? metadata.error?.message ?? "");
        throw new Error("无法读取锁定的原生依赖元数据，详见 cargo-metadata.log");
      }
      const v8 = JSON.parse(metadata.stdout).packages.find((entry) => entry.name === "v8");
      if (!v8) throw new Error("原生依赖清单缺少 V8");
      await run("python3", [join(source, "scripts/prepare-v8-build-tools.py"), dirname(v8.manifest_path), join(env.CARGO_TARGET_DIR, "dev-small/ninja_gn_binaries"), join(receiptDirectory, "v8-build-tools.json")], native, env);
      receipt.v8 = { version: v8.version, build: "source", securityFeaturesUnchanged: true };
    }
    const helpers = workspace ? ["--workspace", "--features", "codex-v8-poc/sandbox", "--bins", "--examples"] : ["-p", "codex-app-server", "--bin", "codex-app-server", "-p", "codex-cli", "--bin", "codex", "-p", "codex-rmcp-client", "--bin", "test_stdio_server", "-p", "codex-code-mode-host", "--bin", "codex-code-mode-host"];
    receipt.workspaceFeatures = workspace ? ["codex-v8-poc/sandbox"] : [];
    await run("cargo", ["build", "--locked", "--profile", "dev-small", ...helpers], native, env);
    receipt.helpersBuilt = true;
    isolated = mkdtempSync(join(tmpdir(), "cilai-native-extensions-"));
    const testEnv = { ...env, CODEX_HOME: join(isolated, "config"), ZDOTDIR: join(isolated, "zsh"), HOME: join(isolated, "home"), USERPROFILE: join(isolated, "home"), CARGO_HOME: process.env.CARGO_HOME ?? join(homedir(), ".cargo"), RUSTUP_HOME: process.env.RUSTUP_HOME ?? join(homedir(), ".rustup") };
    mkdirSync(testEnv.HOME);
    receipt.isolatedUserHome = true;
    receipt.removedTestTerminalVariables = ["NO_COLOR", "CLICOLOR", "CLICOLOR_FORCE", "FORCE_COLOR"].filter((name) => name in testEnv);
    for (const name of receipt.removedTestTerminalVariables) delete testEnv[name];
    testEnv.TERM = "xterm-256color";
    mkdirSync(testEnv.CODEX_HOME);
    mkdirSync(testEnv.ZDOTDIR);
    receipt.isolatedConfiguration = true;
    receipt.removedTestProxyVariables = Object.keys(testEnv).filter((name) => /^(http|https|all|no)_proxy$/i.test(name));
    for (const name of receipt.removedTestProxyVariables) delete testEnv[name];
    testEnv.NO_PROXY = testEnv.no_proxy = "localhost,127.0.0.1,::1";
    receipt.testLoopbackBypass = testEnv.NO_PROXY;
    receipt.removedTestCredentialVariables = Object.keys(testEnv).filter((name) => /(?:_KEY|_TOKEN|_SECRET|_PASSWORD|_ACCESS_KEY_ID)$/i.test(name));
    for (const name of receipt.removedTestCredentialVariables) delete testEnv[name];
    if (["darwin", "linux"].includes(process.platform)) {
      const fixture = join(native, "app-server/tests/suite/zsh");
      const cache = spawnSync("dotslash", ["--", "cache-dir"], { cwd: native, env, encoding: "utf8" });
      const fetched = spawnSync("dotslash", ["--", "fetch", fixture], { cwd: native, env, encoding: "utf8" });
      if (cache.error || cache.status !== 0 || fetched.error || fetched.status !== 0) {
        throw new Error(`无法准备锁定的 zsh 测试工具：${cache.error?.message ?? fetched.error?.message ?? fetched.stderr ?? cache.stderr}`);
      }
      const cacheDirectory = cache.stdout.trim();
      const executable = fetched.stdout.trim();
      if (!cacheDirectory || !executable || !existsSync(executable)) throw new Error("DotSlash 未返回有效的测试工具及缓存路径");
      testEnv.DOTSLASH_CACHE = cacheDirectory;
      const normal = spawnSync(executable, ["-fc", "/usr/bin/true"], { cwd: native, env: testEnv });
      const intercepted = spawnSync(executable, ["-fc", "/usr/bin/true"], { cwd: native, env: { ...testEnv, EXEC_WRAPPER: "/usr/bin/false" } });
      if (normal.error || normal.status !== 0 || intercepted.error || intercepted.status !== 1) throw new Error("锁定的 zsh 测试工具未通过启动与命令拦截核验");
      receipt.dotslash = { manifestSha256: createHash("sha256").update(readFileSync(fixture)).digest("hex"), executableSha256: createHash("sha256").update(readFileSync(executable)).digest("hex"), explicitlyPrepared: true, normalExitCode: normal.status, interceptedExitCode: intercepted.status };
    }
    const packages = workspace ? ["--workspace", "--features", "codex-v8-poc/sandbox"] : ["codex-core", "codex-app-server", "codex-app-server-protocol", "codex-protocol", "codex-rmcp-client", "codex-http-client", "codex-code-mode-host", "codex-code-mode-runtime"].flatMap((name) => ["-p", name]);
    const selection = filter ? ["-E", filter] : [];
    const junit = join(native, "target/nextest/default/junit.xml");
    const reportDigest = () => existsSync(junit) ? createHash("sha256").update(readFileSync(junit)).digest("hex") : null;
    const previousReport = reportDigest();
    try {
      await run("just", ["test", "--profile", "default", "--cargo-profile", "dev-small", "--retries", "0", "--test-threads", "4", ...packages, ...selection], native, testEnv);
      receipt.testsPassed = true;
    } finally {
      receipt.freshTestReport = reportDigest() !== null && reportDigest() !== previousReport;
      if (receipt.freshTestReport) copyFileSync(junit, join(receiptDirectory, "junit.xml"));
      else if (receipt.testsPassed) throw new Error("测试未生成本轮报告，不能记为通过");
    }
  } catch (error) {
    receipt.error = error.message;
    process.exitCode = 1;
  } finally {
    if (isolated) {
      try { rmSync(isolated, { recursive: true }); }
      catch (error) {
        receipt.cleanupError = error.message;
        process.exitCode = 1;
        process.stderr.write(`测试临时目录清理失败：${error.message}\n`);
      }
    }
    receipt.completedAt = new Date().toISOString();
    receipt.passed = receipt.testsPassed === true && receipt.freshTestReport === true && !receipt.error && !receipt.cleanupError;
    writeFileSync(join(receiptDirectory, "result.json"), `${JSON.stringify(receipt, null, 2)}\n`);
  }
}

if (!process.env.CILAI_BUILD_LOCK_TOKEN) process.exitCode = await runLockedBuild([process.execPath, ...process.argv.slice(1)]);
else await main();
