import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const commandFlags = new Set(['delegate', 'parallel', 'data-exclusive', 'data-observation', 'context-change', 'update-read']);
const methodFlags = new Set(['parallel', 'update-read']);

export function validateHostCommands(value) {
  if (!value || value.schemaVersion !== 1 || Object.keys(value).sort().join(',') !== 'appServerMethods,commands,schemaVersion') {
    throw new Error('宿主命令声明版本或字段无效');
  }
  for (const [name, allowed, pattern] of [
    ['commands', commandFlags, /^[a-z][a-z0-9_]*$/],
    ['appServerMethods', methodFlags, /^[a-zA-Z][a-zA-Z0-9]*\/[a-zA-Z][a-zA-Z0-9]*$/],
  ]) {
    const records = value[name];
    if (!records || typeof records !== 'object' || Array.isArray(records) || !Object.keys(records).length) throw new Error(`宿主命令声明缺少 ${name}`);
    for (const [command, flags] of Object.entries(records)) {
      if (!pattern.test(command) || !Array.isArray(flags) || new Set(flags).size !== flags.length || flags.some(flag => !allowed.has(flag))) {
        throw new Error(`宿主命令声明无效：${command}`);
      }
      if ((flags.includes('data-exclusive') && flags.includes('data-observation'))
          || (flags.includes('parallel') && flags.includes('context-change'))
          || (flags.includes('update-read') && (flags.includes('data-exclusive') || flags.includes('context-change')))
          || (command.endsWith('_internal') && flags.includes('delegate'))) throw new Error(`宿主命令策略互斥：${command}`);
    }
  }
  return value;
}

const tagged = (records, flag) => Object.entries(records).filter(([, flags]) => flags.includes(flag))
  .map(([name]) => name).sort((a, b) => a.localeCompare(b, 'en'));

function rustPredicate(name, values) {
  const predicate = values.length > 1
    ? `matches!(\n        value,\n${values.map((value, index) => `${index ? '            | ' : '        '}${JSON.stringify(value)}`).join('\n')}\n    )`
    : values.length ? `matches!(value, ${JSON.stringify(values[0])})` : 'false';
  return `pub(super) fn ${name}(value: &str) -> bool {\n    ${predicate}\n}\n`;
}

export function hostCommandOutputs(declaration) {
  const { commands, appServerMethods } = validateHostCommands(declaration);
  const javascript = `// Generated from apps/desktop/host-commands.json by scripts/generate-host-commands.mjs.\n`
    + `const DELEGATED_COMMANDS = new Set(${JSON.stringify(tagged(commands, 'delegate'), null, 2)});\n`
    + `const UPDATE_READ_COMMANDS = new Set(${JSON.stringify(tagged(commands, 'update-read'), null, 2)});\n`
    + `const UPDATE_READ_METHODS = new Set(${JSON.stringify(tagged(appServerMethods, 'update-read'), null, 2)});\n\n`
    + `function isDelegatedCommand(command) { return DELEGATED_COMMANDS.has(command); }\n`
    + `function isUpdateRead(command, method) {\n  return UPDATE_READ_COMMANDS.has(command) || (command === 'app_server_request' && UPDATE_READ_METHODS.has(method));\n}\n\n`
    + `module.exports = { isDelegatedCommand, isUpdateRead };\n`;
  const rust = '// Generated from apps/desktop/host-commands.json by scripts/generate-host-commands.mjs.\n\n'
    + [
      ['is_parallel_command', commands, 'parallel'],
      ['is_parallel_app_server_method', appServerMethods, 'parallel'],
      ['requires_exclusive_data', commands, 'data-exclusive'],
      ['is_data_observation', commands, 'data-observation'],
      ['changes_local_context', commands, 'context-change'],
    ].map(([name, records, flag]) => rustPredicate(name, tagged(records, flag))).join('\n');
  return new Map([
    ['apps/desktop/shell/host-commands.generated.js', javascript],
    ['apps/desktop/host-service/src/host_commands_generated.rs', rust],
  ]);
}

export function checkHostCommands(projectRoot = root, { write = false } = {}) {
  const declaration = JSON.parse(readFileSync(join(projectRoot, 'apps/desktop/host-commands.json'), 'utf8'));
  for (const [name, content] of hostCommandOutputs(declaration)) {
    const file = join(projectRoot, name);
    if (write) writeFileSync(file, content);
    else if (readFileSync(file, 'utf8').replaceAll('\r\n', '\n') !== content) {
      throw new Error(`宿主命令生成文件不同步：${name}；请执行 node scripts/generate-host-commands.mjs --write`);
    }
  }
  return declaration;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args.some(arg => arg !== '--write') || args.length > 1) throw new Error('宿主命令生成参数无效');
  checkHostCommands(root, { write: args.includes('--write') });
}
