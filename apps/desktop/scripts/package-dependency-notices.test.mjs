import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { gzipSync } from 'node:zlib';
import test from 'node:test';
import { collectCargoNotices, collectNpmNotices, verifyDependencyNoticeFiles } from './package-dependency-notices.mjs';
import { noticeDigest } from './package-runtime-notices.mjs';

function fixture(t) {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'cilai-dependency-notices-'));
  t.after(() => rmSync(root, { force: true, recursive: true }));
  const write = (name, value) => { const path = join(root, name); mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, value); return path; };
  const supplement = () => ({ files: new Map(), sources: [] });
  return { root, write, supplement };
}

test('npm仅收集已安装运行依赖，保留README中的原文许可并登记可选缺项', t => {
  const f = fixture(t);
  f.write('package-lock.json', JSON.stringify({ packages: {
    'node_modules/example': { version: '1.0.0', license: 'MIT' },
    'node_modules/dev-only': { version: '1.0.0', dev: true },
    'node_modules/other-platform': { version: '1.0.0', optional: true },
  } }));
  f.write('node_modules/example/package.json', JSON.stringify({ name: 'example', version: '1.0.0', license: 'MIT' }));
  f.write('node_modules/example/README.md', 'Example\nCopyright Example\nPermission is hereby granted, fixture license text.\n');
  const result = collectNpmNotices({ ...f, packageRoot: f.root, scope: 'test' });
  assert.equal(result.components.length, 1);
  assert.equal(result.components[0].key, 'npm:example@1.0.0');
  assert.deepEqual(result.components[0].files.get('README.md'), readFileSync(join(f.root, 'node_modules/example/README.md')));
  assert.equal(result.omitted.length, 1);
});

test('npm缺少正文、版本漂移和缺少非可选包均阻止生成', t => {
  const f = fixture(t);
  f.write('package-lock.json', JSON.stringify({ packages: { 'node_modules/example': { version: '1.0.0' } } }));
  const call = () => collectNpmNotices({ ...f, packageRoot: f.root, scope: 'test' });
  assert.throws(call, /缺少锁定运行依赖/);
  f.write('node_modules/example/package.json', JSON.stringify({ name: 'example', version: '2.0.0' }));
  assert.throws(call, /版本与锁不符/);
  f.write('node_modules/example/package.json', JSON.stringify({ name: 'example', version: '1.0.0', license: 'MIT' }));
  assert.throws(call, /缺少许可正文/);
});

function crateFixture(t) {
  const f = fixture(t);
  const text = Buffer.from('Copyright Example\nOriginal fixture license.\n');
  const header = Buffer.alloc(512);
  header.write('example-1.0.0/LICENSE');
  header.write(text.length.toString(8).padStart(11, '0') + '\0', 124);
  header.write('0', 156);
  const archive = gzipSync(Buffer.concat([header, text, Buffer.alloc(512 - text.length), Buffer.alloc(1024)]));
  const cargoHome = join(f.root, 'cargo');
  f.write('cargo/registry/cache/registry/example-1.0.0.crate', archive);
  const licensePath = f.write('cargo/registry/src/registry/example-1.0.0/LICENSE', text);
  const manifest = f.write('cargo/registry/src/registry/example-1.0.0/Cargo.toml', 'fixture');
  f.write('Cargo.lock', `[[package]]\nname = "example"\nversion = "1.0.0"\nsource = "registry+fixture"\nchecksum = "${noticeDigest(archive)}"\n`);
  const roots = ['contexthub-app-server', 'cilai-host-service'].map(name => ({ id: name, name, source: null }));
  const metadata = { packages: [...roots, { id: 'example', name: 'example', version: '1.0.0', source: 'registry+fixture', manifest_path: manifest, license: 'MIT', authors: ['Example'], repository: 'https://example.invalid' }], resolve: { nodes: [
    { id: roots[0].id, deps: [{ pkg: 'example', dep_kinds: [{ kind: null }] }, { pkg: 'unavailable-dev-dependency', dep_kinds: [{ kind: 'dev' }] }] },
    { id: roots[1].id, deps: [] }, { id: 'example', deps: [] },
  ] } };
  return { ...f, cargoHome, metadata, target: 'test', licensePath };
}

test('Rust通知来自锁定目标闭包，原文逐字匹配已校验的crate归档', t => {
  const f = crateFixture(t);
  const components = collectCargoNotices(f);
  assert.equal(components.length, 1);
  assert.equal(components[0].name, 'example');
  assert.deepEqual(components[0].files.get('LICENSE'), readFileSync(f.licensePath));
  writeFileSync(f.licensePath, 'modified cache license');
  assert.throws(() => collectCargoNotices(f), /缓存许可与官方归档不符/);
});

test('Rust归档损坏或锁摘要变动时明确停止', t => {
  const f = crateFixture(t);
  f.write('cargo/registry/cache/registry/example-1.0.0.crate', 'invalid archive');
  assert.throws(() => collectCargoNotices(f), /来源归档摘要不符/);
});

test('最终通知被删减、篡改、多出文件或换成链接均验证失败', t => {
  const f = fixture(t);
  const files = new Map([['component/LICENSE', Buffer.from('original')]]);
  assert.throws(() => verifyDependencyNoticeFiles(join(f.root, 'output'), files), /缺少/);
  const path = f.write('output/component/LICENSE', 'original');
  verifyDependencyNoticeFiles(join(f.root, 'output'), files);
  f.write('output/component/LICENSE', 'changed');
  assert.throws(() => verifyDependencyNoticeFiles(join(f.root, 'output'), files), /不符/);
  f.write('output/component/LICENSE', 'original');
  const extra = f.write('output/extra.txt', 'extra');
  assert.throws(() => verifyDependencyNoticeFiles(join(f.root, 'output'), files), /清单不符/);
  rmSync(extra);
  rmSync(path);
  symlinkSync(f.write('original-license', 'original'), path);
  assert.throws(() => verifyDependencyNoticeFiles(join(f.root, 'output'), files), /链接/);
});
