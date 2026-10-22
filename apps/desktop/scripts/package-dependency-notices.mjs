import { spawnSync } from 'node:child_process';
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gunzipSync } from 'node:zlib';
import { readNoticeFile, noticeDigest } from './package-runtime-notices.mjs';

const noticeName = /^(licen[sc]es?|notices?|copying|copyright|authors|ofl)(?:$|[._-])/i;
const targetTriples = { 'darwin-arm64': 'aarch64-apple-darwin', 'darwin-x64': 'x86_64-apple-darwin', 'win32-x64': 'x86_64-pc-windows-msvc' };
const safeName = value => encodeURIComponent(value).replaceAll('%', '_');
const json = value => Buffer.from(JSON.stringify(value, null, 2) + '\n');

function listFiles(directory, { noticesOnly = false } = {}) {
  const result = [];
  function walk(current) {
    for (const entry of readdirSync(current, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
      if (entry.name === 'node_modules' || entry.name === '.git') continue;
      const file = join(current, entry.name);
      if (entry.isSymbolicLink()) {
        if (!noticesOnly || noticeName.test(entry.name)) throw new Error('依赖许可不得经过符号链接');
        continue;
      }
      if (entry.isDirectory()) walk(file);
      else if (entry.isFile() && (!noticesOnly || noticeName.test(entry.name))) result.push(relative(directory, file).split(sep).join('/'));
    }
  }
  walk(directory);
  return result;
}

function packageNotices(directory) {
  const files = new Map(listFiles(directory, { noticesOnly: true }).map(name => [name, readNoticeFile(join(directory, name))]));
  if (!files.size) {
    for (const name of readdirSync(directory).filter(name => /^readme(?:\.[a-z]+)?$/i.test(name))) {
      const content = readNoticeFile(join(directory, name));
      if (content.includes('Permission is hereby granted') && /copyright/i.test(content.toString())) files.set(name, content);
    }
  }
  return files;
}

function supplements(root) {
  const directory = join(root, 'third_party/dependency-notices');
  const manifest = JSON.parse(readNoticeFile(join(directory, 'provenance.json')));
  if (manifest.schemaVersion !== 1) throw new Error('依赖补充许可清单版本无效');
  return key => {
    const files = new Map();
    for (const name of manifest.components[key] ?? []) {
      const record = manifest.files[name];
      if (!record || name.includes('/') || name.includes('\\')) throw new Error('依赖补充许可路径无效');
      const content = readNoticeFile(join(directory, name));
      if (noticeDigest(content) !== record.sha256 || content.length !== record.bytes) throw new Error(`依赖补充许可摘要不符：${key}`);
      files.set(name, content);
    }
    return { files, sources: [...files.keys()].map(name => ({ path: name, ...manifest.files[name] })) };
  };
}

export function collectNpmNotices({ root, packageRoot, scope, supplement }) {
  const lock = JSON.parse(readNoticeFile(join(packageRoot, 'package-lock.json')));
  const components = [];
  const omitted = [];
  for (const [packagePath, locked] of Object.entries(lock.packages).sort(([a], [b]) => a.localeCompare(b, 'en'))) {
    if (!packagePath.includes('node_modules/') || locked.dev || locked.link) continue;
    const directory = resolve(packageRoot, packagePath);
    if (!directory.startsWith(resolve(packageRoot) + sep)) throw new Error('依赖许可来源越界');
    if (!existsSync(directory)) {
      if (!locked.optional) throw new Error(`缺少锁定运行依赖：${packagePath}`);
      omitted.push({ packagePath, version: locked.version, reason: 'optional dependency not installed for this target' });
      continue;
    }
    const metadata = JSON.parse(readNoticeFile(join(directory, 'package.json')));
    if (metadata.version !== locked.version) throw new Error(`依赖许可版本与锁不符：${packagePath}`);
    const key = `npm:${metadata.name}@${metadata.version}`;
    const extra = supplement(key);
    const files = packageNotices(directory);
    // Native canvas is an optional Node-only PDF.js dependency, outside the browser bundle.
    // Its matching parent distribution carries the shared upstream license.
    if (!files.size && /^@napi-rs\/canvas-/.test(metadata.name)) {
      const parent = join(root, 'node_modules/@napi-rs/canvas');
      if (JSON.parse(readNoticeFile(join(parent, 'package.json'))).version !== metadata.version) throw new Error('canvas许可与目标版本不符');
      for (const [name, value] of packageNotices(parent)) files.set(`parent-distribution/${name}`, value);
    }
    for (const [name, value] of extra.files) files.set(`upstream/${name}`, value);
    if (!files.size) throw new Error(`运行依赖缺少许可正文：${key}`);
    components.push({ key, ecosystem: 'npm', scope, name: metadata.name, version: metadata.version,
      license: metadata.license ?? locked.license ?? 'SEE INCLUDED LICENSE FILES', author: metadata.author ?? null,
      repository: metadata.repository ?? null, packagePath, integrity: locked.integrity ?? null,
      supplementalSources: extra.sources, files });
  }
  return { components, omitted };
}

export function collectCargoNotices({ root, target, supplement, metadata, cargoHome }) {
  if (!cargoHome || !existsSync(cargoHome)) throw new Error('依赖通知要求个人版CARGO_HOME');
  const allowed = realpathSync(cargoHome);
  const lockRecords = readNoticeFile(join(root, 'Cargo.lock')).toString().split('[[package]]').slice(1).map(block => {
    const values = {};
    for (const field of ['name', 'version', 'source', 'checksum']) {
      const match = block.match(new RegExp(`^${field} = (".*")$`, 'm'));
      if (match) values[field] = JSON.parse(match[1]);
    }
    return values;
  });
  const packages = new Map(metadata.packages.map(item => [item.id, item]));
  const nodes = new Map(metadata.resolve.nodes.map(item => [item.id, item]));
  const roots = metadata.packages.filter(item => ['contexthub-app-server', 'cilai-host-service'].includes(item.name) && !item.source);
  if (roots.length !== 2) throw new Error('Rust运行依赖根不完整');
  const queue = roots.map(item => item.id);
  const seen = new Set();
  while (queue.length) {
    const id = queue.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    const node = nodes.get(id);
    if (!node) throw new Error('Rust目标依赖图不完整');
    for (const dependency of node.deps) if (dependency.dep_kinds.some(kind => kind.kind === null || kind.kind === 'build')) queue.push(dependency.pkg);
  }
  const components = [];
  for (const id of [...seen].sort()) {
    const item = packages.get(id);
    if (!item.source) continue;
    if (!item.source.startsWith('registry+')) throw new Error(`尚未登记Rust依赖来源：${item.name}`);
    const directory = dirname(item.manifest_path);
    if (!realpathSync(directory).startsWith(allowed + sep)) throw new Error('Rust许可来源超出个人依赖缓存');
    const key = `cargo:${item.name}@${item.version}`;
    const locked = lockRecords.find(record => record.name === item.name && record.version === item.version && record.source === item.source);
    if (!locked?.checksum) throw new Error(`Rust许可来源未在锁文件登记：${key}`);
    const files = packageNotices(directory);
    if (item.license_file) {
      const file = resolve(directory, item.license_file);
      if (!file.startsWith(resolve(directory) + sep)) throw new Error('Rust声明许可路径越界');
      files.set(relative(directory, file).split(sep).join('/'), readNoticeFile(file));
    }
    const extra = supplement(key);
    const registry = dirname(directory).split(sep).at(-1);
    const archive = readNoticeFile(join(allowed, 'registry/cache', registry, `${item.name}-${item.version}.crate`));
    if (noticeDigest(archive) !== locked.checksum) throw new Error(`Rust来源归档摘要不符：${key}`);
    const upstream = crateFiles(archive);
    for (const [name, content] of files) {
      const original = upstream.get(`${item.name}-${item.version}/${name}`);
      if (!original?.equals(content)) throw new Error(`Rust缓存许可与官方归档不符：${key}/${name}`);
    }
    for (const [name, value] of extra.files) files.set(`upstream/${name}`, value);
    if (!files.size) throw new Error(`Rust依赖缺少许可正文：${key}`);
    components.push({ key, ecosystem: 'cargo', scope: 'host-and-app-server-target-closure-including-build-tools',
      name: item.name, version: item.version, license: item.license, authors: item.authors,
      repository: item.repository, registry: item.source, checksum: locked.checksum,
      supplementalSources: extra.sources, files });
  }
  return components;
}

function crateFiles(archive) {
  const data = gunzipSync(archive);
  const files = new Map();
  let extendedPath;
  for (let offset = 0; offset + 512 <= data.length;) {
    const header = data.subarray(offset, offset + 512);
    if (header.every(value => value === 0)) break;
    const field = (start, length) => header.subarray(start, start + length).toString().replace(/\0.*$/s, '');
    const size = Number.parseInt(field(124, 12).trim(), 8);
    if (!Number.isSafeInteger(size) || size < 0 || offset + 512 + size > data.length) throw new Error('Rust来源归档结构不完整');
    const content = data.subarray(offset + 512, offset + 512 + size);
    const type = field(156, 1);
    const name = [field(345, 155), field(0, 100)].filter(Boolean).join('/');
    if (type === 'L') extendedPath = content.toString().replace(/\0.*$/s, '');
    else if (type === 'x') {
      for (let at = 0; at < content.length;) {
        const space = content.indexOf(32, at);
        const length = Number.parseInt(content.subarray(at, space).toString(), 10);
        if (!Number.isSafeInteger(length) || length <= 0 || at + length > content.length) throw new Error('Rust来源归档扩展头无效');
        const record = content.subarray(space + 1, at + length - 1).toString();
        if (record.startsWith('path=')) extendedPath = record.slice(5);
        at += length;
      }
    } else {
      if (type === '0' || type === '') files.set(extendedPath ?? name, content);
      extendedPath = undefined;
    }
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  return files;
}

export function dependencyNoticeInputs({ desktopDir, platform, arch, cargoMetadata, supplementRoot }) {
  const target = targetTriples[`${platform}-${arch}`];
  if (!target) throw new Error('未登记依赖许可目标平台');
  const root = resolve(desktopDir, '../..');
  const supplement = supplements(supplementRoot ?? root);
  const ui = collectNpmNotices({ root, packageRoot: root, scope: 'frontend-declared-production-closure', supplement });
  const shell = collectNpmNotices({ root, packageRoot: desktopDir, scope: 'electron-main-production-closure', supplement });
  let metadata = cargoMetadata;
  if (!metadata) {
    const result = spawnSync(process.env.CARGO || 'cargo', ['metadata', '--locked', '--offline', '--format-version', '1', '--filter-platform', target],
      { cwd: root, env: process.env, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 });
    if (result.status !== 0) throw new Error('无法取得锁定Rust目标依赖图；依赖通知未生成');
    metadata = JSON.parse(result.stdout);
  }
  const cargo = collectCargoNotices({ root, target, supplement, metadata, cargoHome: process.env.CARGO_HOME });
  const components = [...ui.components, ...shell.components, ...cargo];
  const files = new Map();
  const records = components.map(component => {
    const directory = `${component.ecosystem}/${safeName(component.scope)}/${safeName(component.name)}-${component.version}`;
    const { files: notices, ...record } = component;
    const entries = [...notices].map(([path, content]) => {
      files.set(`${directory}/${path}`, content);
      return { path: `${directory}/${path}`, bytes: content.length, sha256: noticeDigest(content) };
    });
    return { ...record, files: entries };
  });
  const manifest = { schemaVersion: 1, platform, arch, target,
    scope: 'Conservative declared production npm and native target dependency closures. Includes installed optional and build-time components; not a claim that every listed component is present in the executable.',
    lockfiles: Object.fromEntries(['Cargo.lock', 'package-lock.json', 'apps/desktop/package-lock.json'].map(path => [path, noticeDigest(readNoticeFile(join(root, path)))])),
    omittedOptionalNpm: [...ui.omitted, ...shell.omitted], components: records };
  files.set('manifest.json', json(manifest));
  files.set('README.txt', Buffer.from('Third-party dependency notices\n\nOriginal upstream license, copyright and notice files are retained without replacing their terms. The manifest records component versions, lock checksums and file digests. Browser dependencies are collected conservatively from the production lock graph; Rust includes build and proc-macro dependencies of the two native services.\n\nlazy-val and several objc2 crates declare MIT but do not ship a full MIT text. Their exact upstream declarations are included together with a clearly named SPDX standard template. Placeholders in that template are not assertions about an upstream copyright year or holder. The objc2 upstream SDK licensing discussion is preserved unchanged.\n\nNode, Electron/Chromium, PDF.js, official Codex and first-party notices are also distributed in their existing adjacent locations. No Windows MinGit runtime is included in a macOS package.\n'));
  return { files, manifest };
}

export function verifyDependencyNoticeFiles(directory, files) {
  if (!existsSync(directory) || !lstatSync(directory).isDirectory() || lstatSync(directory).isSymbolicLink()) throw new Error('产物缺少依赖许可目录');
  const actual = listFiles(directory).sort();
  if (JSON.stringify(actual) !== JSON.stringify([...files.keys()].sort())) throw new Error('依赖许可目录清单不符');
  for (const [name, content] of files) if (!readNoticeFile(join(directory, name)).equals(content)) throw new Error(`依赖许可或摘要不符：${name}`);
}

export function installDependencyNotices(options) {
  const { files, manifest } = dependencyNoticeInputs(options);
  const directory = join(options.resources, 'licenses/dependencies');
  if (!existsSync(directory)) {
    for (const [name, content] of files) {
      const path = join(directory, name);
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, content, { flag: 'wx', mode: 0o644 });
    }
  }
  verifyDependencyNoticeFiles(directory, files);
  return { directory: 'licenses/dependencies', components: manifest.components.length, files: files.size, manifestSha256: noticeDigest(files.get('manifest.json')) };
}

export function verifyDependencyNotices(options) {
  const { files, manifest } = dependencyNoticeInputs(options);
  verifyDependencyNoticeFiles(join(options.resources, 'licenses/dependencies'), files);
  return { directory: 'licenses/dependencies', components: manifest.components.length, files: files.size, manifestSha256: noticeDigest(files.get('manifest.json')) };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [resources] = process.argv.slice(2);
  if (!resources) throw new Error('请提供待核对的产物Resources目录');
  console.log(JSON.stringify(verifyDependencyNotices({ desktopDir: resolve(dirname(fileURLToPath(import.meta.url)), '..'), resources, platform: process.platform, arch: process.arch })));
}
