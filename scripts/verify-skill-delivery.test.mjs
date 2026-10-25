import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, writeFile, readFile, rm, access, lstat } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';

const verifier = new URL('./verify-skill-delivery.mjs', import.meta.url).href;
const skill = { name: '测试技能', description: '说明', instructions: '执行指令', files: [{ path: 'readme.txt', content: 'resource' }] };

async function fixture(t, mode) {
  const base = await mkdtemp(join(tmpdir(), 'cilai-skill-delivery-test-'));
  t.after(() => rm(base, { recursive: true, force: true }));
  const responsePath = join(base, 'response.md'), reportPath = join(base, 'report.json');
  const response = '```cilai-skill\n' + JSON.stringify(skill) + '\n```\n';
  await writeFile(responsePath, response);
  await writeFile(reportPath, 'previous-report');
  const eventPath = join(base, 'events.jsonl'), fake = join(base, 'fake-service.cjs');
  await writeFile(fake, `
const fs = require('node:fs');
const path = require('node:path');
const readline = require('node:readline');
const mode = ${JSON.stringify(mode)};
const root = process.argv[process.argv.indexOf('--data-dir') + 1];
const record = value => fs.appendFileSync(${JSON.stringify(eventPath)}, JSON.stringify({ ...value, pid: process.pid }) + '\\n');
record({ state: 'started', root });
const skill = { ...${JSON.stringify(skill)}, skillId: 'saved', source: 'user' };
skill.files = skill.files.map(file => ({ ...file, sizeBytes: Buffer.byteLength(file.content) }));
if (mode === 'wrong-skill') skill.name = 'unexpected';
const lines = readline.createInterface({ input: process.stdin });
lines.on('close', () => {
  record({ state: 'closed' });
  if (mode === 'shutdown-timeout') setInterval(() => {}, 1000);
});
lines.on('line', line => {
  const message = JSON.parse(line);
  if (message.id === undefined) return;
  if (mode === 'invalid-json') { process.stdout.write('invalid json\\n'); return; }
  if (mode === 'rpc-timeout') return;
  if (mode === 'report-inside' && message.method === 'initialize' && !fs.lstatSync(${JSON.stringify(reportPath)}).isSymbolicLink()) {
    const inside = path.join(root, 'report.json');
    fs.writeFileSync(inside, 'retained-inside-report');
    fs.unlinkSync(${JSON.stringify(reportPath)});
    fs.symlinkSync(inside, ${JSON.stringify(reportPath)});
  }
  const results = {
    initialize: {}, 'aiState/get': { stateRevision: 0 }, 'aiState/replace': {},
    'skill/runList': { installations: [{ status: 'installed', skillId: 'saved' }] },
    'skill/get': { skill }, 'skill/list': { skills: [skill] }, 'skill/runRetry': { skillId: 'saved' },
  };
  process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: message.id, result: results[message.method] }) + '\\n');
  if (mode === 'late-invalid-json' && message.method === 'skill/list') process.stdout.write('invalid late json\\n');
});
`);
  const options = { executable: mode === 'spawn-failure' ? join(base, 'does-not-exist') : process.execPath,
    servicePrefixArgs: mode === 'spawn-failure' ? [] : [fake], responsePath, reportPath, requestTimeoutMs: 500, stopTimeoutMs: 300 };
  const program = `import { verifySkillDelivery } from ${JSON.stringify(verifier)};
    try { await verifySkillDelivery(${JSON.stringify(options)}); }
    catch (error) { console.error(JSON.stringify({ message: error.message, diagnosticDirectory: error.diagnosticDirectory })); process.exitCode = 1; }`;
  const result = await new Promise(resolveResult => {
    execFile(process.execPath, ['--input-type=module', '-e', program], { encoding: 'utf8', timeout: 7000 }, (error, stdout, stderr) => resolveResult({ error, stdout, stderr }));
  });
  const failure = result.stderr.split('\n').filter(line => line.startsWith('{')).map(line => JSON.parse(line)).at(-1);
  if (failure?.diagnosticDirectory) t.after(() => rm(failure.diagnosticDirectory, { recursive: true, force: true }));
  const events = await readFile(eventPath, 'utf8').then(text => text.trim().split('\n').map(line => JSON.parse(line)), error => {
    if (error.code === 'ENOENT') return [];
    throw error;
  });
  assert.equal(await readFile(responsePath, 'utf8'), response);
  return { ...result, failure, events, reportPath };
}

test('成功报告落盘且两次服务正常退出后清除本轮临时目录', async t => {
  const f = await fixture(t, 'success');
  assert.equal(f.error, null, f.stderr);
  const report = JSON.parse(await readFile(f.reportPath, 'utf8'));
  assert.equal(report.status, 'passed');
  assert.equal(f.events.filter(event => event.state === 'started').length, 2);
  assert.equal(f.events.filter(event => event.state === 'closed').length, 2);
  await assert.rejects(access(dirname(report.isolatedDataDirectory)), { code: 'ENOENT' });
  assert.equal(JSON.parse(f.stdout.trim()).temporaryDataRemoved, true);
});

for (const [mode, message] of [
  ['wrong-skill', /Expected values/],
  ['invalid-json', /不是有效 JSON/],
  ['late-invalid-json', /不是有效 JSON/],
  ['rpc-timeout', /请求超时/],
  ['spawn-failure', /服务启动失败/],
  ['shutdown-timeout', /未在期限内正常退出/],
]) {
  test(`${mode} 有界退出并保留诊断和原始报告`, async t => {
    const f = await fixture(t, mode);
    assert.equal(f.error?.code, 1, f.stderr);
    assert.equal(f.error?.killed, false, f.stderr);
    assert.match(f.failure.message, message);
    const diagnostic = JSON.parse(await readFile(join(f.failure.diagnosticDirectory, 'failure.json'), 'utf8'));
    assert.equal(diagnostic.status, 'failed');
    assert.equal(await readFile(f.reportPath, 'utf8'), 'previous-report');
    for (const service of diagnostic.services) {
      if (service.pid) assert.ok(service.exit, '本轮服务必须已退出');
    }
  });
}

test('拒绝经符号链接写入本轮临时目录的报告，保留诊断', { skip: process.platform === 'win32' }, async t => {
  const f = await fixture(t, 'report-inside');
  assert.equal(f.error?.code, 1, f.stderr);
  assert.match(f.failure.message, /临时目录之外/);
  assert.equal((await lstat(f.reportPath)).isSymbolicLink(), true);
  assert.equal(await readFile(f.reportPath, 'utf8'), 'retained-inside-report');
  await access(join(f.failure.diagnosticDirectory, 'failure.json'));
});
