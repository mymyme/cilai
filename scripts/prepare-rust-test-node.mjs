import { createHash } from "node:crypto";
import { accessSync, constants, copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, realpathSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { assertBuildLock } from "./build-lock.mjs";
import { assertNoSymlinkPath, assertPersonalBuildRoot, personalBuildRoot } from "./personal-build.mjs";
import { validatedPersonalNode } from "./prepare-personal-node.mjs";

const digest = file => createHash("sha256").update(readFileSync(file)).digest("hex");

export function prepareRustTestNode({
  buildRoot = personalBuildRoot(), expectedRoot = personalBuildRoot(), environment = process.env,
  runtime = { nodeVersion: process.versions.node, platform: process.platform, arch: process.arch, executable: process.execPath },
} = {}) {
  const root = resolve(buildRoot);
  if (root !== resolve(expectedRoot)) throw new Error("Rust 测试 Node 只能写入标准个人构建根");
  if (root === resolve(personalBuildRoot())) assertPersonalBuildRoot(root);
  assertNoSymlinkPath(root);
  if (realpathSync(root) !== root) throw new Error("个人构建根的规范路径不一致");
  const target = join(root, "target");
  if (environment.CARGO_BUILD_TARGET
      || (environment.CILAI_BUILD_ROOT && resolve(environment.CILAI_BUILD_ROOT) !== root)
      || (environment.CARGO_TARGET_DIR && resolve(environment.CARGO_TARGET_DIR) !== target)) {
    throw new Error("Rust 测试 Node 的构建目标必须为个人构建根内 target/debug/deps，且不指定目标三元组");
  }
  const token = environment.CILAI_BUILD_LOCK_TOKEN;
  assertBuildLock(root, token);
  const { executable, license, suffix } = validatedPersonalNode(runtime);
  const directory = join(target, "debug", "deps");
  assertNoSymlinkPath(directory);
  const files = [[executable, join(directory, `cilai-node${suffix}`)], [license, join(directory, "node-LICENSE.txt")]]
    .map(([source, destination]) => {
      assertNoSymlinkPath(destination);
      const sha256 = digest(source);
      const present = existsSync(destination);
      if (present && (!lstatSync(destination).isFile() || digest(destination) !== sha256)) {
        throw new Error(`Rust 测试组件已存在不同内容，未覆盖：${destination}`);
      }
      if (present && source === executable) accessSync(destination, constants.X_OK);
      return { source, destination, sha256, present };
    });
  assertBuildLock(root, token);
  mkdirSync(directory, { recursive: true });
  for (const file of files) {
    assertBuildLock(root, token);
    assertNoSymlinkPath(file.destination);
    if (!file.present) copyFileSync(file.source, file.destination, constants.COPYFILE_EXCL);
    if (digest(file.destination) !== file.sha256) throw new Error("Rust 测试 Node 准备期间内容发生变化");
    if (file.source === executable) accessSync(file.destination, constants.X_OK);
  }
  return { action: files.every(file => file.present) ? "reused" : "prepared", executable: files[0].destination, license: files[1].destination };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 2) throw new Error("Rust 测试 Node 准备不接受自定义目标");
  console.log(JSON.stringify(prepareRustTestNode()));
}
