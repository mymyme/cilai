const fs = require('node:fs');
const path = require('node:path');

const APPLICATION_ID = 'cn.cilai.personal.workbench';
const location = require('./personal-build-location');
const sourceRoot = path.resolve(__dirname, '../../..');
const personalProjectRoot = () => location.sourceContext(sourceRoot).projectRoot;

function rejectLinkedAncestors(directory) {
  let current = path.resolve(directory);
  while (true) {
    try {
      if (fs.lstatSync(current).isSymbolicLink()) throw new Error(`个人版目录不能经过符号链接：${current}`);
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
    const parent = path.dirname(current);
    if (parent === current) break;
    current = parent;
  }
}

function personalDirectories({ packaged, appData, requestedUserData = '' }) {
  const root = packaged ? path.join(appData, APPLICATION_ID) : path.join(personalProjectRoot(), '.personal-run', APPLICATION_ID);
  if (requestedUserData && path.resolve(requestedUserData) !== root) throw new Error('个人版只能使用已登记的独立资料目录。');
  rejectLinkedAncestors(root);
  return { userData: root, sessionData: path.join(root, 'browser'), logs: path.join(root, 'logs'),
    hostArguments: ['--data-dir', path.join(root, 'business')] };
}

function personalBuildRoot(requested) {
  const buildRoot = location.personalBuildRoot();
  if (requested && path.resolve(requested) !== buildRoot) throw new Error('个人版构建目录必须使用已登记的本地构建根。');
  rejectLinkedAncestors(buildRoot);
  return buildRoot;
}

function personalServiceEnvironment(source = {}, explicit = {}) {
  const environment = Object.fromEntries(Object.entries(source).filter(([key]) => !(
    /^CILAI_/i.test(key) && !['CILAI_BROWSER_MCP_URL', 'CILAI_BROWSER_MCP_TOKEN', 'CILAI_MEMORY_GIT_RUNTIME'].includes(key)
    || /^(CODEX_|OPENAI_|ANTHROPIC_)/i.test(key)
  )));
  if (Object.hasOwn(explicit, 'CILAI_DATA_ENVIRONMENT')) {
    const selected = explicit.CILAI_DATA_ENVIRONMENT;
    if (selected !== 'dev' && selected !== 'prod') throw new Error('数据环境配置无效，只允许 dev 或 prod。');
    environment.CILAI_DATA_ENVIRONMENT = selected;
  }
  return environment;
}

module.exports = { APPLICATION_ID, personalDirectories, personalBuildRoot, personalProjectRoot, personalServiceEnvironment, rejectLinkedAncestors };
