const ACTIVE_RUNS = new Set(['queued', 'running', 'paused', 'terminating', 'waiting_permission']);
const READ_COMMANDS = new Set(['ai_runtime_snapshot', 'ai_runtime_activity_snapshot', 'ai_conversation_session_get', 'desktop_update_preflight']);
const READ_METHODS = new Set(['event/replay', 'workspace/list', 'workspace/listDirectory', 'project/list']);

function isUpdateRead(command, method) {
  return READ_COMMANDS.has(command) || (command === 'app_server_request' && READ_METHODS.has(method));
}

async function holdServiceForUpdate(service, { timeoutMs = 10000 } = {}) {
  // The error/login page can still update when no business process is running.
  if (!service.child) return () => {};
  if ([...service.pending.values()].some(entry => !isUpdateRead(entry.command, entry.method))) {
    throw new Error('仍有文件保存、上传或其他操作正在进行，请完成后再重启更新。');
  }
  service.updateHold = true;
  const release = () => { service.updateHold = false; };
  let timer;
  try {
    const [runtime, session, preflight] = await Promise.race([
      Promise.all([service.call('ai_runtime_snapshot'), service.call('ai_conversation_session_get'), service.call('desktop_update_preflight')]),
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('确认任务状态超时，请稍后重试更新。')), timeoutMs); }),
    ]);
    if (!Array.isArray(runtime?.runs) || !session?.values || typeof preflight?.ready !== 'boolean'
      || !Array.isArray(preflight.blockers) || preflight.blockers.some(item => typeof item !== 'string')
      || preflight.ready !== (preflight.blockers.length === 0)) throw new Error('无法确认当前任务状态，请稍后重试更新。');
    if (runtime.runs.some(run => ACTIVE_RUNS.has(run.status) || run.threadWriterActive)
      || session.values.queuedFollowUps?.some(item => ['queued', 'sending'].includes(item.status))) {
      throw new Error('仍有 AI 任务或排队消息，请等待完成或停止任务后再重启更新。');
    }
    if (!preflight.ready) throw new Error('仍有应用任务、写入确认或材料上传，请处理完成后再重启更新。');
    return release;
  } catch (error) {
    release();
    throw error;
  } finally {
    clearTimeout(timer);
  }
}

module.exports = { isUpdateRead, holdServiceForUpdate };
