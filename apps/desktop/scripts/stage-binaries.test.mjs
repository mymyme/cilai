// 随包清单与 sidecar 产出脚本的一致性检查。
//
// Windows 端的清单在本机无法实跑验证——交叉编译走不通，实机验证要等那端的机器。
// 但清单写错这件事不必等到那时才发现：清单里的每个取件路径，都应当能在 sidecar 产出
// 脚本里找到对应的写入。本测试就核对这一条对应关系，两端各核一次。
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { assertStagingInventory, buildManifest } from "./stage-binaries.mjs";
import { frontendBuildInvocation, prepareInvocation } from "./prepare-package.mjs";
import { assertDebugPackageLocation, canonicalDebugBuildRoot } from "./debug-package-location.mjs";

const projectRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const prepareMac = await readFile(join(projectRoot, "script", "prepare_sidecar.sh"), "utf8");
const prepareWindows = await readFile(join(projectRoot, "script", "prepare_sidecar.ps1"), "utf8");

const preparePersonalNode = await readFile(join(projectRoot, "scripts", "prepare-personal-node.mjs"), "utf8");

const macTriple = "aarch64-apple-darwin";
const windowsTriple = "x86_64-pc-windows-msvc";

test("两端测试包只允许从个人项目构建根生成，release 同样受约束", () => {
  for (const [platform, project, expected] of [
    ['darwin', '/Users/tester/personal', '/Users/tester/personal/.personal-build'],
    ['win32', 'D:\\personal', 'D:\\personal\\.personal-build'],
  ]) {
    const buildRoot = canonicalDebugBuildRoot({ platform, project });
    assert.equal(buildRoot, expected);
    for (const profile of ['debug', 'release']) {
      assert.doesNotThrow(() => assertDebugPackageLocation({ platform, project, profile, projectRoot: buildRoot, buildRoot }));
      assert.throws(() => assertDebugPackageLocation({ platform, project, profile, projectRoot: project, buildRoot }), /个人构建目录/);
    }
  }
});

const mac = buildManifest({ platform: "darwin", profile: "release", triple: macTriple, buildRoot: "/build" });
const windows = buildManifest({ platform: "win32", profile: "release", triple: windowsTriple, buildRoot: "C:\\build" });

test("两端清单都包含业务服务、应用服务与对话运行时", () => {
  for (const [label, items, suffix] of [["macOS", mac, ""], ["Windows", windows, ".exe"]]) {
    const landed = items.map((item) => item.to);
    assert.ok(landed.includes("codex-runtime"));
    for (const name of ["cilai-host-service", "cilai-app-server", "cilai-node"]) {
      assert.ok(landed.includes(name + suffix), `${label} 清单缺少 ${name}`);
    }
  }
});

test("PDF 渲染器只出现在 macOS 清单里", () => {
  assert.ok(mac.some((item) => item.to === "cilai-pdf-renderer"));
  assert.ok(!windows.some((item) => item.to.includes("pdf-renderer")));
});

test("完整官方 Codex 发行目录同时交付所有原生组件", () => {
  for (const items of [mac,windows]) assert.ok(items.some(item=>item.to === "codex-runtime"));
});

test("打包前拦截旧版私有 Harness 和官方运行目录内的历史残留", () => {
  const root = mkdtempSync(join(tmpdir(), "cilai-stage-inventory-"));
  try {
    const source = join(root, "source");
    const target = join(root, "stage");
    for (const directory of [source, join(target, "codex-runtime")]) {
      mkdirSync(join(directory, "bin"), { recursive: true });
      writeFileSync(join(directory, "bin", "codex-app-server.exe"), "official");
    }
    const items = [{ from: source, to: "codex-runtime" }];
    assert.doesNotThrow(() => assertStagingInventory(items, target));
    const legacy = join(target, "contexthub-codex-harness.exe");
    writeFileSync(legacy, "patched runtime");
    assert.throws(() => assertStagingInventory(items, target), /contexthub-codex-harness\.exe/);
    rmSync(legacy);
    writeFileSync(join(target, "codex-runtime", "bin", "stale-helper.exe"), "old");
    assert.throws(() => assertStagingInventory(items, target), /codex-runtime\/bin\/stale-helper\.exe/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("Windows Git 完整运行时从当前配置的 sidecar 取件", () => {
  const runtime = windows.find((item) => item.to === "git");
  assert.equal(runtime.from, join("C:\\build", "target", "sidecar", "release", "git"));
  assert.equal(mac.some((item) => item.to === "git"), false);
  assert.match(prepareWindows, /prepare-git-runtime\.mjs/);
});

test("落地文件名不带目标三元组后缀：业务侧按固定名在同目录查找", () => {
  for (const item of [...mac, ...windows]) {
    assert.ok(
      !item.to.includes(macTriple) && !item.to.includes(windowsTriple),
      `${item.to} 仍带三元组后缀`,
    );
  }
});

test("每个取件路径都能在对应平台的 sidecar 产出脚本里找到写入", () => {
  // 产出脚本里三元组是变量而非字面量，因此按各自的变量名还原后再比对
  const cases = [
    ["macOS", mac, prepareMac, macTriple, "$TARGET_TRIPLE"],
    ["Windows", windows, prepareWindows, windowsTriple, "$TargetTriple"],
  ];
  for (const [label, items, script, triple, variable] of cases) {
    for (const item of items) {
      const fileName = item.from.split(/[\\/]/).pop();
      // 宿主服务由 cargo 直接产出，不经 sidecar 脚本
      if (fileName.startsWith("cilai-host-service")) continue;
      const written = fileName.replace(triple, variable);
      assert.ok(
        (script.includes(written) || (fileName === "node-LICENSE.txt" && script.includes("prepare-personal-node.mjs") && preparePersonalNode.includes('cpSync(license, join(directory, "node-LICENSE.txt"))'))),
        `${label}：${written} 未出现在 prepare_sidecar 中，取件路径可能已失效`,
      );
    }
    assert.ok(
      items.some((item) => item.from.includes("sidecar")),
      `${label}：清单未从 sidecar 目录取件`,
    );
  }
});

test("取件位置按构建配置分目录，正式暂存不会取到调试产物", () => {
  // 两种配置的产物文件名完全一致，混用不会有任何外在迹象；分目录是唯一能把它们分开的地方。
  for (const platform of ["darwin", "win32"]) {
    const triple = platform === "darwin" ? macTriple : windowsTriple;
    const debug = buildManifest({ platform, profile: "debug", triple, buildRoot: "/build" });
    const release = buildManifest({ platform, profile: "release", triple, buildRoot: "/build" });
    for (const [a, b] of debug.map((item, index) => [item, release[index]])) {
      assert.equal(a.to, b.to, "落地文件名不随配置变化");
      assert.notEqual(a.from, b.from, `${a.to} 的取件位置未按配置区分`);
    }
    assert.ok(debug.some((item) => item.from.includes("sidecar") && item.from.includes("debug")));
    assert.ok(release.some((item) => item.from.includes("sidecar") && item.from.includes("release")));
  }
  // 产出脚本两端都要按配置分目录，否则暂存这一侧分了也没用
  assert.match(prepareMac, /target\/sidecar\/\$PROFILE/);
  assert.match(prepareWindows, /target\\sidecar\\\$BuildProfile/);
});

test("Windows 的宿主服务多一层目标目录：该端构建传了 --target", () => {
  const host = windows.find((item) => item.to === "cilai-host-service.exe");
  assert.ok(host.from.includes(windowsTriple), "Windows 取件路径缺少目标目录层级");
  const macHost = mac.find((item) => item.to === "cilai-host-service");
  assert.ok(!macHost.from.includes(macTriple), "macOS 取件路径不应有目标目录层级");
});

test("两端打包前都先构建当前前端与 sidecar，再进入暂存", async () => {
  const packageJson = JSON.parse(await readFile(join(projectRoot, "apps", "desktop", "package.json"), "utf8"));
  assert.equal(
    packageJson.scripts.prepackage,
    "npm --prefix ../.. run check:toolchain && node scripts/prepare-package.mjs --profile debug",
  );
  assert.equal(
    packageJson.scripts["prepackage:release"],
    "npm --prefix ../.. run check:toolchain && node scripts/prepare-package.mjs --profile release",
  );

  assert.deepEqual(
    frontendBuildInvocation({ platform: "darwin", projectRoot: "/project" }),
    { command: "npm", args: ["run", "build"], cwd: "/project" },
  );
  assert.deepEqual(
    frontendBuildInvocation({ platform: "win32", projectRoot: "C:\\project" }),
    { command: "cmd.exe", args: ["/d", "/s", "/c", "npm.cmd run build"], cwd: "C:\\project" },
  );

  const macPrepare = prepareInvocation({
    platform: "darwin", profile: "debug", projectRoot: "/project",
  });
  assert.equal(macPrepare.command, "bash");
  assert.deepEqual(macPrepare.args, ["/project/script/prepare_sidecar.sh", "debug"]);

  const windowsPrepare = prepareInvocation({
    platform: "win32", profile: "release", projectRoot: "C:\\project",
  });
  assert.equal(windowsPrepare.command, "powershell.exe");
  assert.ok(windowsPrepare.args.includes("C:\\project\\script\\prepare_sidecar.ps1"));
  assert.deepEqual(windowsPrepare.args.slice(-2), ["-BuildProfile", "release"]);
});

test("两端打包前都先构建当前源码的前端", () => {
  const macBuild = frontendBuildInvocation({ platform: "darwin", projectRoot: "/project" });
  assert.equal(macBuild.command, "npm");
  assert.deepEqual(macBuild.args, ["run", "build"]);
  assert.equal(macBuild.cwd, "/project");

  const windowsBuild = frontendBuildInvocation({
    platform: "win32", projectRoot: "C:\\project",
  });
  assert.equal(windowsBuild.command, "cmd.exe");
  assert.deepEqual(windowsBuild.args, ["/d", "/s", "/c", "npm.cmd run build"]);
  assert.equal(windowsBuild.cwd, "C:\\project");
});

test("打包完成后核对两端随包资源与主进程依赖", async () => {
  const packageScript = await readFile(
    join(projectRoot, "apps", "desktop", "scripts", "package.mjs"),
    "utf8",
  );
  assert.match(packageScript, /verifyPackageLayout/);
  assert.match(packageScript, /platform: process\.platform/);
  assert.match(packageScript, /ELECTRON_RUN_AS_NODE/);
  assert.match(packageScript, /app\.asar\/node_modules\/electron-updater/);
  assert.match(packageScript, /产物主进程运行依赖不完整/);
});
