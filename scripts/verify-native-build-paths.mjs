import {readFileSync} from 'node:fs';
import {basename, resolve} from 'node:path';
import {pathToFileURL} from 'node:url';

export function verifyNativeBuildPaths(files, privatePrefixes) {
  const prefixes = privatePrefixes.filter(Boolean).map(value => Buffer.from(value));
  if (!prefixes.length || !files.length) throw new Error('缺少原生构建路径检查参数');
  for (const file of files) {
    const bytes = readFileSync(file);
    if (prefixes.some(prefix => bytes.includes(prefix))) {
      throw new Error(`原生组件包含构建机绝对路径：${basename(file)}`);
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [buildRoot, sysroot, ...files] = process.argv.slice(2);
  verifyNativeBuildPaths(files, [buildRoot, sysroot]);
  console.log(`已核对 ${files.length} 个原生组件，无构建机路径`);
}
