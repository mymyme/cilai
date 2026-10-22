const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

function createHeadlessIsolation({ headless, temporaryDirectory = os.tmpdir() }) {
  if (!headless) return null;
  if (!path.isAbsolute(temporaryDirectory)) throw new Error('自检临时目录必须是绝对路径');
  const parent = fs.realpathSync(temporaryDirectory);
  const root = fs.mkdtempSync(path.join(parent, 'cilai-headless-'));
  fs.chmodSync(root, 0o700);
  const userData = path.join(root, 'shell');
  const sessionData = path.join(root, 'session');
  const logs = path.join(root, 'logs');
  for (const directory of [userData, sessionData, logs]) fs.mkdirSync(directory, { mode: 0o700 });
  return { root, userData, sessionData, logs,
    hostArguments: ['--headless-check', '--data-dir', path.join(root, 'host')] };
}

module.exports = { createHeadlessIsolation };
