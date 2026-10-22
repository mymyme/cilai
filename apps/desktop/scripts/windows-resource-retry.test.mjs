import assert from "node:assert/strict";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';
import test from "node:test";
import { installWindowsResourceRetry } from "./windows-resource-retry.mjs";
import { copyFile, mkdtemp, mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';

const out = resolve("fixture-package");
const executable = join(out, "词来工作台.exe");
const busy = (path = executable) => Object.assign(new Error("locked"), { code: "EBUSY", syscall: "open", path });
const context = (edit) => ({
  electronPlatformName: "win32", appOutDir: out,
  packager: { appInfo: { productFilename: "词来工作台" }, signAndEditResources: edit },
});

async function offlineAfterPack(fixtureRoot) {
  const sourceRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
  const fixtureSource = join(fixtureRoot, 'project');
  const copied = new Set();
  async function copy(relativeName) {
    const source = resolve(sourceRoot, relativeName);
    const name = relative(sourceRoot, source);
    assert.ok(name !== '..' && !name.startsWith('../'));
    const target = join(fixtureSource, name);
    await mkdir(dirname(target), { recursive: true });
    await copyFile(source, target);
    return target;
  }
  async function module(relativeName) {
    const name = relative(sourceRoot, resolve(sourceRoot, relativeName));
    if (copied.has(name)) return;
    copied.add(name);
    await copy(name);
    const text = await readFile(join(sourceRoot, name), 'utf8');
    const require = createRequire(join(sourceRoot, name));
    for (const match of text.matchAll(/(?:\bfrom\s*|\bimport\s*|\brequire\s*\(\s*)["'](\.[^"']+)["']/g)) {
      await module(relative(sourceRoot, require.resolve(match[1])));
    }
  }
  await module('apps/desktop/scripts/after-pack.mjs');
  for (const name of ['package.json', 'package-lock.json', 'LICENSE', 'NOTICE', 'Cargo.toml', 'apps/desktop/package.json',
    'apps/desktop/package-lock.json', 'apps/desktop-ui/package.json', 'packages/app-ui/package.json',
    'packages/app-ui/LICENSE', 'packages/app-ui/NOTICE', 'third_party/openai-codex.release.json']) await copy(name);
  const write = async (name, value) => {
    const target = join(fixtureSource, name);
    await mkdir(dirname(target), { recursive: true });
    await writeFile(target, value);
  };
  const electronVersion = JSON.parse(await readFile(join(fixtureSource, 'apps/desktop/package.json'), 'utf8')).devDependencies.electron;
  await write('apps/desktop/node_modules/electron/package.json', JSON.stringify({ version: electronVersion }));
  await write('apps/desktop/node_modules/electron/dist/version', electronVersion);
  await write('apps/desktop/node_modules/electron/dist/LICENSE', 'fixture Electron license\r\n');
  await write('apps/desktop/node_modules/electron/dist/LICENSES.chromium.html', '<html>fixture Chromium notices</html>\n');
  const pdfVersion = JSON.parse(await readFile(join(fixtureSource, 'apps/desktop-ui/package.json'), 'utf8')).dependencies['pdfjs-dist'];
  await write('node_modules/pdfjs-dist/package.json', JSON.stringify({ version: pdfVersion }));
  for (const name of ['LICENSE', 'wasm/LICENSE_OPENJPEG', 'wasm/LICENSE_PDFJS_OPENJPEG', 'wasm/LICENSE_JBIG2', 'wasm/LICENSE_PDFJS_JBIG2']) {
    await write('node_modules/pdfjs-dist/' + name, 'fixture ' + name + '\n');
  }
  for (const name of ['openjpeg_nowasm_fallback.js', 'jbig2_nowasm_fallback.js']) {
    await write('node_modules/pdfjs-dist/wasm/' + name, 'fixture ' + name);
    const packaged = join(fixtureRoot, 'output/resources/frontend/pdfjs/wasm', name);
    await mkdir(dirname(packaged), { recursive: true });
    await writeFile(packaged, 'fixture ' + name);
  }
  return (await import(pathToFileURL(join(fixtureSource, 'apps/desktop/scripts/after-pack.mjs')))).default;
}

test("Windows 资源编辑重试保留签名调用、参数及结果，不重复安装包装", async () => {
  const waits = [];
  const warnings = [];
  let calls = 0;
  const args = [executable, "x64", "output", "词来工作台", "asInvoker"];
  const ctx = context(async function (...received) {
    assert.equal(this, ctx.packager);
    assert.deepEqual(received, args);
    if (++calls < 3) throw busy();
    return "resource-and-signing-complete";
  });
  installWindowsResourceRetry(ctx, { wait: async (ms) => waits.push(ms), warn: (message) => warnings.push(message) });
  installWindowsResourceRetry(ctx);
  assert.equal(await ctx.packager.signAndEditResources(...args), "resource-and-signing-complete");
  assert.equal(calls, 3);
  assert.deepEqual(waits, [1000, 2000]);
  assert.equal(warnings.length, 2);
});

test("持续占用在有限次数后保留原错误", async () => {
  const failure = busy();
  let calls = 0;
  const ctx = context(async () => { calls += 1; throw failure; });
  installWindowsResourceRetry(ctx, { wait: async () => {}, warn: () => {} });
  await assert.rejects(ctx.packager.signAndEditResources(executable), (error) => error === failure);
  assert.equal(calls, 5);
});

test("非占用、非打开操作和其他文件的失败立即上报", async () => {
  for (const failure of [
    Object.assign(busy(), { code: "EACCES" }),
    Object.assign(busy(), { syscall: "write" }),
    busy(join(out, "helper.exe")),
    new Error("signing failed"),
  ]) {
    let calls = 0;
    const ctx = context(async () => { calls += 1; throw failure; });
    installWindowsResourceRetry(ctx, { wait: async () => assert.fail("must not retry") });
    await assert.rejects(ctx.packager.signAndEditResources(executable), (error) => error === failure);
    assert.equal(calls, 1);
  }
});

test("仅 Windows 安装资源重试，打包器接口漂移明确失败", async t => {
  installWindowsResourceRetry({ electronPlatformName: "darwin" });
  assert.throws(() => installWindowsResourceRetry(context(undefined)), /资源编辑接口不可用/);
  const ctx = context(async () => "done");
  const fixtureRoot = await mkdtemp(join(await realpath(tmpdir()), 'cilai-after-pack-'));
  t.after(async () => { await rm(fixtureRoot, { recursive: true, force: true }); });
  ctx.appOutDir = join(fixtureRoot, 'output');
  const stagedDir = join(fixtureRoot, 'source');
  await Promise.all([
    mkdir(join(stagedDir, 'codex-runtime'), { recursive: true }),
    mkdir(join(ctx.appOutDir, 'resources', 'sidecar', 'codex-runtime'), { recursive: true }),
  ]);
  const environment = { CILAI_PACKAGE_ARCH: 'x64', CILAI_PACKAGE_PROFILE: 'debug', CILAI_UPDATE_CHANNEL: 'testing',
    CILAI_PACKAGE_DISTRIBUTION: 'local', CILAI_SIDECAR_STAGE: stagedDir };
  const previous = Object.fromEntries(Object.keys(environment).map(key => [key, process.env[key]]));
  Object.assign(process.env, environment);
  t.after(async () => {
    for (const [key, value] of Object.entries(previous)) { if (value === undefined) delete process.env[key]; else process.env[key] = value; }
  });
  const original = ctx.packager.signAndEditResources;
  const afterPack = await offlineAfterPack(fixtureRoot);
  await afterPack(ctx);
  assert.notEqual(ctx.packager.signAndEditResources, original);
  assert.equal(await ctx.packager.signAndEditResources(executable), "done");
});
