const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const isolation = require('./personal-isolation');

async function capturePersonalScreenshots({ window, service, root, startup, frontendErrors, destination }) {
  const temporary = fs.realpathSync(process.env.CILAI_BUILD_RUN_TMP);
  const output = path.resolve(destination);
  const releases = path.join(isolation.personalBuildRoot(), 'releases') + path.sep;
  isolation.rejectLinkedAncestors(output);
  if (!fs.realpathSync(root).startsWith(temporary + path.sep) || !output.startsWith(releases)
    || startup.result !== 'passed' || !startup.ui.assistantReady || startup.conversations !== 0 || startup.memoryEnabled) {
    throw new Error('product_screenshot_scope_invalid');
  }
  fs.mkdirSync(output, { recursive: true });
  const ownerPath = path.join(output, 'capture.json');
  const names = ['personal-assistant.png'];
  if (fs.readdirSync(output).length) {
    const owner = JSON.parse(fs.readFileSync(ownerPath, 'utf8'));
    if (owner.schema !== 'cilai-product-screenshots-v1' || owner.version !== require('../package.json').version
      || fs.readdirSync(output).some(name => ![...names, 'capture.json'].includes(name))) throw new Error('product_screenshot_destination_unowned');
  }
  fs.writeFileSync(ownerPath, JSON.stringify({ schema: 'cilai-product-screenshots-v1', version: require('../package.json').version, status: 'preparing' }));
  const profile = await service.call('local_profile_status', {});
  if (profile.profileId !== startup.profile.profileId) throw new Error('product_screenshot_profile_changed');
  const session = await service.call('ai_conversation_session_get', {});
  if (typeof session.sessionId !== 'string' || !session.sessionId) throw new Error('product_screenshot_session_invalid');
  const save = (kind, operationId, item) => service.call('ai_pa_records', {
    runtimeSessionId: session.sessionId, kind, operationId, request: { action: 'save', item },
  });
  await save('todo', 'screenshot-todo-1', { title: '梳理本周重点', details: '演示事项：整理优先级、时间安排和待补信息。',
    dueAt: '2026-11-18T17:00:00+08:00', reminder: { localTime: '2026-11-18T09:30', timeZone: 'Asia/Shanghai', repeat: 'daily' } });
  await save('todo', 'screenshot-todo-2', { title: '核对方案资料', details: '演示事项：确认版本，保留来源和待确认问题。', dueAt: null, reminder: null });
  await save('memo', 'screenshot-memo-1', { title: '项目推进约定', body: '每项工作明确目标、成果与完成时间。需要补充时先确认，再继续推进。完成后整理可核对的成果。', url: null, scope: null });
  await save('memo', 'screenshot-memo-2', { title: '周会准备清单', body: '本周进展、待补信息、下一步安排。', url: null, scope: null });
  window.setContentSize(1280, 900);
  await window.webContents.executeJavaScript(`(() => {
    const navigation = document.querySelector('.activity-nav-toggle');
    if (!navigation) throw new Error('product_screenshot_navigation_missing');
    if (navigation.getAttribute('aria-expanded') !== 'true') navigation.click();
    for (const kind of ['todo', 'memo']) window.dispatchEvent(new CustomEvent('cilai-personal-records-changed', {detail:{runtimeSessionId:${JSON.stringify(session.sessionId)},kind}}));
    const input=document.querySelector('.pa-page textarea[aria-label="给个人助理发消息"]');
    if (!input) throw new Error('product_screenshot_composer_missing');
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set.call(input,'帮我整理本周工作，分成几项推进，缺少的信息及时告诉我。');
    input.dispatchEvent(new Event('input',{bubbles:true}));
    input.blur();
  })()`);
  async function waitFor(texts) {
    const deadline = Date.now() + 15_000;
    do {
      if (frontendErrors.length) throw new Error('product_screenshot_frontend_error');
      if (await window.webContents.executeJavaScript(`(() => {
        const expanded = document.querySelector('.activity-nav-toggle')?.getAttribute('aria-expanded') === 'true';
        const labels = [...document.querySelectorAll('[aria-label="主导航"] button > span')];
        return expanded && labels.length === 8 && labels.every(node => {
          const rect = node.getBoundingClientRect(), style = getComputedStyle(node);
          return rect.width > 0 && rect.height > 0 && style.visibility === 'visible' && style.opacity === '1';
        }) && ${JSON.stringify(texts)}.every(text=>document.body.innerText.includes(text));
      })()`)) {
        await window.webContents.executeJavaScript('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
        return;
      }
      await new Promise(resolve => setTimeout(resolve, 100));
    } while (Date.now() < deadline);
    throw new Error('product_screenshot_content_unready');
  }
  const captures = [];
  async function capture(name, text) {
    await waitFor(text);
    const bytes = (await window.webContents.capturePage()).toPNG();
    fs.writeFileSync(path.join(output, name), bytes);
    captures.push({ name, sha256: crypto.createHash('sha256').update(bytes).digest('hex'), bytes: bytes.length });
  }
  await capture(names[0], ['梳理本周重点', '项目推进约定']);
  const snapshot = await service.call('ai_runtime_snapshot', {});
  if (snapshot.runs.length || snapshot.messages.length || frontendErrors.length) throw new Error('product_screenshot_execution_unexpected');
  const evidence = { schema: 'cilai-product-screenshots-v1', version: require('../package.json').version,
    modelCalls: 0, draftSent: false, manualDemoRecords: 4, navigationExpanded: true, captures };
  fs.writeFileSync(ownerPath, JSON.stringify(evidence, null, 2));
  console.log('[product-screenshots]', JSON.stringify(evidence));
}
module.exports = { capturePersonalScreenshots };
