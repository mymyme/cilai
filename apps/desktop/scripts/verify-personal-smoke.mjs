import { createHash, randomUUID } from 'node:crypto';
import { lstatSync, readFileSync, realpathSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { assertManagedPath } from '../../../scripts/storage-lifecycle.mjs';

const prefix = '[personal-check] ';
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
function plainFile(path) {
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink()) throw new Error('个人启动检查文件必须为普通文件');
  return stat;
}
function allowPlainTarget(path) {
  try { plainFile(path); } catch (error) { if (error.code !== 'ENOENT') throw error; }
}
function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
  }
  return (crc ^ 0xffffffff) >>> 0;
}
function validatePng(bytes) {
  if (!bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) throw new Error('个人启动截图不是 PNG');
  let offset = 8, header = false, image = false, ended = false;
  while (offset + 12 <= bytes.length) {
    const size = bytes.readUInt32BE(offset), end = offset + 12 + size;
    if (end > bytes.length) throw new Error('个人启动截图数据不完整');
    const type = bytes.toString('ascii', offset + 4, offset + 8);
    if (crc32(bytes.subarray(offset + 4, end - 4)) !== bytes.readUInt32BE(end - 4)) throw new Error('个人启动截图校验失败');
    if (!header && type !== 'IHDR') throw new Error('个人启动截图缺少头信息');
    if (type === 'IHDR') {
      if (header || size !== 13 || !bytes.readUInt32BE(offset + 8) || !bytes.readUInt32BE(offset + 12)) throw new Error('个人启动截图头信息无效');
      header = true;
    }
    if (type === 'IDAT' && size) image = true;
    offset = end;
    if (type === 'IEND') { if (size) throw new Error('个人启动截图结束信息无效'); ended = true; break; }
  }
  if (!header || !image || !ended || offset !== bytes.length) throw new Error('个人启动截图数据不完整');
}
function registeredTemporary(environment, log) {
  if (!environment.CILAI_BUILD_ROOT || !isAbsolute(environment.CILAI_BUILD_ROOT) || !isAbsolute(environment.CILAI_BUILD_RUN_TMP)) throw new Error('个人启动检查缺少构建目录登记');
  const root = realpathSync(environment.CILAI_BUILD_ROOT);
  if (root !== resolve(environment.CILAI_BUILD_ROOT)) throw new Error('个人构建根路径包含符号链接');
  const temporary = assertManagedPath(root, resolve(environment.CILAI_BUILD_RUN_TMP));
  const run = dirname(temporary), id = basename(run);
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(id)
    || temporary !== join(root, 'tmp/storage-runs', id, 'tmp') || realpathSync(temporary) !== temporary) throw new Error('个人启动临时目录登记无效');
  assertManagedPath(root, log);
  const read = path => { assertManagedPath(root, path); plainFile(path); return JSON.parse(readFileSync(path, 'utf8')); };
  const owner = read(join(run, 'owner.json'));
  const index = read(join(root, 'target/storage-reports/index.json'));
  const reportPath = join(root, 'target/storage-reports', `build-${id}.json`);
  const report = read(reportPath);
  if (owner.schema !== 'cilai-build-storage-v1' || owner.id !== id || owner.root !== root || !Number.isSafeInteger(owner.pid) || owner.pid < 1
    || index.schema !== 'cilai-build-storage-index-v1' || index.root !== root || !Array.isArray(index.ids) || !index.ids.includes(id)
    || report.schema !== owner.schema || report.root !== root || report.id !== id || report.run !== run || report.temporary !== temporary
    || report.report !== reportPath || report.pid !== owner.pid || report.startedAt !== owner.startedAt || report.status !== 'running') throw new Error('个人启动临时目录未完成有效登记');
  return { root, temporary };
}

export function verifyPersonalSmoke(file, { environment = process.env, exitCode = 0 } = {}) {
  if (exitCode !== 0) throw new Error('个人启动检查进程未成功退出');
  if (!file) throw new Error('缺少个人启动检查日志');
  plainFile(resolve(file));
  const log = realpathSync(file), original = readFileSync(log, 'utf8');
  const lines = original.split(/\r?\n/), matches = lines.map((line, index) => line.startsWith(prefix) ? index : -1).filter(index => index >= 0);
  if (matches.length !== 1) throw new Error('个人启动检查未完成或结果不唯一');
  const value = JSON.parse(lines[matches[0]].slice(prefix.length));
  if (value.result !== 'passed' || value.platform !== process.platform || value.arch !== process.arch
    || value.memoryEnabled !== false || value.ui?.navigation?.join(',') !== '工作台,对话,项目,文件,自动化,技能,应用'
    || value.ui.login || value.ui.alerts?.length || typeof value.screenshot !== 'string' || !isAbsolute(value.screenshot)) throw new Error('个人启动检查结果不完整');
  plainFile(value.screenshot);
  const screenshot = realpathSync(value.screenshot), bytes = readFileSync(screenshot);
  validatePng(bytes);
  const digest = sha256(bytes);
  if (value.screenshotSha256 !== undefined && value.screenshotSha256 !== digest) throw new Error('个人启动截图摘要不一致');
  const target = join(dirname(log), 'headless-check.png');
  allowPlainTarget(target);
  if (screenshot === target) {
    if (value.screenshotSha256 !== digest) throw new Error('归档截图缺少完整性摘要');
    return { ...value, archived: true };
  }
  if (!environment.CILAI_BUILD_RUN_TMP) return { ...value, archived: false };
  const { root, temporary } = registeredTemporary(environment, log);
  if (assertManagedPath(temporary, resolve(value.screenshot)) !== screenshot) throw new Error('个人启动截图来源路径不一致');
  assertManagedPath(root, target);
  const updated = { ...value, screenshot: target, screenshotSha256: digest };
  const next = original.replace(/^\[personal-check\] [^\r\n]*/m, () => prefix + JSON.stringify(updated));
  const imagePending = join(dirname(log), `.headless-check.${randomUUID()}.png.pending`);
  const logPending = `${log}.${randomUUID()}.pending`;
  try {
    writeFileSync(imagePending, bytes, { flag: 'wx', mode: 0o600 });
    if (sha256(readFileSync(imagePending)) !== digest) throw new Error('归档截图写入校验失败');
    writeFileSync(logPending, next, { flag: 'wx', mode: plainFile(log).mode & 0o777 });
    if (readFileSync(log, 'utf8') !== original) throw new Error('个人启动检查日志在归档期间发生变化');
    assertManagedPath(root, target); allowPlainTarget(target);
    assertManagedPath(root, log); plainFile(log);
    renameSync(imagePending, target);
    renameSync(logPending, log);
  } finally {
    rmSync(imagePending, { force: true });
    rmSync(logPending, { force: true });
  }
  return { ...updated, archived: true };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    verifyPersonalSmoke(process.argv[2]);
    console.log('个人版无模型启动与七个主入口检查通过；完整首版能力另行验收');
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
