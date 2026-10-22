import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

export function windowsElectronDistribution(projectDir, expectedVersion) {
  const directory = join(projectDir, 'node_modules', 'electron', 'dist');
  if (!existsSync(join(directory, 'electron.exe')) || !existsSync(join(directory, 'version'))) {
    throw new Error('Windows 打包缺少已安装的 Electron 运行时，请先通过统一入口安装依赖');
  }
  const version = readFileSync(join(directory, 'version'), 'utf8').trim();
  if (version !== expectedVersion) throw new Error(`Electron 运行时版本不匹配：需要 ${expectedVersion}，实际 ${version}`);
  return directory;
}
