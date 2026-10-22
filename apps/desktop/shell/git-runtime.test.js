const test = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const vm = require("node:vm");

function loadRuntime({ version = "2.56.0.windows.2", execute = () => `git version ${version}`, read = () => JSON.stringify({ version }) } = {}) {
  const calls = [];
  const context = { module: { exports: {} }, process, require(name) {
    if (name === "node:fs") return { readFileSync: read };
    if (name === "node:child_process") return { execFileSync(...args) { calls.push(args); return execute(...args); } };
    return require(name);
  } };
  vm.runInNewContext(readFileSync(join(__dirname, "git-runtime.js"), "utf8"), context);
  return { configure: context.module.exports.gitRuntimeEnvironment, calls };
}

test("Windows 无系统 Git 时优先使用随包 Git，并仅修改子进程环境", () => {
  const { configure, calls } = loadRuntime();
  const original = { PATH: "C:\\Windows\\System32", Path: "C:\\other", ACCOUNT: "test" };
  const env = configure("C:\\app\\resources\\sidecar\\cilai-host-service.exe", original, "win32");
  assert.equal(env.Path, "C:\\app\\resources\\sidecar\\git\\cmd;C:\\Windows\\System32");
  assert.equal(env.PATH, undefined);
  assert.equal(env.CILAI_MEMORY_GIT_RUNTIME, "mingit-2.56.0.windows.2");
  assert.deepEqual(original, { PATH: "C:\\Windows\\System32", Path: "C:\\other", ACCOUNT: "test" });
  assert.equal(calls[0][0], "C:\\app\\resources\\sidecar\\git\\cmd\\git.exe");
  assert.equal(calls[0][2].windowsHide, true);
});

test("开发宿主使用当前构建配置的 Git，macOS 保持系统环境", () => {
  const { configure, calls } = loadRuntime();
  configure("D:\\build\\target\\x86_64-pc-windows-msvc\\debug\\cilai-host-service.exe", {}, "win32");
  assert.equal(calls[0][0], "D:\\build\\target\\sidecar\\debug\\git\\cmd\\git.exe");
  const env = configure("/build/cilai-host-service", { PATH: "/usr/bin" }, "darwin");
  assert.equal(env.PATH, "/usr/bin");
  assert.equal(calls.length, 1);
});

test("Git 缺件、DLL 故障或版本错误时明确失败，不采用系统 Git", () => {
  for (const options of [
    { read: () => { throw new Error("ENOENT manifest"); } },
    { execute: () => { throw new Error("DLL missing"); } },
    { execute: () => "git version wrong" },
  ]) {
    const { configure } = loadRuntime(options);
    assert.throws(() => configure("C:\\app\\cilai-host-service.exe", { PATH: "C:\\system-git" }, "win32"), /工作台 Git 运行时不可用/);
  }
});
