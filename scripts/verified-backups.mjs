import { createHash, randomUUID } from 'node:crypto';
import { constants, closeSync, chmodSync, copyFileSync, fstatSync, lstatSync, mkdirSync, openSync, readFileSync, readdirSync, readlinkSync, readSync, realpathSync, renameSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, parse, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const TOOL = 'cilai-verified-backups';
const VERSION = 1;
const REGISTRY = '.verified-backups.json';
const LOCK = '.verified-backups.lock';
const OWNER = '.backup-owner.json';
const SHA = /^[a-f0-9]{64}$/;
const ID = /^backup-[a-f0-9-]{36}$/;
const digest = (value) => createHash('sha256').update(value).digest('hex');
const encode = (value) => JSON.stringify(value);
const within = (parent, child) => child === parent || child.startsWith(`${parent}${sep}`);
const present = (path) => { try { lstatSync(path); return true; } catch (error) { if (error.code === 'ENOENT') return false; throw error; } };
function fail(message) { throw new Error(message); }

function directory(path, { missing = false } = {}) {
  if (typeof path !== 'string' || !isAbsolute(path) || path.split(/[\\/]/).includes('..')) fail('目录必须使用不含上级跳转的绝对路径');
  const canonical = resolve(path);
  if (canonical === parse(canonical).root) fail('禁止使用文件系统根目录');
  let current = parse(canonical).root;
  for (const part of relative(current, canonical).split(sep)) {
    current = join(current, part);
    if (!present(current)) { if (missing) continue; fail(`目录不存在：${current}`); }
    const st = lstatSync(current);
    if (!st.isDirectory() || st.isSymbolicLink()) fail(`目录包含符号链接或非目录：${current}`);
  }
  return canonical;
}
function inputs({ source, backupRoot, dataset }, allowMissing = true) {
  if (typeof dataset !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,79}$/.test(dataset)) fail('dataset 必须为明确的短标识');
  const root = directory(backupRoot, { missing: allowMissing });
  const src = source === undefined ? undefined : directory(source);
  if (src && (within(src, root) || within(root, src))) fail('源目录与备份目录不得相同或互相包含');
  return { source: src, backupRoot: root, dataset };
}
function plainJson(path) {
  const st = lstatSync(path);
  if (!st.isFile() || st.isSymbolicLink()) fail(`元数据必须为普通文件：${path}`);
  return JSON.parse(readFileSync(path, 'utf8'));
}
function atomicJson(path, value) {
  const temp = `${path}.${randomUUID()}.tmp`;
  writeFileSync(temp, `${encode(value)}\n`, { flag: 'wx', mode: 0o600 });
  renameSync(temp, path);
}
function registryAt(root, required = true) {
  const path = join(root, REGISTRY);
  if (!present(path)) { if (!required) return null; fail('备份根尚未由本工具登记'); }
  const reg = plainJson(path);
  if (reg.tool !== TOOL || reg.version !== VERSION || reg.root !== root || typeof reg.rootId !== 'string' || !Array.isArray(reg.backups) || !reg.datasets || typeof reg.datasets !== 'object') fail('备份根登记信息无效');
  const seen = new Set(), sequences = new Set();
  for (const item of reg.backups) {
    if (!ID.test(item.id) || seen.has(item.id) || !Number.isSafeInteger(item.sequence) || item.sequence < 1 || sequences.has(item.sequence) || !['creating', 'failed', 'verified'].includes(item.status) || typeof item.dataset !== 'string' || item.source !== reg.datasets[item.dataset]) fail('备份登记条目无效');
    if (item.status === 'verified' && !SHA.test(item.manifestSha256)) fail('已验证备份缺少清单摘要');
    seen.add(item.id); sequences.add(item.sequence);
  }
  return reg;
}
function withRootLock(root, action) {
  directory(root);
  const path = join(root, LOCK);
  let fd;
  try { fd = openSync(path, 'wx', 0o600); } catch (error) { if (error.code === 'EEXIST') fail('备份根已被占用；遗留锁必须人工核实后处理'); throw error; }
  try { writeFileSync(fd, encode({ tool: TOOL, pid: process.pid, token: randomUUID() })); return action(); }
  finally { closeSync(fd); rmSync(path); }
}
function hashFile(path, initial) {
  const fd = openSync(path, constants.O_RDONLY);
  try {
    const opened = fstatSync(fd);
    if (!opened.isFile() || opened.dev !== initial.dev || opened.ino !== initial.ino) fail(`读取期间文件被替换：${path}`);
    const h = createHash('sha256'), buffer = Buffer.allocUnsafe(1024 * 1024);
    let n;
    while ((n = readSync(fd, buffer, 0, buffer.length, null)) > 0) h.update(buffer.subarray(0, n));
    const after = fstatSync(fd), current = lstatSync(path);
    if ([after, current].some((s) => !s.isFile() || s.dev !== initial.dev || s.ino !== initial.ino || s.size !== initial.size || s.mtimeMs !== initial.mtimeMs || s.ctimeMs !== initial.ctimeMs)) fail(`读取期间源文件发生变化：${path}`);
    return h.digest('hex');
  } finally { closeSync(fd); }
}
function snapshot(root) {
  directory(root);
  const entries = [];
  function visit(dir, prefix = '') {
    for (const name of readdirSync(dir).sort()) {
      const path = join(dir, name), key = prefix ? `${prefix}/${name}` : name;
      if (name.includes('\\')) fail('不支持包含反斜杠的文件名');
      const st = lstatSync(path);
      if (st.isSymbolicLink()) {
        const target = readlinkSync(path);
        if (isAbsolute(target) || !within(root, resolve(dirname(path), target))) fail(`符号链接逃出源目录：${key}`);
        let actual;
        try { actual = realpathSync(path); } catch { fail(`符号链接目标不存在或循环：${key}`); }
        if (!within(root, actual)) fail(`符号链接间接逃出源目录：${key}`);
        entries.push({ path: key, kind: 'link', target });
      } else if (st.isDirectory()) {
        entries.push({ path: key, kind: 'directory', mode: st.mode & 0o777 });
        visit(path, key);
      } else if (st.isFile()) entries.push({ path: key, kind: 'file', mode: st.mode & 0o777, size: st.size, sha256: hashFile(path, st) });
      else fail(`不支持特殊文件：${key}`);
    }
  }
  visit(root);
  return { rootMode: lstatSync(root).mode & 0o777, entries };
}
function copySnapshot(source, dest, snap) {
  mkdirSync(dest, { mode: 0o700 });
  for (const entry of snap.entries) {
    const target = join(dest, entry.path);
    if (entry.kind === 'directory') mkdirSync(target, { mode: 0o700 });
    else if (entry.kind === 'link') symlinkSync(entry.target, target);
    else { copyFileSync(join(source, entry.path), target, constants.COPYFILE_EXCL); chmodSync(target, entry.mode); }
  }
  for (const entry of [...snap.entries].reverse()) if (entry.kind === 'directory') chmodSync(join(dest, entry.path), entry.mode);
  chmodSync(dest, snap.rootMode);
}
function sourceGuard(options, source) {
  if (options.sourceGuard && typeof options.sourceGuard.assertHeld === 'function') {
    return () => {
      const proof = options.sourceGuard.assertHeld();
      if (!proof || typeof proof.then === 'function' || proof.source !== source || proof.exclusive !== true) fail('源目录的独占写保护未持有');
    };
  }
  if (!options.guardFile || !options.guardToken) fail('创建备份必须提供已持有的源目录独占写保护，不能仅靠前后校验代替');
  const guardFile = resolve(options.guardFile);
  directory(dirname(guardFile));
  return () => {
    const guard = plainJson(guardFile);
    if (guard.tool !== 'cilai-source-write-guard' || guard.version !== 1 || guard.source !== source || guard.exclusive !== true || guard.token !== options.guardToken || !Number.isSafeInteger(guard.pid) || guard.pid < 1) fail('源目录独占写保护无效');
    try { process.kill(guard.pid, 0); } catch { fail('源目录独占写保护进程不可用'); }
  };
}
function controlled(root, reg, record) {
  const path = directory(join(root, record.id));
  const owner = plainJson(join(path, OWNER));
  if (owner.tool !== TOOL || owner.version !== VERSION || owner.rootId !== reg.rootId || owner.id !== record.id || owner.dataset !== record.dataset || owner.source !== record.source) fail('备份归属不匹配，禁止处理');
  const allowed = new Set([OWNER, 'manifest.json', 'receipt.json', 'data', 'PINNED', 'PENDING_RECOVERY']);
  if (readdirSync(path).some((name) => !allowed.has(name))) fail('备份目录含未登记的额外内容，禁止自动清理');
  return path;
}
function verified(root, reg, record) {
  if (record.status !== 'verified') fail('备份尚未完成验证');
  const path = controlled(root, reg, record);
  const receipt = plainJson(join(path, 'receipt.json'));
  const manifestPath = join(path, 'manifest.json');
  const manifest = plainJson(manifestPath);
  if (receipt.status !== 'verified' || receipt.id !== record.id || receipt.dataset !== record.dataset || receipt.source !== record.source || receipt.manifestSha256 !== record.manifestSha256 || digest(encode(manifest)) !== record.manifestSha256 || manifest.tool !== TOOL || manifest.version !== VERSION || manifest.source !== record.source || manifest.dataset !== record.dataset) fail('备份回执或清单与登记不一致');
  const actual = snapshot(join(path, 'data'));
  if (encode(actual) !== encode(manifest.snapshot)) fail('备份文件与完整校验清单不一致');
  return { id: record.id, path, dataset: record.dataset, status: 'verified', files: actual.entries.filter((x) => x.kind === 'file').length, bytes: actual.entries.reduce((n, x) => n + (x.size ?? 0), 0) };
}
function protectedBackup(path, record) {
  const receipt = plainJson(join(path, 'receipt.json'));
  return record.pinned === true || record.pendingRecovery === true || receipt.pinned === true || receipt.pendingRecovery === true || present(join(path, 'PINNED')) || present(join(path, 'PENDING_RECOVERY'));
}
function pruneInside(root, dataset, reg, dryRun) {
  if (!Object.hasOwn(reg.datasets, dataset)) fail('dataset 尚未登记');
  const candidates = reg.backups.filter((x) => x.dataset === dataset && x.status === 'verified').sort((a, b) => b.sequence - a.sequence);
  const retained = candidates.slice(0, 2);
  const verifiedRetained = retained.map((x) => verified(root, reg, x));
  const remove = [], protectedIds = [];
  for (const record of candidates.slice(2)) {
    const path = controlled(root, reg, record);
    if (protectedBackup(path, record)) { protectedIds.push(record.id); continue; }
    verified(root, reg, record);
    remove.push(record);
  }
  if (!dryRun) {
    for (const record of remove) {
      for (const keep of retained) verified(root, reg, keep);
      const path = controlled(root, reg, record);
      if (protectedBackup(path, record)) fail('备份清理前被标记保留，已停止');
      verified(root, reg, record);
      rmSync(path, { recursive: true });
      reg.backups = reg.backups.filter((x) => x.id !== record.id);
      atomicJson(join(root, REGISTRY), reg);
    }
  }
  return { dryRun, dataset, retained: verifiedRetained.map((x) => x.id), protected: protectedIds, [dryRun ? 'wouldDelete' : 'deleted']: remove.map((x) => x.id) };
}

export function verifyBackup(options) {
  const { backupRoot, dataset } = inputs(options, false);
  const reg = registryAt(backupRoot);
  if (!ID.test(options.id ?? '')) fail('需要明确的备份 id');
  const record = reg.backups.find((x) => x.id === options.id && x.dataset === dataset);
  if (!record) fail('备份未登记在指定 dataset，禁止处理');
  return verified(backupRoot, reg, record);
}

export function pruneBackups(options) {
  const { backupRoot, dataset } = inputs(options, false);
  const run = () => pruneInside(backupRoot, dataset, registryAt(backupRoot), options.dryRun === true);
  return options.dryRun === true ? run() : withRootLock(backupRoot, run);
}

export function createVerifiedBackup(options) {
  const { source, backupRoot, dataset } = inputs(options);
  if (!source) fail('创建备份必须明确 source');
  const assertHeld = sourceGuard(options, source);
  assertHeld();
  const existing = present(backupRoot) ? registryAt(backupRoot, false) : null;
  if (existing?.datasets[dataset] && existing.datasets[dataset] !== source) fail('dataset 已绑定另一源目录');
  if (options.dryRun === true) {
    const before = snapshot(source); assertHeld();
    if (encode(snapshot(source)) !== encode(before)) fail('源目录在预检期间变化');
    assertHeld();
    return { dryRun: true, source, backupRoot, dataset, files: before.entries.filter((x) => x.kind === 'file').length, bytes: before.entries.reduce((n, x) => n + (x.size ?? 0), 0), retention: 2, wouldCreate: true, wouldDelete: [] };
  }
  mkdirSync(backupRoot, { recursive: true, mode: 0o700 });
  return withRootLock(backupRoot, () => {
    const reg = registryAt(backupRoot, false) ?? { tool: TOOL, version: VERSION, root: backupRoot, rootId: randomUUID(), datasets: {}, backups: [] };
    if (reg.datasets[dataset] && reg.datasets[dataset] !== source) fail('dataset 已绑定另一源目录');
    reg.datasets[dataset] = source;
    const id = `backup-${randomUUID()}`, path = join(backupRoot, id);
    const record = { id, dataset, source, sequence: Math.max(0, ...reg.backups.map((x) => x.sequence)) + 1, createdAt: new Date().toISOString(), status: 'creating', pinned: options.pinned === true, pendingRecovery: options.pendingRecovery === true };
    mkdirSync(path, { mode: 0o700 });
    atomicJson(join(path, OWNER), { tool: TOOL, version: VERSION, rootId: reg.rootId, id, dataset, source });
    reg.backups.push(record); atomicJson(join(backupRoot, REGISTRY), reg);
    try {
      assertHeld();
      const before = snapshot(source); assertHeld();
      copySnapshot(source, join(path, 'data'), before); assertHeld();
      const after = snapshot(source); assertHeld();
      if (encode(before) !== encode(after)) fail('备份期间源目录发生变化，备份不予验证');
      if (encode(snapshot(join(path, 'data'))) !== encode(before)) fail('副本内容与源目录清单不一致');
      assertHeld();
      const manifest = { tool: TOOL, version: VERSION, dataset, source, snapshot: before };
      atomicJson(join(path, 'manifest.json'), manifest);
      record.manifestSha256 = digest(encode(manifest)); record.status = 'verified'; record.verifiedAt = new Date().toISOString();
      atomicJson(join(path, 'receipt.json'), { ...record, tool: TOOL, version: VERSION });
      atomicJson(join(backupRoot, REGISTRY), reg);
    } catch (error) {
      record.status = 'failed'; record.failure = error.message;
      atomicJson(join(path, 'receipt.json'), { ...record, tool: TOOL, version: VERSION });
      atomicJson(join(backupRoot, REGISTRY), reg);
      throw error;
    }
    const result = verified(backupRoot, reg, record);
    return { ...result, retention: pruneInside(backupRoot, dataset, reg, false) };
  });
}

export function runCli(argv = process.argv.slice(2)) {
  const [command, ...args] = argv;
  if (command === '--help' || command === 'help') return { usage: 'node scripts/verified-backups.mjs create --source ABS --backup-root ABS --dataset NAME --guard-file ABS --guard-token TOKEN [--dry-run] [--pinned] [--pending-recovery]; verify --backup-root ABS --dataset NAME --id ID; prune --backup-root ABS --dataset NAME [--dry-run]', guard: '独占写保护由所有源写入者共同遵守；本工具不创建或推断停写保护。guard JSON: {tool:"cilai-source-write-guard",version:1,source:ABS,exclusive:true,pid:LIVE_PID,token:TOKEN}' };
  const options = {}, names = { '--source': 'source', '--backup-root': 'backupRoot', '--dataset': 'dataset', '--id': 'id', '--guard-file': 'guardFile', '--guard-token': 'guardToken', '--dry-run': 'dryRun', '--pinned': 'pinned', '--pending-recovery': 'pendingRecovery' };
  for (let i = 0; i < args.length; i++) {
    const name = names[args[i]];
    if (!name || Object.hasOwn(options, name)) fail(`未知或重复参数：${args[i]}`);
    if (['dryRun', 'pinned', 'pendingRecovery'].includes(name)) options[name] = true;
    else { const value = args[++i]; if (!value || value.startsWith('--')) fail('参数缺少值'); options[name] = value; }
  }
  if (command === 'create') return createVerifiedBackup(options);
  if (command === 'verify') return verifyBackup(options);
  if (command === 'prune') return pruneBackups(options);
  fail('命令必须为 create、verify 或 prune');
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { console.log(JSON.stringify(runCli(), null, 2)); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
