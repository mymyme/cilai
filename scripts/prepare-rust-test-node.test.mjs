import assert from "node:assert/strict";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { acquireBuildLock, releaseBuildLock } from "./build-lock.mjs";
import { prepareRustTestNode } from "./prepare-rust-test-node.mjs";

function fixture(t, platform = "darwin", arch = "arm64") {
  const parent = mkdtempSync(join(realpathSync(tmpdir()), "cilai-rust-test-node-"));
  const root = join(parent, "personal");
  const bin = join(parent, "node", "bin");
  mkdirSync(root); mkdirSync(bin, { recursive: true });
  const executable = join(bin, platform === "win32" ? "node.exe" : "node");
  writeFileSync(executable, "fixture Node binary", { mode: 0o755 });
  const license = platform === "win32" ? join(bin, "LICENSE") : join(bin, "..", "LICENSE");
  writeFileSync(license, "Node.js full fixture license\n");
  const token = acquireBuildLock(root, "Node fixture");
  t.after(() => { releaseBuildLock(root, token); rmSync(parent, { recursive: true, force: true }); });
  return { parent, root, license, executable, options: { buildRoot: root, expectedRoot: root,
    environment: { CILAI_BUILD_LOCK_TOKEN: token, CARGO_TARGET_DIR: join(root, "target") },
    runtime: { nodeVersion: "24.18.0", platform, arch, executable } } };
}

test("Rust 测试同目录准备固定组件与许可证，相同内容复用不重写", t => {
  const f = fixture(t);
  const first = prepareRustTestNode(f.options);
  assert.equal(first.action, "prepared");
  assert.equal(first.executable, join(f.root, "target/debug/deps/cilai-node"));
  assert.equal(readFileSync(first.executable, "utf8"), readFileSync(f.executable, "utf8"));
  assert.equal(readFileSync(first.license, "utf8"), readFileSync(f.license, "utf8"));
  const before = [statSync(first.executable), statSync(first.license)];
  assert.equal(prepareRustTestNode(f.options).action, "reused");
  for (const [index, file] of [first.executable, first.license].entries()) {
    assert.equal(statSync(file).mtimeMs, before[index].mtimeMs);
    assert.equal(statSync(file).ino, before[index].ino);
  }
});

test("Windows x64 契约使用 exe 固定名并收集同目录许可证", t => {
  const f = fixture(t, "win32", "x64");
  const prepared = prepareRustTestNode(f.options);
  assert.equal(prepared.executable, join(f.root, "target/debug/deps/cilai-node.exe"));
  assert.equal(readFileSync(prepared.license, "utf8"), readFileSync(f.license, "utf8"));
});

test("版本、平台架构及许可证错误在创建目标前明确拒绝", t => {
  const f = fixture(t);
  for (const change of [{ nodeVersion: "24.17.0" }, { platform: "linux" }, { platform: "win32", arch: "arm64" }]) {
    assert.throws(() => prepareRustTestNode({ ...f.options, runtime: { ...f.options.runtime, ...change } }), /Node 版本不匹配|构建环境不匹配/);
  }
  writeFileSync(f.license, "unrelated license");
  assert.throws(() => prepareRustTestNode(f.options), /缺少 Node.js/);
  assert.equal(existsSync(join(f.root, "target")), false);
});

test("根目录、Cargo 目标和构建锁不符时不创建测试组件", t => {
  const f = fixture(t);
  assert.throws(() => prepareRustTestNode({ ...f.options, buildRoot: f.parent }), /标准个人构建根/);
  assert.throws(() => prepareRustTestNode({ ...f.options, environment: { CILAI_BUILD_LOCK_TOKEN: "" } }), /构建锁无效/);
  assert.throws(() => prepareRustTestNode({ ...f.options, environment: { ...f.options.environment, CILAI_BUILD_LOCK_TOKEN: "other" } }), /另一任务/);
  for (const environment of [{ CARGO_TARGET_DIR: f.parent }, { CILAI_BUILD_ROOT: f.parent }, { CARGO_BUILD_TARGET: "aarch64-apple-darwin" }]) {
    assert.throws(() => prepareRustTestNode({ ...f.options, environment: { ...f.options.environment, ...environment } }), /构建目标必须/);
  }
  assert.equal(existsSync(join(f.root, "target")), false);
});

test("目标目录链接不能把 Node 准备重定向到构建根外", t => {
  const f = fixture(t);
  const outside = join(f.parent, "outside"); mkdirSync(outside);
  symlinkSync(outside, join(f.root, "target"), process.platform === "win32" ? "junction" : "dir");
  assert.throws(() => prepareRustTestNode(f.options), /符号链接/);
  assert.equal(existsSync(join(outside, "debug")), false);
});

test("不同内容或链接组件保持原样，不覆盖未知组件", t => {
  const f = fixture(t);
  const directory = join(f.root, "target/debug/deps"); mkdirSync(directory, { recursive: true });
  const node = join(directory, "cilai-node"); writeFileSync(node, "unowned binary");
  assert.throws(() => prepareRustTestNode(f.options), /不同内容/);
  assert.equal(readFileSync(node, "utf8"), "unowned binary");
  assert.equal(existsSync(join(directory, "node-LICENSE.txt")), false);
  rmSync(node);
  symlinkSync(process.platform === "win32" ? dirname(f.executable) : f.executable, node, process.platform === "win32" ? "junction" : "file");
  assert.throws(() => prepareRustTestNode(f.options), /符号链接/);
  assert.equal(readFileSync(f.executable, "utf8"), "fixture Node binary");
});

test("同字节但不可执行的既有组件不能冒充已准备运行时", { skip: process.platform === "win32" }, t => {
  const f = fixture(t);
  const prepared = prepareRustTestNode(f.options);
  chmodSync(prepared.executable, 0o600);
  assert.throws(() => prepareRustTestNode(f.options), /EACCES/);
  assert.equal(statSync(prepared.executable).mode & 0o777, 0o600);
});

test("两端质量入口均在 Rust 测试之前准备同目录 Node", () => {
  const source = join(dirname(fileURLToPath(import.meta.url)), "..");
  for (const script of ["quality.sh", "quality.ps1"]) {
    const content = readFileSync(join(source, "script", script), "utf8");
    const preparation = content.indexOf("node scripts/prepare-rust-test-node.mjs");
    assert.ok(preparation > content.indexOf("cargo clippy --workspace"));
    assert.ok(preparation < content.indexOf("cargo test --workspace"));
  }
});
