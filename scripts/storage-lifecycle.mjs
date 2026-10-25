import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, renameSync, rmSync, statfsSync, writeFileSync } from 'node:fs';
import { basename, dirname, isAbsolute, join, parse, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const schema = 'cilai-build-storage-v1';
const indexSchema = 'cilai-build-storage-index-v1';
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const budget = 20 * 1000 ** 3;
const temporaryVariables = ['TMPDIR', 'TMP', 'TEMP'];
const originalTemporaryEnvironment = 'CILAI_BUILD_ORIGINAL_TEMP_ENV';
const present = (path) => { try { lstatSync(path); return true; } catch (error) { if (error.code === 'ENOENT') return false; throw error; } };
const localDay = (value) => { const date = new Date(value); return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`; };

export function assertManagedPath(root, path) {
  const parent = realpathSync(root), target = resolve(path);
  if (parent === parse(parent).root) throw new Error('空间管理不能使用文件系统根目录');
  const local = relative(parent, target);
  if (!local || isAbsolute(local) || local.startsWith(`..${sep}`) || local === '..' || resolve(parent, local) !== target) throw new Error('空间管理路径越界');
  let cursor = parent;
  for (const part of local.split(sep)) {
    cursor = join(cursor, part);
    if (present(cursor) && lstatSync(cursor).isSymbolicLink()) throw new Error(`空间管理路径不能经过符号链接：${cursor}`);
  }
  return target;
}

export function directoryBytes(path, seen = new Set()) {
  if (!present(path)) return 0;
  const stat = lstatSync(path);
  if (stat.isSymbolicLink()) return 0;
  if (stat.isDirectory()) return readdirSync(path).reduce((sum, name) => sum + directoryBytes(join(path, name), seen), 0);
  if (!stat.isFile()) return 0;
  const id = `${stat.dev}:${stat.ino}`;
  if (stat.ino && seen.has(id)) return 0;
  seen.add(id);
  return stat.size;
}

const inventorySchema = 'cilai-build-storage-inventory-v2';
const legacyInventorySchema = 'cilai-build-storage-inventory-v1';
const legacyInventoryNames = ['target', 'tmp', 'verification', 'cache', 'toolchains', 'node_modules', 'apps/desktop/node_modules'];
const inventoryNames = [...legacyInventoryNames, 'target-runtime-audit'];

export function storageSnapshot(buildRoot, { statfs = statfsSync, measureDirectory = directoryBytes, now = () => new Date() } = {}) {
  const root = realpathSync(buildRoot), at = new Date(now()).toISOString(), disk = statfs(root);
  const freeBytes = disk.bavail * disk.bsize;
  if (!Number.isSafeInteger(freeBytes) || freeBytes < 0) throw new Error('卷可用空间采样无效');
  const cachePath = assertManagedPath(root, join(root, 'target/storage-reports/storage-inventory.json'));
  let inventory;
  if (present(cachePath)) {
    inventory = readOwnedJson(root, cachePath);
    const sizes = inventory.directoryLogicalBytes;
    const recognized = inventory.schema === inventorySchema || inventory.schema === legacyInventorySchema;
    const expectedNames = inventory.schema === legacyInventorySchema ? legacyInventoryNames : inventoryNames;
    if (!recognized || inventory.root !== root || !Number.isFinite(Date.parse(inventory.at)) || !Number.isSafeInteger(inventory.availableBytes) || inventory.availableBytes < 0 || !sizes || typeof sizes !== 'object' || Array.isArray(sizes) || Object.keys(sizes).length !== expectedNames.length || expectedNames.some((name) => !Object.hasOwn(sizes, name) || !Number.isSafeInteger(sizes[name]) || sizes[name] < 0) || !Number.isSafeInteger(inventory.totalLogicalBytes) || inventory.totalLogicalBytes !== Object.values(sizes).reduce((a, b) => a + b, 0)) throw new Error('空间采样缓存归属或结构无效，禁止覆盖未知缓存');
  }
  const reason = !inventory ? 'first-sample' : inventory.schema === legacyInventorySchema ? 'schema-upgrade' : localDay(inventory.at) !== localDay(at) ? 'date-changed' : inventory.availableBytes - freeBytes >= budget ? 'available-space-drop' : 'cached';
  const fresh = reason !== 'cached';
  if (fresh) {
    const sizes = {};
    for (const name of inventoryNames) {
      const path = join(root, name);
      if (dirname(path) !== root) assertManagedPath(root, dirname(path));
      if (present(path) && lstatSync(path).isSymbolicLink()) { sizes[name] = 0; continue; }
      assertManagedPath(root, path);
      const bytes = measureDirectory(path);
      if (!Number.isSafeInteger(bytes) || bytes < 0) throw new Error(`目录空间采样无效：${name}`);
      sizes[name] = bytes;
    }
    inventory = { schema: inventorySchema, root, at, availableBytes: freeBytes, directoryLogicalBytes: sizes, totalLogicalBytes: Object.values(sizes).reduce((a, b) => a + b, 0) };
    mkdirSync(dirname(cachePath), { recursive: true });
    save(root, cachePath, inventory);
  }
  return { at, freeBytes, directoryLogicalBytes: inventory.directoryLogicalBytes, totalLogicalBytes: inventory.totalLogicalBytes,
    inventoryFresh: fresh, inventoryAt: inventory.at, inventoryReason: reason, inventoryAvailableBytes: inventory.availableBytes };
}

export function openProcesses(path, inspect = spawnSync, platform = process.platform) {
  if (platform === 'darwin' || platform === 'linux') {
    const result = inspect('lsof', ['-nP', '-t', '+D', path], { encoding: 'utf8', maxBuffer: 8 * 1024 ** 2 });
    if (result.error || ![0, 1].includes(result.status) || result.stderr?.trim()) throw new Error(`无法确认临时目录无进程占用：${result.error?.message ?? result.stderr ?? result.status}`);
    const text = String(result.stdout ?? '').trim();
    if (text && !/^\d+(\s+\d+)*$/.test(text)) throw new Error('进程检查输出无效');
    const pids = [...new Set(text.split(/\s+/).filter(Boolean).map(Number))];
    if (pids.some((pid) => !Number.isSafeInteger(pid) || pid < 1)) throw new Error('进程检查输出包含无效进程号');
    return pids;
  }
  if (platform === 'win32') throw new Error('Windows 尚无完整的打开文件句柄检查，临时目录需要人工核实');
  throw new Error(`空间管理不支持平台：${platform}`);
}

function readOwnedJson(root, path) {
  assertManagedPath(root, path);
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 1024 ** 2) throw new Error('空间管理元数据不是受控普通文件');
  return JSON.parse(readFileSync(path, 'utf8'));
}
function save(root, path, value) {
  assertManagedPath(root, path);
  if (present(path) && !lstatSync(path).isFile()) throw new Error('空间管理报告路径不是普通文件');
  const pending = assertManagedPath(root, `${path}.${randomUUID()}.pending`);
  writeFileSync(pending, JSON.stringify(value, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
  renameSync(pending, path);
}
function reportIndex(root, { create = false } = {}) {
  const path = assertManagedPath(root, join(root, 'target/storage-reports/index.json'));
  if (!present(path)) {
    if (!create) throw new Error('空间报告登记索引不存在');
    const index = { schema: indexSchema, root, ids: [], dailyBaselines: {} };
    save(root, path, index); return index;
  }
  const index = readOwnedJson(root, path);
  if (index.schema !== indexSchema || index.root !== root || !Array.isArray(index.ids) || index.ids.some((id) => !uuid.test(id)) || new Set(index.ids).size !== index.ids.length || !index.dailyBaselines || typeof index.dailyBaselines !== 'object' || Array.isArray(index.dailyBaselines) || Object.entries(index.dailyBaselines).some(([day, bytes]) => !/^\d{4}-\d{2}-\d{2}$/.test(day) || !Number.isFinite(bytes) || bytes < 0)) throw new Error('空间报告登记索引归属无效');
  return index;
}
function ownReport(root, id) {
  const path = join(root, 'target/storage-reports', `build-${id}.json`);
  const item = readOwnedJson(root, path);
  if (item.schema !== schema || item.root !== root || item.id !== id || item.report !== path || !Number.isFinite(Date.parse(item.startedAt)) || !Number.isFinite(item.before?.totalLogicalBytes)) throw new Error('空间报告归属或结构无效');
  return { path, item };
}
function assertRunOwner(state) {
  const { root, run, report, temporary } = state;
  if (!uuid.test(state.id) || realpathSync(root) !== root || run !== join(root, 'tmp/storage-runs', state.id) || temporary !== join(run, 'tmp') || report !== join(root, 'target/storage-reports', `build-${state.id}.json`)) throw new Error('本轮空间登记无效');
  assertManagedPath(root, run); assertManagedPath(root, report); assertManagedPath(root, temporary);
  const owner = readOwnedJson(root, join(run, 'owner.json'));
  if (owner.schema !== schema || owner.id !== state.id || owner.root !== root || owner.pid !== process.pid || owner.startedAt !== state.startedAt) throw new Error('临时目录归属校验失败');
  if (!reportIndex(root).ids.includes(state.id)) throw new Error('本轮报告未登记');
  ownReport(root, state.id);
  if (readdirSync(run).some((name) => !['owner.json', 'tmp'].includes(name))) throw new Error('本轮目录包含未登记内容，禁止自动清理');
}

export function beginStorageRun(buildRoot, command, { measure = storageSnapshot } = {}) {
  const root = realpathSync(buildRoot), id = randomUUID();
  const runs = assertManagedPath(root, join(root, 'tmp/storage-runs'));
  const reports = assertManagedPath(root, join(root, 'target/storage-reports'));
  mkdirSync(runs, { recursive: true }); mkdirSync(reports, { recursive: true });
  const index = reportIndex(root, { create: true });
  const run = join(runs, id), temporary = join(run, 'tmp');
  mkdirSync(run, { mode: 0o700 }); mkdirSync(temporary, { mode: 0o700 });
  const marker = { schema, id, root, pid: process.pid, startedAt: new Date().toISOString() };
  save(root, join(run, 'owner.json'), marker);
  const report = join(reports, `build-${id}.json`);
  const state = { ...marker, day: localDay(marker.startedAt), command: basename(command), run, temporary, report, before: measure(root), status: 'running' };
  save(root, report, state);
  index.ids.push(id);
  if (!Object.hasOwn(index.dailyBaselines, state.day)) index.dailyBaselines[state.day] = state.before.totalLogicalBytes;
  save(root, join(reports, 'index.json'), index);
  return state;
}

export function storageEnvironment(state, environment) {
  const original = launchEnvironment(environment);
  const saved = Object.fromEntries(temporaryVariables.map((name) => [name, original[name] ?? null]));
  return { ...environment, [originalTemporaryEnvironment]: JSON.stringify(saved), CILAI_BUILD_RUN_TMP: state.temporary, TMPDIR: state.temporary, TMP: state.temporary, TEMP: state.temporary,
    CILAI_BUILD_ROOT: state.root, CARGO_TARGET_DIR: join(state.root, 'target'), CARGO_INCREMENTAL: '0' };
}

export function launchEnvironment(environment) {
  const result = { ...environment }, encoded = environment[originalTemporaryEnvironment];
  delete result.CILAI_BUILD_LOCK_TOKEN;
  if (encoded === undefined) {
    if (environment.CILAI_BUILD_RUN_TMP) throw new Error('缺少构建前临时环境记录，不能启动常驻应用');
    return result;
  }
  let saved;
  try { saved = JSON.parse(encoded); } catch { throw new Error('构建前临时环境记录无效'); }
  if (!saved || typeof saved !== 'object' || Array.isArray(saved) || Object.keys(saved).length !== temporaryVariables.length || temporaryVariables.some((name) => !Object.hasOwn(saved, name) || (saved[name] !== null && (typeof saved[name] !== 'string' || saved[name].includes('\0'))))) throw new Error('构建前临时环境记录无效');
  for (const name of temporaryVariables) {
    if (saved[name] === null) delete result[name];
    else result[name] = saved[name];
  }
  delete result[originalTemporaryEnvironment];
  delete result.CILAI_BUILD_RUN_TMP;
  return result;
}

export function launchPackagedMacApp(app, environment, run = spawnSync) {
  if (!isAbsolute(app) || !app.endsWith('.app')) throw new Error('启动需要应用包绝对路径');
  const result = run('/usr/bin/open', [app], { env: launchEnvironment(environment), stdio: 'inherit' });
  if (result.error || result.status !== 0) throw new Error(`无法打开应用：${result.error?.message ?? result.status}`);
}

export function finishStorageRun(state, status, { measure = storageSnapshot, inspect = openProcesses, measureDirectory = directoryBytes } = {}) {
  assertRunOwner(state);
  const { root, run, report } = state;
  state.status = status === 0 ? 'passed' : 'failed';
  state.completedAt = new Date().toISOString();
  state.elapsedSeconds = (Date.now() - Date.parse(state.startedAt)) / 1000;
  state.exitCode = status;
  state.cleanup = status === 0 ? 'checking' : 'retained-failure';
  state.warnings = [];
  save(root, report, state);
  try { state.temporaryBytes = measureDirectory(state.temporary); }
  catch (error) { state.cleanup = 'needs-review'; state.cleanupError = error.message; save(root, report, state); throw error; }
  save(root, report, state);
  if (status === 0) {
    let active;
    try {
      active = inspect(run);
      if (!Array.isArray(active) || active.some((pid) => !Number.isSafeInteger(pid) || pid < 1)) throw new Error('进程检查未提供有效占用结果');
    } catch (error) {
      state.cleanup = 'needs-review'; state.cleanupError = error.message;
      state.warnings.push(`未能完成占用检查，保留临时目录：${error.message}`);
    }
    if (active?.length) { state.cleanup = 'retained-active'; state.activePids = active; }
    else if (active) {
      assertRunOwner(state);
      state.cleanup = 'approved-for-removal'; save(root, report, state);
      rmSync(run, { recursive: true }); state.cleanup = 'removed';
    }
    save(root, report, state);
  }
  try { state.after = measure(root); }
  catch (error) { state.storageStatus = 'needs-review'; state.measurementError = error.message; save(root, report, state); throw error; }
  const distinctMeasurements = state.before.inventoryFresh !== false && state.after.inventoryFresh !== false && (!state.before.inventoryAt || !state.after.inventoryAt || state.before.inventoryAt !== state.after.inventoryAt);
  state.directoryGrowthBytes = distinctMeasurements ? state.after.totalLogicalBytes - state.before.totalLogicalBytes : null;
  state.directoryGrowthBasis = distinctMeasurements ? 'measured-before-after' : 'inventory-reused';
  state.directoryGrowthNote = distinctMeasurements ? '本轮前后分别完成目录扫描' : '复用最近一次完整目录采样，本轮准确目录增量未测量';
  state.diskFreeChangeBytes = state.after.freeBytes - state.before.freeBytes;
  const index = reportIndex(root);
  const previous = index.ids.filter((id) => id !== state.id).map((id) => ownReport(root, id)).sort((a, b) => b.item.startedAt.localeCompare(a.item.startedAt));
  if (!Object.hasOwn(index.dailyBaselines, state.day)) throw new Error('本轮每日空间基线未登记');
  state.dailyGrowthBytes = state.after.totalLogicalBytes - index.dailyBaselines[state.day];
  state.dailyGrowthInventoryAt = state.after.inventoryAt ?? state.after.at;
  state.dailyGrowthFresh = state.after.inventoryFresh !== false;
  if (measureDirectory(join(root, 'tmp/storage-runs')) > budget) state.warnings.push('登记的临时数据超过 20 GB');
  try {
    const fixturePath = assertManagedPath(root, join(root, 'target-runtime-audit/fixtures'));
    state.runtimeAuditFixtureBytes = measureDirectory(fixturePath);
    if (!Number.isSafeInteger(state.runtimeAuditFixtureBytes) || state.runtimeAuditFixtureBytes < 0) throw new Error('运行时验证夹具空间采样无效');
    state.runtimeAuditFixtureMeasuredAt = new Date().toISOString();
  } catch (error) { state.storageStatus = 'needs-review'; state.measurementError = error.message; save(root, report, state); throw error; }
  if (state.runtimeAuditFixtureBytes > budget) state.warnings.push('运行时验证夹具超过 20 GB，需核对活动任务与失败证据');
  if (state.dailyGrowthBytes > budget) state.warnings.push(state.dailyGrowthFresh ? '今日构建目录增长超过 20 GB' : `最近完整采样显示今日构建目录增长超过 20 GB（${state.dailyGrowthInventoryAt}）`);
  if (state.after.freeBytes < 100 * 1000 ** 3) state.warnings.push('磁盘可用空间低于 100 GB');
  save(root, report, state);
  const currentPassed = state.status === 'passed' && state.cleanup === 'removed';
  const obsolete = previous.filter(({ item }) => item.status === 'passed' && item.cleanup === 'removed').slice(currentPassed ? 29 : 30);
  for (const old of obsolete) {
    ownReport(root, old.item.id);
    index.ids = index.ids.filter((id) => id !== old.item.id);
    save(root, join(root, 'target/storage-reports/index.json'), index);
    rmSync(old.path);
  }
  for (const warning of state.warnings) console.warn(`空间提醒：${warning}；请检查 ${report}`);
  console.log(`构建空间报告：${report}`);
  return state;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [mode, app, ...extra] = process.argv.slice(2);
    if (mode === '--launch-open' && app && extra.length === 0) launchPackagedMacApp(app, process.env);
    else if (mode === '--launch-temp-environment-json' && app === undefined) {
      const environment = launchEnvironment(process.env);
      process.stdout.write(JSON.stringify(Object.fromEntries(temporaryVariables.map((name) => [name, environment[name] ?? null]))));
    } else throw new Error('空间管理启动环境参数无效');
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
