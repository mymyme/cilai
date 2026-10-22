import { mkdirSync } from 'node:fs';
import { isAbsolute, join } from 'node:path';

const inheritedNames = new Set([
  'PATH', 'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'PATHEXT', 'SYSTEMDRIVE',
  'PROCESSOR_ARCHITECTURE', 'NUMBER_OF_PROCESSORS', 'LANG', 'LC_ALL', 'TZ',
]);

export function verificationEnvironment(root, overrides = {}) {
  if (!isAbsolute(root)) throw new Error('验收目录必须使用独立绝对路径');
  const home = join(root, 'isolated-home');
  const temporary = join(root, 'isolated-tmp');
  const codexHome = join(root, 'isolated-codex');
  const localAppData = join(home, 'AppData', 'Local');
  const roamingAppData = join(home, 'AppData', 'Roaming');
  for (const directory of [home, temporary, codexHome, localAppData, roamingAppData]) {
    mkdirSync(directory, { recursive: true, mode: 0o700 });
  }
  const environment = Object.fromEntries(Object.entries(process.env)
    .filter(([name]) => inheritedNames.has(name.toUpperCase())));
  return {
    ...environment,
    HOME: home, USERPROFILE: home, LOCALAPPDATA: localAppData, APPDATA: roamingAppData,
    TMPDIR: temporary, TEMP: temporary, TMP: temporary,
    CODEX_HOME: codexHome, CODEX_SQLITE_HOME: codexHome,
    ...overrides,
  };
}
