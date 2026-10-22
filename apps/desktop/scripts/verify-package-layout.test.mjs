import assert from "node:assert/strict";
import { cpSync, existsSync, lstatSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { verifyPackageLayout } from "./verify-package-layout.mjs";
import { restoreEmptyDirectories } from './package-empty-directories.mjs';

function write(file, value) {
  mkdirSync(join(file, ".."), { recursive: true });
  writeFileSync(file, value);
}

function codexDirectories() {
  const root = mkdtempSync(join(tmpdir(), 'cilai-codex-empty-directories-'));
  const source = join(root, 'source');
  const packaged = join(root, 'packaged');
  write(join(source, 'bin', 'codex-app-server'), 'official-runtime');
  write(join(source, 'codex-path', 'rg'), 'official-rg');
  mkdirSync(join(source, 'codex-resources'));
  cpSync(source, packaged, { recursive: true });
  rmSync(join(packaged, 'codex-resources'), { recursive: true });
  return { root, source, packaged };
}

test('官方包真实空资源目录在签名前复制，不增加占位文件或改变程序字节', () => {
  const { root, source, packaged } = codexDirectories();
  try {
    assert.equal(restoreEmptyDirectories(source, packaged), 1);
    const resources = join(packaged, 'codex-resources');
    assert.equal(lstatSync(resources).isDirectory(), true);
    assert.deepEqual(readdirSync(resources), []);
    for (const file of ['bin/codex-app-server', 'codex-path/rg']) {
      assert.deepEqual(readFileSync(join(packaged, file)), readFileSync(join(source, file)));
    }
    assert.equal(restoreEmptyDirectories(source, packaged), 0);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('空目录复制拒绝目标文件和符号链接，保留原目标', () => {
  for (const kind of ['file', 'link']) {
    const { root, source, packaged } = codexDirectories();
    const resources = join(packaged, 'codex-resources');
    try {
      if (kind === 'file') writeFileSync(resources, 'unexpected');
      else {
        const outside = join(root, 'outside');
        mkdirSync(outside);
        symlinkSync(outside, resources, process.platform === 'win32' ? 'junction' : 'dir');
      }
      assert.throws(() => restoreEmptyDirectories(source, packaged), /不是普通目录/);
      assert.equal(kind === 'file' ? lstatSync(resources).isFile() : lstatSync(resources).isSymbolicLink(), true);
    } finally { rmSync(root, { recursive: true, force: true }); }
  }
});

test('非空来源目录缺失或空目录目标有额外内容时保持失败，不重建缺失文件', () => {
  const { root, source, packaged } = codexDirectories();
  try {
    rmSync(join(packaged, 'bin'), { recursive: true });
    assert.throws(() => restoreEmptyDirectories(source, packaged), /缺少非空随包目录/);
    assert.equal(existsSync(join(packaged, 'bin')), false);
    assert.equal(existsSync(join(packaged, 'codex-resources')), false);
    cpSync(join(source, 'bin'), join(packaged, 'bin'), { recursive: true });
    write(join(packaged, 'codex-resources', 'unexpected'), 'unexpected');
    assert.throws(() => restoreEmptyDirectories(source, packaged), /清单外内容/);
    assert.equal(readFileSync(join(packaged, 'codex-resources', 'unexpected'), 'utf8'), 'unexpected');
  } finally { rmSync(root, { recursive: true, force: true }); }
});

for (const [platform, arch] of [["darwin", "arm64"], ["win32", "x64"]]) {
  test(`${platform} 产物须包含完整前端与嵌套随包组件`, () => {
    const root = mkdtempSync(join(tmpdir(), "cilai-package-layout-"));
    try {
      const output = join(root, "output");
      const stagedDir = join(root, "stage");
      const frontendDir = join(root, "frontend");
      const bundle = platform === "win32"
        ? join(output, "win-unpacked")
        : join(output, "mac-arm64", "词来工作台.app");
      const resources = platform === "win32"
        ? join(bundle, "resources")
        : join(bundle, "Contents", "Resources");
      const executable = platform === "win32"
        ? join(bundle, "词来工作台.exe")
        : join(bundle, "Contents", "MacOS", "词来工作台");
      write(executable, "app");
      write(join(stagedDir, "cilai-host-service"), "host");
      const runner = join("codex-runtime", "codex-resources", "codex-command-runner.exe");
      write(join(stagedDir, runner), "runner");
      write(join(frontendDir, "index.html"), "<main></main>");
      write(join(frontendDir, "assets", "index.js"), "content");
      write(join(resources, "sidecar", "cilai-host-service"), "host");
      write(join(resources, "sidecar", runner), "runner");
      write(join(resources, "frontend", "index.html"), "<main></main>");
      write(join(resources, "frontend", "assets", "index.js"), "content");
      const input = { output, platform, arch, productName: "词来工作台", stagedDir, frontendDir };
      assert.deepEqual(verifyPackageLayout(input), { bundle, resources, executable, sidecarCount: 2, frontendCount: 2 });
      rmSync(join(resources, "sidecar", runner));
      assert.throws(() => verifyPackageLayout(input), /缺少随包组件/);
      write(join(resources, "sidecar", runner), "runner");
      write(join(resources, "frontend", "assets", "index.js"), "stale");
      assert.throws(() => verifyPackageLayout(input), /前端与当前构建结果不一致/);
      write(join(resources, "frontend", "assets", "index.js"), "content");
      write(join(resources, "sidecar", "cilai-host-service"), "host-signed");
      assert.throws(() => verifyPackageLayout(input), /随包组件与当前构建结果不一致/);
      assert.throws(() => verifyPackageLayout({ ...input, signedSidecars: true }), /随包组件与当前构建结果不一致/);
      write(join(resources, "sidecar", "cilai-host-service"), "host");
      assert.equal(verifyPackageLayout({ ...input, signedSidecars: true }).sidecarCount, 2);
      write(join(resources, "sidecar", runner), "runner-resigned");
      assert.throws(() => verifyPackageLayout({ ...input, signedSidecars: true }), /随包组件与当前构建结果不一致/);
      write(join(resources, "sidecar", runner), "runner");
      write(join(resources, "sidecar", "unexpected.exe"), "unexpected");
      assert.throws(() => verifyPackageLayout({ ...input, signedSidecars: true }), /额外随包组件/);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
}
