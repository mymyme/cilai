const fs = require('node:fs');
const path = require('node:path');

const APPLICATION_ID = 'cn.cilai.personal.workbench';
let stagedRoot = path.resolve(__dirname);
while (path.basename(stagedRoot) !== '.personal-build' && path.dirname(stagedRoot) !== stagedRoot) stagedRoot = path.dirname(stagedRoot);
const projectRoot = path.basename(stagedRoot) === '.personal-build' ? path.dirname(stagedRoot) : path.resolve(__dirname, '../../../../..');
const buildRoot = path.join(projectRoot, '.personal-build');

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
  const root = packaged ? path.join(appData, APPLICATION_ID) : path.join(projectRoot, '.personal-run', APPLICATION_ID);
  if (requestedUserData && path.resolve(requestedUserData) !== root) throw new Error('个人版只能使用已登记的独立资料目录。');
  rejectLinkedAncestors(root);
  return { userData: root, sessionData: path.join(root, 'browser'), logs: path.join(root, 'logs'),
    hostArguments: ['--data-dir', path.join(root, 'business')] };
}

function personalBuildRoot(requested) {
  if (requested && path.resolve(requested) !== buildRoot) throw new Error('个人版构建目录必须使用已登记的 .personal-build。');
  rejectLinkedAncestors(buildRoot);
  return buildRoot;
}

function personalServiceEnvironment(source = {}) {
  return Object.fromEntries(Object.entries(source).filter(([key]) => !(
    /^CILAI_/i.test(key) && !['CILAI_BROWSER_MCP_URL', 'CILAI_BROWSER_MCP_TOKEN', 'CILAI_MEMORY_GIT_RUNTIME'].includes(key)
    || /^(CODEX_|OPENAI_|ANTHROPIC_)/i.test(key)
  )));
}

module.exports = { APPLICATION_ID, personalDirectories, personalBuildRoot, personalServiceEnvironment, rejectLinkedAncestors };
