import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, realpathSync, rmSync, symlinkSync, statSync, utimesSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { assertPrimarySource, sourceFiles, stageBuild } from './stage-build.mjs';
import { personalSourceRoot } from './personal-build.mjs';

function fixture(context) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'cilai-personal-stage-')));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const source = join(root, 'source'), destination = join(root, 'build');
  mkdirSync(source); mkdirSync(destination);
  return { root, source, destination };
}
test('无 Git 的暂存按内容更新，保留缓存，只清理来源已删除文件', context => {
  const { source, destination } = fixture(context);
  writeFileSync(join(source, 'one.txt'), 'AAAA'); writeFileSync(join(source, 'old.txt'), 'old');
  writeFileSync(join(destination, '.env.server'), '本机构建配置');
  mkdirSync(join(destination, 'target')); writeFileSync(join(destination, 'target/cached'), '缓存');
  assert.equal(stageBuild({ source, destination }).copied, 2);
  const oldTime = statSync(join(source, 'one.txt')).mtime;
  writeFileSync(join(source, 'one.txt'), 'BBBB'); utimesSync(join(source, 'one.txt'), oldTime, oldTime);
  rmSync(join(source, 'old.txt'));
  const next = stageBuild({ source, destination });
  assert.equal(next.copied, 1); assert.equal(next.removed, 1);
  assert.equal(readFileSync(join(destination, 'one.txt'), 'utf8'), 'BBBB');
  assert.equal(readFileSync(join(destination, '.env.server'), 'utf8'), '本机构建配置');
  assert.equal(readFileSync(join(destination, 'target/cached'), 'utf8'), '缓存');
});
test('暂存入口只接受登记的个人源码，不检查或变更 Git 分支', context => {
  const { source } = fixture(context);
  assert.doesNotThrow(() => assertPrimarySource(personalSourceRoot));
  assert.throws(() => assertPrimarySource(source), /个人版源码目录/);
});
test('清单排除私有环境和缓存，保留上游源码点目录', context => {
  const { root, source, destination } = fixture(context);
  for (const name of ['.git', '.venv', 'node_modules', 'target', '.codex', 'third_party/runtime/.venv', 'third_party/openai-codex/.vscode']) mkdirSync(join(source, name), { recursive: true });
  symlinkSync(root, join(source, '.venv/python'));
  writeFileSync(join(source, '.env.secret'), 'private');
  writeFileSync(join(source, 'third_party/openai-codex/.vscode/extensions.json'), 'official');
  assert.deepEqual(sourceFiles(source), []);
  assert.deepEqual(sourceFiles(source, { profile: 'native-extensions' }), ['third_party/openai-codex/.vscode/extensions.json']);
  stageBuild({ source, destination, profile: 'native-extensions' });
  assert.equal(readFileSync(join(destination, 'third_party/openai-codex/.vscode/extensions.json'), 'utf8'), 'official');
});
test('源码链接、构建目录链接和目标文件父目录链接都被拒绝，外部数据保持原状', context => {
  const { root, source, destination } = fixture(context);
  const outside = join(root, 'outside'); mkdirSync(outside); writeFileSync(join(outside, 'keep'), '原内容');
  symlinkSync(outside, join(source, 'linked'));
  assert.throws(() => sourceFiles(source), /符号链接/);
  rmSync(join(source, 'linked'));
  mkdirSync(join(source, 'directory')); writeFileSync(join(source, 'directory/keep'), '新内容');
  symlinkSync(outside, join(destination, 'directory'));
  assert.throws(() => stageBuild({ source, destination }), /符号链接/);
  assert.equal(readFileSync(join(outside, 'keep'), 'utf8'), '原内容');
  const linked = join(root, 'linked-build'); symlinkSync(destination, linked);
  assert.throws(() => stageBuild({ source, destination: linked }), /符号链接/);
});
test('构建目录单独修改过的已删除源码不会被覆盖或清除', context => {
  const { source, destination } = fixture(context);
  writeFileSync(join(source, 'keep'), '原内容'); stageBuild({ source, destination });
  writeFileSync(join(destination, 'keep'), '独立修改'); rmSync(join(source, 'keep'));
  assert.throws(() => stageBuild({ source, destination }), /单独修改/);
  assert.equal(readFileSync(join(destination, 'keep'), 'utf8'), '独立修改');
});

test('上游合法相对文件链接保持原链接及许可内容', context => {
  const { source, destination } = fixture(context);
  writeFileSync(join(source, 'COPYING'), '上游许可证'); symlinkSync('COPYING', join(source, 'LICENSE'));
  stageBuild({ source, destination });
  assert.equal(readFileSync(join(destination, 'LICENSE'), 'utf8'), '上游许可证');
  assert.deepEqual(sourceFiles(source), ['COPYING', 'LICENSE']);
});

test('桌面用途只暂存模型元数据和许可证，切换用途保留源码与构建缓存', context => {
  const { source, destination } = fixture(context);
  const files = {
    'third_party/openai-codex/LICENSE': 'license',
    'third_party/openai-codex/NOTICE': 'notice',
    'third_party/openai-codex/codex-rs/models-manager/models.json': '{}',
    'third_party/openai-codex/codex-rs/core/src/lib.rs': 'upstream',
    'third_party/openai-codex.release.json': 'release',
  };
  for (const [name, content] of Object.entries(files)) {
    mkdirSync(join(source, name, '..'), { recursive: true });
    writeFileSync(join(source, name), content);
  }
  const vendor = join(source, 'third_party/openai-codex/vendor');
  mkdirSync(vendor);
  writeFileSync(join(vendor, 'COPYING'), 'vendor license');
  symlinkSync('COPYING', join(vendor, 'LICENSE'));
  const full = stageBuild({ source, destination, profile: 'native-extensions' });
  mkdirSync(join(destination, 'target'));
  writeFileSync(join(destination, 'target/cache'), 'keep');
  const desktop = stageBuild({ source, destination });
  assert.equal(full.total, 7);
  assert.equal(desktop.total, 4);
  assert.equal(desktop.removed, 3);
  assert.equal(readFileSync(join(source, 'third_party/openai-codex/codex-rs/core/src/lib.rs'), 'utf8'), 'upstream');
  assert.equal(readFileSync(join(destination, 'target/cache'), 'utf8'), 'keep');
  assert.throws(() => sourceFiles(source, { profile: 'unknown' }), /未知暂存用途/);
});
