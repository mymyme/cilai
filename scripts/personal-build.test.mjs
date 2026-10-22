import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { assertPersonalBuildRoot, personalApplication, personalBuildChannel, personalBuildRoot, personalSourceRoot } from './personal-build.mjs';
import { packageBundlePaths } from '../apps/desktop/scripts/package-output.mjs';

test('个人构建根拒绝任意外部目录及企业构建根', () => {
  assert.doesNotThrow(() => assertPersonalBuildRoot());
  for (const root of ['/tmp/desktop', '/Users/tester/.cilai-builds/desktop']) assert.throws(() => assertPersonalBuildRoot(root), /个人构建根/);
});
test('全部打包身份与可执行文件名独立', () => {
  for (const name of ['electron-builder.yml', 'electron-builder.release.yml', 'electron-builder.portable.yml']) {
    const source = readFileSync(join(personalSourceRoot, 'apps/desktop', name), 'utf8');
    assert.match(source, /^appId: cn\.cilai\.personal\.workbench$/m);
    assert.match(source, /^productName: 词来个人工作台$/m);
    assert.match(source, /^executableName: cilai-personal-workbench$/m);
    assert.doesNotMatch(source, /https:\/\/updates\.cilai\.com\.cn/);
  }
  const app = packageBundlePaths({ output: personalBuildRoot(), platform: 'darwin', arch: 'arm64', ...personalApplication });
  assert.ok(app.bundle.endsWith('cilai-personal-workbench.app'));
  assert.ok(app.executable.endsWith('/MacOS/cilai-personal-workbench'));
});
test('发布、在线包和免安装发布入口真实调用均在执行外部动作前拒绝', () => {
  for (const args of [
    ['scripts/package-entry.mjs', '--release'], ['scripts/package-entry.mjs', '--portable'], ['scripts/package-entry.mjs', '--online-test'],
    ['apps/desktop/scripts/publish-release.mjs'], ['apps/desktop/scripts/publish-portable.mjs'],
    ['apps/desktop/scripts/package.mjs', 'electron-builder.release.yml'],
  ]) {
    const result = spawnSync(process.execPath, args, { cwd: personalSourceRoot, encoding: 'utf8', env: { ...process.env, CILAI_RELEASE_HOST: 'never-contact.invalid' } });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /个人版|个人开源版/);
  }
});

test('个人构建忽略企业通道，显式个人通道只接受 testing', () => {
  assert.equal(personalBuildChannel({}), 'testing');
  assert.equal(personalBuildChannel({ CILAI_UPDATE_CHANNEL: 'stable' }), 'testing');
  assert.equal(personalBuildChannel({ CILAI_UPDATE_CHANNEL: 'stable', CILAI_PERSONAL_UPDATE_CHANNEL: 'testing' }), 'testing');
  for (const selected of ['stable', 'enterprise', 'TESTING']) assert.throws(() => personalBuildChannel({ CILAI_PERSONAL_UPDATE_CHANNEL: selected }), /当前只支持 testing/);
  const inherited = { ...process.env, CILAI_UPDATE_CHANNEL: 'stable' }; delete inherited.CILAI_PERSONAL_UPDATE_CHANNEL;
  const result = spawnSync(process.execPath, ['scripts/personal-build.mjs', '--channel'], { cwd: personalSourceRoot, encoding: 'utf8', env: inherited });
  assert.equal(result.status, 0); assert.equal(result.stdout.trim(), 'testing');
  for (const script of ['script/package.sh', 'script/package.ps1']) {
    const source = readFileSync(join(personalSourceRoot, script), 'utf8');
    assert.match(source, /personal-build\.mjs.*--channel/);
    assert.doesNotMatch(source, /\$\{CILAI_UPDATE_CHANNEL|\$env:CILAI_UPDATE_CHANNEL/);
  }
});
test('入口在个人通道不支持时拒绝，不启动打包子进程', () => {
  const result = spawnSync(process.execPath, ['scripts/package-entry.mjs'], { cwd: personalSourceRoot, encoding: 'utf8', env: { ...process.env, CILAI_PERSONAL_UPDATE_CHANNEL: 'stable' } });
  assert.equal(result.status, 1); assert.match(result.stderr, /CILAI_PERSONAL_UPDATE_CHANNEL/);
});
