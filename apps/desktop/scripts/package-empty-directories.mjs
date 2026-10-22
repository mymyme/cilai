import { lstatSync, mkdirSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

export function restoreEmptyDirectories(sourceRoot, packagedRoot) {
  const pending = [];
  function inspect(source, packaged) {
    if (!lstatSync(source).isDirectory()) throw new Error(`随包目录来源无效：${source}`);
    const entries = readdirSync(source, { withFileTypes: true });
    const actual = lstatSync(packaged, { throwIfNoEntry: false });
    if (actual && !actual.isDirectory()) throw new Error(`随包目录目标不是普通目录：${packaged}`);
    if (entries.length === 0) {
      if (!actual) pending.push(packaged);
      else if (readdirSync(packaged).length !== 0) throw new Error(`随包空目录包含清单外内容：${packaged}`);
      return;
    }
    if (!actual) throw new Error(`产物缺少非空随包目录：${packaged}`);
    for (const entry of entries) {
      if (entry.isDirectory()) inspect(join(source, entry.name), join(packaged, entry.name));
      else if (!entry.isFile()) throw new Error(`随包目录来源包含非普通文件：${join(source, entry.name)}`);
    }
  }
  inspect(sourceRoot, packagedRoot);
  for (const directory of pending) mkdirSync(directory);
  return pending.length;
}
