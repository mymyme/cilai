const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');

const APPLICATION_ID = 'cn.cilai.personal.workbench';
const registrationName = '.cilai-personal-build-registration.json';
const cloudPath = /(?:^|[\\/])(?:Mobile Documents|CloudStorage|OneDrive(?: - [^\\/]+)?|Dropbox(?: \([^\\/]+\))?|Google Drive|GoogleDrive|iCloud Drive)(?:[\\/]|$)/i;

function rejectLinkedAncestors(directory) {
  let current = path.resolve(directory);
  while (true) {
    try {
      const info = fs.lstatSync(current);
      if (info.isSymbolicLink()) throw new Error(`个人工程路径不能经过符号链接：${current}`);
    } catch (error) { if (error.code !== 'ENOENT') throw error; }
    const parent = path.dirname(current);
    if (parent === current) break;
    current = parent;
  }
  return path.resolve(directory);
}

function personalBuildRoot({ platform = process.platform, home = os.homedir() } = {}) {
  const paths = platform === 'win32' ? path.win32 : platform === 'darwin' ? path.posix : null;
  if (!paths) throw new Error(`个人桌面构建尚未支持 ${platform}`);
  if (typeof home !== 'string' || !paths.isAbsolute(home)) throw new Error('个人构建根需要明确的用户主目录');
  const root = paths.resolve(home, '.cilai-personal-builds', 'desktop');
  if (cloudPath.test(root)) throw new Error(`个人构建根必须位于本地非云同步目录：${root}`);
  return root;
}

function sourceIdentity(sourceRoot) {
  rejectLinkedAncestors(sourceRoot);
  const absolute = path.resolve(sourceRoot);
  if (path.basename(absolute) !== 'cilai-workbench' || path.basename(path.dirname(absolute)) !== '06代码索引') {
    throw new Error('个人源码必须具有明确的 06代码索引/cilai-workbench 布局');
  }
  const configuration = fs.readFileSync(path.join(absolute, 'apps/desktop/electron-builder.yml'), 'utf8');
  if (!/^appId: cn\.cilai\.personal\.workbench\s*$/m.test(configuration)) throw new Error('源码不是已登记的个人版工程');
  return { sourceRoot: absolute, projectRoot: path.resolve(absolute, '../..') };
}

function validateRegistration(value, buildRoot) {
  if (!value || value.schemaVersion !== 1 || value.applicationId !== APPLICATION_ID || value.buildRoot !== buildRoot
    || typeof value.sourceRoot !== 'string' || typeof value.projectRoot !== 'string') throw new Error('个人构建登记无效');
  const identity = sourceIdentity(value.sourceRoot);
  if (identity.projectRoot !== value.projectRoot || identity.sourceRoot !== value.sourceRoot) throw new Error('个人构建登记的源码或工程路径不一致');
  return value;
}

function readRegistration(buildRoot) {
  rejectLinkedAncestors(buildRoot);
  const file = path.join(buildRoot, registrationName);
  rejectLinkedAncestors(file);
  let metadata;
  try { metadata = fs.lstatSync(file); }
  catch (error) {
    if (error.code === 'ENOENT') throw new Error(`个人暂存构建根尚未登记，请从个人源码执行标准暂存：${buildRoot}`);
    throw error;
  }
  if (!metadata.isFile()) throw new Error('个人构建登记必须是普通文件');
  return validateRegistration(JSON.parse(fs.readFileSync(file, 'utf8')), buildRoot);
}

function sourceContext(moduleRoot, options = {}) {
  const buildRoot = personalBuildRoot(options);
  const root = path.resolve(moduleRoot);
  rejectLinkedAncestors(root);
  if (root === buildRoot || root === path.join(buildRoot, 'runtime-source')) {
    const registration = readRegistration(buildRoot);
    return { buildRoot, sourceRoot: registration.sourceRoot, projectRoot: registration.projectRoot };
  }
  const identity = sourceIdentity(root);
  const file = path.join(buildRoot, registrationName);
  if (fs.existsSync(file)) {
    const registration = readRegistration(buildRoot);
    if (registration.sourceRoot !== identity.sourceRoot || registration.projectRoot !== identity.projectRoot) throw new Error('当前源码与个人构建根登记不一致');
  }
  return { buildRoot, ...identity };
}

function registerPersonalBuild(moduleRoot, options = {}) {
  const context = sourceContext(moduleRoot, options);
  rejectLinkedAncestors(context.buildRoot);
  fs.mkdirSync(context.buildRoot, { recursive: true });
  const file = path.join(context.buildRoot, registrationName);
  if (fs.existsSync(file)) return readRegistration(context.buildRoot);
  const value = { schemaVersion: 1, applicationId: APPLICATION_ID, ...context };
  const pending = path.join(context.buildRoot, `${registrationName}.${crypto.randomUUID()}.tmp`);
  try {
    fs.writeFileSync(pending, `${JSON.stringify(value, null, 2)}\n`, { flag: 'wx', mode: 0o600 });
    fs.linkSync(pending, file);
  } finally { if (fs.existsSync(pending)) fs.unlinkSync(pending); }
  return readRegistration(context.buildRoot);
}

module.exports = { APPLICATION_ID, registrationName, personalBuildRoot, rejectLinkedAncestors, sourceContext, readRegistration, registerPersonalBuild };
