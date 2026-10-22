import assert from 'node:assert/strict';
import { test } from 'node:test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

test('验收子进程只接收独立资料和明确传入的本机探测配置', () => {
  const root = mkdtempSync(join(tmpdir(), 'cilai-personal-verification-'));
  try {
    const moduleUrl = new URL('./verification-environment.mjs', import.meta.url).href;
    const received = JSON.parse(execFileSync(process.execPath, ['--input-type=module', '-e', `
      import { verificationEnvironment } from ${JSON.stringify(moduleUrl)};
      import { execFileSync } from 'node:child_process';
      const environment = verificationEnvironment(${JSON.stringify(root)}, { LOCAL_PROBE: 'explicit' });
      process.stdout.write(execFileSync(process.execPath, ['-e', 'process.stdout.write(JSON.stringify(process.env))'], { env: environment }));
    `], {
      encoding: 'utf8',
      env: {
        ...process.env,
        OPENAI_API_KEY: 'credential-must-not-be-inherited',
        CILAI_ENTERPRISE_ORIGIN: 'http://enterprise.invalid',
        CILAI_WORKBENCH_ACCESS_TOKEN: 'enterprise-token-must-not-be-inherited',
        DATABASE_URL: 'postgres://enterprise.invalid/test',
        PGHOST: 'enterprise.invalid',
        CODEX_HOME: join(root, 'unrelated-codex'),
      },
    }));
    for (const name of ['OPENAI_API_KEY', 'CILAI_ENTERPRISE_ORIGIN', 'CILAI_WORKBENCH_ACCESS_TOKEN', 'DATABASE_URL', 'PGHOST']) {
      assert.equal(received[name], undefined);
    }
    assert.equal(received.HOME, join(root, 'isolated-home'));
    assert.equal(received.CODEX_HOME, join(root, 'isolated-codex'));
    assert.equal(received.CODEX_SQLITE_HOME, received.CODEX_HOME);
    assert.equal(received.LOCAL_PROBE, 'explicit');
    assert.equal(received.LOCALAPPDATA, join(root, 'isolated-home', 'AppData', 'Local'));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
