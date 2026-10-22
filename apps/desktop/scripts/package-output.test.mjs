import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { packageBundlePaths, packageOutputDirectory } from './package-output.mjs';

test('Windows 任意旧输出开关组合均只得到两个固定通道目录', () => {
  const outputs = new Set();
  for (const channel of ['testing', 'stable']) {
    for (const separateChannelOutput of ['', '1']) {
      for (const backgroundOutput of ['', '1']) {
        outputs.add(packageOutputDirectory({buildRoot:'D:\\构建根\\desktop',platform:'win32',
          channel,separateChannelOutput,backgroundOutput}));
      }
    }
  }
  assert.deepEqual([...outputs].sort(), [
    'D:\\构建根\\desktop\\target\\desktop-electron-stable',
    'D:\\构建根\\desktop\\target\\desktop-electron-testing',
  ]);
});

test('输出解析拒绝非法通道、相对根目录和无效旧开关', () => {
  const valid = {buildRoot:'D:\\build',platform:'win32',channel:'testing',separateChannelOutput:'',backgroundOutput:''};
  for (const extra of [{channel:'../other'}, {buildRoot:'relative'}, {separateChannelOutput:'0'},
    {backgroundOutput:'true'}, {platform:'linux'}]) {
    assert.throws(() => packageOutputDirectory({...valid,...extra}));
  }
});

test('macOS 任意有效旧开关均使用固定通道目录', () => {
  const base = {buildRoot:'/Users/test/.cilai-builds/desktop',platform:'darwin',channel:'testing',backgroundOutput:''};
  assert.equal(packageOutputDirectory({...base,separateChannelOutput:''}), '/Users/test/.cilai-builds/desktop/target/desktop-electron-testing');
  assert.equal(packageOutputDirectory({...base,separateChannelOutput:'1'}), '/Users/test/.cilai-builds/desktop/target/desktop-electron-testing');
  assert.throws(() => packageOutputDirectory({...base,backgroundOutput:'1'}));
});

test('同一通道的构建、自检、启动与发布按实际架构定位应用', () => {
  const output = '/Users/test/.cilai-builds/desktop/target/desktop-electron-testing';
  for (const [arch, directory] of [['arm64', 'mac-arm64'], ['x64', 'mac']]) {
    const bundle = `${output}/${directory}/词来工作台.app`;
    assert.deepEqual(packageBundlePaths({ output, platform: 'darwin', arch, productName: '词来工作台' }), {
      bundle, resources: `${bundle}/Contents/Resources`, executable: `${bundle}/Contents/MacOS/词来工作台`,
    });
  }
  assert.throws(() => packageBundlePaths({ output, platform: 'darwin', arch: 'ia32', productName: '词来工作台' }));
  for (const productName of ['../other', '.', '..']) {
    assert.throws(() => packageBundlePaths({ output, platform: 'darwin', arch: 'arm64', productName }));
  }
});

test('Windows PowerShell 使用的实际 Node 入口输出固定通道路径', {skip:process.platform !== 'win32'}, () => {
  const script = fileURLToPath(new URL('./package-output.mjs',import.meta.url));
  for (const channel of ['testing','stable']) {
    const result = spawnSync(process.execPath,[script,channel], {encoding:'utf8',env:{...process.env,
      CILAI_BUILD_ROOT:'D:\\中文 构建根\\desktop',CILAI_SEPARATE_CHANNEL_OUTPUT:'1',CILAI_BACKGROUND_OUTPUT:'1'}});
    assert.equal(result.status,0,result.stderr);
    assert.equal(result.stdout.trim(),`D:\\中文 构建根\\desktop\\target\\desktop-electron-${channel}`);
  }
});

test('Windows PowerShell 5.1 使用 OEM 编码也能完整读取中文输出目录', {skip:process.platform !== 'win32'}, () => {
  const script = fileURLToPath(new URL('./package-output.mjs',import.meta.url));
  const command = `$ErrorActionPreference='Stop'; $previous=[Console]::OutputEncoding;
    try { [Console]::OutputEncoding=[Text.Encoding]::GetEncoding(936);
      $raw=& $env:CILAI_TEST_NODE $env:CILAI_TEST_OUTPUT_SCRIPT testing --json;
      if ($LASTEXITCODE -ne 0) { throw 'node failed' };
      $path=$raw | ConvertFrom-Json;
      if ($path -cne $env:CILAI_TEST_EXPECTED_PATH) { throw 'path mismatch' };
      Write-Output 'PASS';
    } finally { [Console]::OutputEncoding=$previous }`;
  const result = spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',command],{encoding:'utf8',env:{...process.env,
    CILAI_BUILD_ROOT:'D:\\中文 构建根\\desktop',CILAI_TEST_NODE:process.execPath,CILAI_TEST_OUTPUT_SCRIPT:script,
    CILAI_TEST_EXPECTED_PATH:'D:\\中文 构建根\\desktop\\target\\desktop-electron-testing',
    CILAI_SEPARATE_CHANNEL_OUTPUT:'1',CILAI_BACKGROUND_OUTPUT:'1'}});
  assert.equal(result.status,0,result.stderr);
  assert.equal(result.stdout.trim(),'PASS');
});
