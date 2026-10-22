import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { codexRelease, codexRuntimeTarget, prepareCodexNotices, verifyCodexArtifact, verifyCodexNotices, verifyCodexRuntime, verifyCodexSource } from "./prepare-codex-runtime.mjs";
import { personalBuildRoot } from "./personal-build.mjs";

function noticeFixture(t) {
  const parent = join(personalBuildRoot(), 'target/codex-notice-test-fixtures');
  mkdirSync(parent, { recursive: true });
  const root = mkdtempSync(join(parent, 'notices-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
}

test("官方二进制发行件独立固定版本、归档和可执行文件哈希", () => {
  assert.equal(codexRelease.version, "0.163.0");
  for (const artifact of Object.values(codexRelease.packages)) {
    assert.match(artifact.sha256, /^[a-f0-9]{64}$/);
    assert.match(artifact.executableSha256, /^[a-f0-9]{64}$/);
    assert.match(artifact.releaseUrl, /^https:\/\/github\.com\/openai\/codex\/releases\//);
  }
});

test('运行包严格绑定平台、版本和入口布局', () => {
  assert.equal(codexRuntimeTarget({ platform: 'darwin', arch: 'arm64' }), 'aarch64-apple-darwin');
  assert.equal(codexRuntimeTarget({ platform: 'darwin', arch: 'x64' }), 'x86_64-apple-darwin');
  assert.equal(codexRuntimeTarget({ platform: 'win32', arch: 'x64' }), 'x86_64-pc-windows-msvc');
  assert.throws(() => codexRuntimeTarget({ platform: 'darwin', arch: 'ia32' }));
  const root = mkdtempSync(join(tmpdir(), 'codex-runtime-layout-'));
  const description = { layoutVersion: 1, version: codexRelease.version, target: 'aarch64-apple-darwin',
    variant: 'codex-app-server', entrypoint: 'bin/codex-app-server', resourcesDir: 'codex-resources', pathDir: 'codex-path' };
  try {
    for (const change of [{ version: '0.1.0' }, { target: 'x86_64-apple-darwin' }, { layoutVersion: 2 },
      { entrypoint: '../codex-app-server' }, { resourcesDir: '../resources' }, { pathDir: 'other' }]) {
      writeFileSync(join(root, 'codex-package.json'), JSON.stringify({ ...description, ...change }));
      assert.throws(() => verifyCodexRuntime(root, description.target), /目录清单不匹配/);
    }
    writeFileSync(join(root, 'codex-package.json'), JSON.stringify(description));
    for (const directory of ['bin', 'codex-resources', 'codex-path']) mkdirSync(join(root, directory));
    writeFileSync(join(root, description.entrypoint), 'modified runtime');
    assert.throws(() => verifyCodexRuntime(root, description.target), /SHA-256/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
test("锁文件改写、额外补丁文件和发行件篡改均阻止集成", () => {
  const root=mkdtempSync(join(tmpdir(),"codex-integrity-"));
  const bytes=Buffer.from("original lock\n");
  const sha=createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
  try {
    writeFileSync(join(root,"Cargo.lock"),bytes);
    const manifest={files:[{path:"Cargo.lock",mode:"100644",sha}]};
    assert.equal(verifyCodexSource(root,manifest),1);
    writeFileSync(join(root,"Cargo.lock"),"normalized lock\n");
    assert.throws(()=>verifyCodexSource(root,manifest),/Cargo.lock/);
    writeFileSync(join(root,"Cargo.lock"),bytes);
    writeFileSync(join(root,"patch.rs"),"private patch");
    assert.throws(()=>verifyCodexSource(root,manifest),/额外文件/);
    assert.throws(()=>verifyCodexArtifact(bytes,"0".repeat(64)),/SHA-256/);
  } finally {rmSync(root,{recursive:true,force:true});}
});

test('现行许可通知离线绑定同版本官方标签、commit及两份源文件', t => {
  const root = noticeFixture(t);
  prepareCodexNotices(root);
  const manifest = verifyCodexNotices(root);
  assert.equal(manifest.version, '0.163.0');
  assert.equal(manifest.tagObject, codexRelease.tagObject);
  assert.equal(manifest.sourceCommit, '83086188ae0614bd882818ff1aa23f3194962012');
  assert.deepEqual(manifest.files.map(file => file.path), ['LICENSE', 'NOTICE']);
  for (const file of manifest.files) {
    const bytes = readFileSync(join(root, file.path));
    assert.equal(createHash('sha256').update(bytes).digest('hex'), file.sha256);
    assert.equal(createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex'), file.upstreamBlobSha1);
  }
  const original = readFileSync(join(root, 'NOTICE'));
  prepareCodexNotices(root);
  assert.deepEqual(readFileSync(join(root, 'NOTICE')), original);
});

test('运行件缺通知、通知损坏或来源清单改变时拒绝交付', t => {
  const root = noticeFixture(t);
  assert.throws(() => verifyCodexNotices(root), /ENOENT/);
  prepareCodexNotices(root);
  writeFileSync(join(root, 'NOTICE'), 'unrelated legacy notice');
  assert.throws(() => verifyCodexNotices(root), /SHA-256/);
  assert.throws(() => prepareCodexNotices(root), /固定来源不一致/);
  assert.equal(readFileSync(join(root, 'NOTICE'), 'utf8'), 'unrelated legacy notice');
  rmSync(join(root, 'NOTICE'));
  prepareCodexNotices(root);
  writeFileSync(join(root, 'cilai-codex-notices.json'), '{}');
  assert.throws(() => verifyCodexNotices(root), /SHA-256/);
  rmSync(join(root, 'LICENSE'));
  assert.throws(() => verifyCodexNotices(root), /SHA-256|ENOENT/);
});

test('许可通知不沿目标文件链接读写其他文件', t => {
  const root = noticeFixture(t);
  const outside = join(root, 'protected-source.txt');
  writeFileSync(outside, 'protected');
  try { symlinkSync(outside, join(root, 'LICENSE')); }
  catch (error) {
    if (process.platform === 'win32' && error.code === 'EPERM') { t.skip('当前 Windows 测试进程无创建符号链接权限'); return; }
    throw error;
  }
  assert.throws(() => prepareCodexNotices(root), /普通文件/);
  assert.equal(readFileSync(outside, 'utf8'), 'protected');
});
