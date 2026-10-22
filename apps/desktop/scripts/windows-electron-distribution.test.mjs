import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { windowsElectronDistribution } from './windows-electron-distribution.mjs';
test('Windows 打包只使用版本匹配的已安装运行时，缺失或错误时明确失败', async () => {
  const root = await mkdtemp(join(tmpdir(),'cilai-electron-dist-'));
  assert.throws(()=>windowsElectronDistribution(root,'44.4.4'),/缺少已安装/);
  const directory=join(root,'node_modules/electron/dist'); await mkdir(directory,{recursive:true});
  await writeFile(join(directory,'electron.exe'),'fixture'); await writeFile(join(directory,'version'),'43.0.0');
  assert.throws(()=>windowsElectronDistribution(root,'44.4.4'),/版本不匹配/);
  await writeFile(join(directory,'version'),'44.4.4\n');
  assert.equal(windowsElectronDistribution(root,'44.4.4'),directory);
});
