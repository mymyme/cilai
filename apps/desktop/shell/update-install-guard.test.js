const test = require('node:test');
const assert = require('node:assert/strict');
const { holdServiceForUpdate } = require('./update-install-guard');
const { HostService } = require('./service');
function serviceFor(runs = [], queuedFollowUps = [], blockers = []) {
  return { child: {}, pending: new Map(), call: async command => {
    if (command === 'ai_runtime_snapshot') return { runs };
    if (command === 'ai_conversation_session_get') return { values: { queuedFollowUps } };
    if (command === 'desktop_update_preflight') return { ready: blockers.length === 0, blockers };
    throw new Error('未知读取');
  } };
}
test('文件操作、AI 任务和排队内容阻止重启，失败解除占用', async () => {
  const writing = serviceFor(); writing.pending.set(1, { command: 'workspace_write_file' });
  await assert.rejects(holdServiceForUpdate(writing), /文件保存/);
  for (const service of [serviceFor([{ status: 'running' }]), serviceFor([{ status: 'completed', threadWriterActive: true }]), serviceFor([], [{ status: 'queued' }])]) {
    await assert.rejects(holdServiceForUpdate(service), /AI 任务/);
    assert.equal(service.updateHold, false);
  }
});
test('空闲可更新，保护期间拒绝新写入，失败后恢复业务；未启动服务不阻塞更新', async () => {
  const service = serviceFor([{ status: 'completed' }]);
  service.pending.set(1, { command: 'app_server_request', method: 'event/replay' });
  const release = await holdServiceForUpdate(service);
  assert.equal(service.updateHold, true);
  await assert.rejects(HostService.prototype.call.call(service, 'ai_message_submit', {}), /准备重启/);
  release(); assert.equal(service.updateHold, false);
  await holdServiceForUpdate({ child: null });
});
test('不能读取任务状态时拒绝安装并恢复业务操作', async () => {
  const service = serviceFor(); service.call = async () => { throw new Error('状态读取失败'); };
  await assert.rejects(holdServiceForUpdate(service), /状态读取失败/);
  assert.equal(service.updateHold, false);
});
test('业务服务无响应时超时解除保护，应用继续可操作', async () => {
  const service = serviceFor(); service.call = () => new Promise(() => {});
  await assert.rejects(holdServiceForUpdate(service, { timeoutMs: 10 }), /状态超时/);
  assert.equal(service.updateHold, false);
});
test('主对话空闲时应用在途、写入确认、上传及未知结果仍阻止重启，处理完成后可继续', async () => {
  for (const blocker of ['application_model_run_active', 'write_confirmation_pending', 'attachment_upload_pending', 'application_write_outcome_unsettled']) {
    const blockers = [blocker];
    const service = serviceFor([], [], blockers);
    await assert.rejects(holdServiceForUpdate(service), /应用任务/);
    assert.equal(service.updateHold, false);
    blockers.length = 0;
    const release = await holdServiceForUpdate(service);
    assert.equal(service.updateHold, true);
    release();
    assert.equal(service.updateHold, false);
  }
});
test('应用预检缺失、字段不完整或自相矛盾时明确拒绝并解除保护', async () => {
  for (const preflight of [null, {}, { ready: true }, { ready: true, blockers: [1] }, { ready: true, blockers: ['application_model_run_active'] }, { ready: false, blockers: [] }]) {
    const service = serviceFor();
    const call = service.call;
    service.call = async command => command === 'desktop_update_preflight' ? preflight : call(command);
    await assert.rejects(holdServiceForUpdate(service), /无法确认/);
    assert.equal(service.updateHold, false);
  }
});
