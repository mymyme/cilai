import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

for (const name of ['verify-skill-creator-harness.mjs', 'verify-persistent-thread-harness.mjs', 'verify-capability-creation-delivery.mjs']) {
  test(`${name} 初始化失败关闭自有服务并保留诊断`, { skip: process.platform === 'win32' }, (t) => {
    const root = mkdtempSync(join(tmpdir(), 'cilai-managed-harness-test-'));
    t.after(() => rmSync(root, { recursive: true, force: true }));
    const fake = join(root, 'fake-service');
    writeFileSync(fake, `#!${process.execPath}\nconst readline = require('node:readline');\nconst lines = readline.createInterface({ input: process.stdin });\nlines.on('line', (line) => { const request = JSON.parse(line); if (request.id) process.stdout.write(JSON.stringify({id:request.id,error:{message:'fixture initialize rejected'}})+'\\n'); });\n`);
    chmodSync(fake, 0o700);
    const url = new URL(name, import.meta.url).href;
    const code = `process.argv[1] = '[import-fixture]'; process.argv[2] = ${JSON.stringify(fake)}; await import(${JSON.stringify(url)});`;
    const result = spawnSync(process.execPath, ['--input-type=module', '-e', code], {
      encoding: 'utf8', timeout: 15000, env: { ...process.env, TMPDIR: root, TMP: root, TEMP: root },
    });
    assert.equal(result.error, undefined, result.error?.message);
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /fixture initialize rejected/);
    const diagnostics = readdirSync(root).filter((entry) => existsSync(join(root, entry, 'failure.json')));
    assert.equal(diagnostics.length, 1, result.stderr);
    const report = JSON.parse(readFileSync(join(root, diagnostics[0], 'failure.json')));
    assert.equal(report.status, 'failed');
    assert.equal(report.children.length, 1);
    assert.deepEqual(report.children[0].exit, { code: 0, signal: null });
  });
}

test('线程协议 Python 入口从受管构建日志提取唯一结果', { skip: process.platform === 'win32' }, () => {
  const file = fileURLToPath(new URL('./verify-codex-thread-contract.py', import.meta.url));
  const code = `import importlib.util, json
spec=importlib.util.spec_from_file_location('contract', ${JSON.stringify(file)})
m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
assert m.parse_harness_report('build log\\n{"persistentResume":true,"rounds":20}\\n构建空间报告：test\\n')['rounds']==20
for value in ['no report', '{"persistentResume":false}', '{"persistentResume":true}\\n{"persistentResume":true}']:
 try: m.parse_harness_report(value)
 except ValueError: pass
 else: raise AssertionError('invalid report accepted')
`;
  const result = spawnSync('python3', ['-B', '-c', code], { encoding: 'utf8', env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' } });
  assert.equal(result.status, 0, result.stderr);
});
