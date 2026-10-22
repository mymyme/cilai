import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertDebugPackageLocation, canonicalDebugBuildRoot } from './debug-package-location.mjs';
import { assertNoSymlinkPath } from '../../../scripts/personal-build.mjs';

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const project = resolve(desktop, '../..');
const buildRoot = canonicalDebugBuildRoot();
assertDebugPackageLocation({ profile: 'debug', projectRoot: project, buildRoot });
const runtime = join(desktop, 'node_modules/electron');
assertNoSymlinkPath(runtime);
const expected = JSON.parse(readFileSync(join(desktop, 'package.json'), 'utf8')).devDependencies.electron;
const installed = JSON.parse(readFileSync(join(runtime, 'package.json'), 'utf8')).version;
if (expected !== installed) throw new Error(`Electron 包版本不匹配：需要 ${expected}，已安装 ${installed}`);
const environment = { ...process.env, electron_config_cache: join(buildRoot, 'cache/electron/Cache') };
for (const key of ['ELECTRON_OVERRIDE_DIST_PATH', 'ELECTRON_MIRROR', 'ELECTRON_NIGHTLY_MIRROR',
  'ELECTRON_CUSTOM_DIR', 'ELECTRON_CUSTOM_FILENAME', 'electron_use_remote_checksums', 'npm_config_electron_use_remote_checksums',
  'ELECTRON_INSTALL_PLATFORM', 'ELECTRON_INSTALL_ARCH', 'npm_config_platform', 'npm_config_arch']) delete environment[key];
const result = spawnSync(process.execPath, [join(runtime, 'install.js')], { cwd: desktop, env: environment, stdio: 'inherit' });
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);
const version = readFileSync(join(runtime, 'dist/version'), 'utf8').trim().replace(/^v/, '');
if (version !== expected) throw new Error(`Electron 运行时版本不匹配：需要 ${expected}，实际 ${version}`);
console.log(`个人版 Electron 运行时已准备：${version}，${process.platform}/${process.arch}`);
