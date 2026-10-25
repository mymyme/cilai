import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { access, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { createVerificationFixture } from './verification-fixture.mjs';

test('成功后只关闭自己创建的服务并回收本轮目录', async () => {
  const f = await createVerificationFixture('cilai-fixture-success-', { stopTimeoutMs: 1000, inspect: () => [] });
  const child = f.track(spawn(process.execPath, ['-e', 'process.stdin.resume(); process.stdin.on("end",()=>process.exit(0));'], { stdio: ['pipe', 'pipe', 'pipe'] }));
  await f.finish();
  assert.equal(child.exitCode, 0);
  await assert.rejects(access(f.root), /ENOENT/);
  await assert.rejects(async () => f.stop({}), /当前验收/);
});
test('初始化失败仍关闭已创建服务，保留诊断和现场', async () => {
  const f = await createVerificationFixture('cilai-fixture-failure-', { stopTimeoutMs: 1000, inspect: () => [] });
  const child = f.track(spawn(process.execPath, ['-e', 'process.stdin.resume(); process.stdin.on("end",()=>process.exit(0));'], { stdio: ['pipe', 'pipe', 'pipe'] }));
  await assert.rejects(f.finish(new Error('initialize rejected')), /initialize rejected/);
  assert.equal(child.exitCode, 0);
  const report = JSON.parse(await readFile(join(f.root, 'failure.json')));
  assert.equal(report.status, 'failed');
  assert.equal(report.children[0].pid, child.pid);
  await rm(f.root, { recursive: true });
});
test('异常退出与无法启动保留诊断，不删除本轮现场', async () => {
  for (const mode of ['exit', 'missing']) {
    const f = await createVerificationFixture('cilai-fixture-abnormal-', { stopTimeoutMs: 1000, inspect: () => [] });
    f.track(spawn(mode === 'exit' ? process.execPath : join(f.root, 'missing'), mode === 'exit' ? ['-e', 'process.exit(9)'] : [], { stdio: ['pipe', 'pipe', 'pipe'] }));
    await assert.rejects(f.finish(), /异常退出|ENOENT/);
    await access(join(f.root, 'failure.json'));
    await rm(f.root, { recursive: true });
  }
});
test('超时只终止本轮服务并保留失败现场', async () => {
  const f = await createVerificationFixture('cilai-fixture-timeout-', { stopTimeoutMs: 100, inspect: () => [] });
  const child = f.track(spawn(process.execPath, ['-e', 'process.stdin.resume(); setInterval(()=>{},100);'], { stdio: ['pipe', 'pipe', 'pipe'] }));
  await assert.rejects(f.finish(), /未正常退出/);
  assert.ok(child.signalCode);
  await access(join(f.root, 'failure.json'));
  await rm(f.root, { recursive: true });
});

test('仍有子孙进程占用时保留现场，不终止未跟踪进程', async () => {
  const f = await createVerificationFixture('cilai-fixture-active-', { inspect: () => [777777] });
  await assert.rejects(f.finish(), /仍有进程占用/);
  await access(join(f.root, 'failure.json'));
  await rm(f.root, { recursive: true });
});
test('不能完成完整句柄检查时明确保留待核实现场', async () => {
  const f = await createVerificationFixture('cilai-fixture-inspection-', { inspect: () => { throw new Error('inspection unavailable'); } });
  assert.equal((await f.finish()).cleanup, 'needs-review');
  await access(join(f.root, 'cleanup-needs-review.json'));
  await rm(f.root, { recursive: true });
});
