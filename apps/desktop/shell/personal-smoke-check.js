const fs = require('node:fs');
const path = require('node:path');

async function checkPersonalStartup({ window, service, root }) {
  const deadline = Date.now() + 30_000;
  let status;
  let ui;
  do {
    status = await service.call('app_server_status', {});
    if (status.runtimeInitializationError || status.phase === 'failed') throw new Error(status.runtimeInitializationError ?? status.errorReason);
    ui = await window.webContents.executeJavaScript(`(() => ({
      navigation: [...document.querySelectorAll('[aria-label="主导航"] button')].map(node => node.getAttribute('aria-label')),
      login: Boolean(document.querySelector('.enterprise-login-page')),
      alerts: [...document.querySelectorAll('[role="alertdialog"]')].map(node => node.innerText),
      title: document.title,
    }))()`);
    if (status.phase === 'running' && ui.navigation.length === 7) break;
    await new Promise(resolve => setTimeout(resolve, 200));
  } while (Date.now() < deadline);
  if (status.phase !== 'running' || ui.navigation.join(',') !== '工作台,对话,项目,文件,自动化,技能,应用' || ui.login || ui.alerts.length) {
    throw new Error(`个人启动检查失败：${JSON.stringify({ status, ui })}`);
  }
  const profile = await service.call('local_profile_status', {});
  if (!/^[a-f0-9]{32}$/.test(profile.profileId)) throw new Error('本地资料标识无效');
  const memory = await service.call('ai_memory_settings', {});
  if (memory.enabled) throw new Error('新资料不得自动启用记忆');
  const snapshot = await service.call('ai_runtime_snapshot', {});
  for (const command of ['enterprise_login', 'capture_request']) {
    let denied = false;
    try { await service.call(command, {}); } catch (error) { denied = /personal_feature_removed/.test(error.message); }
    if (!denied) throw new Error(`已删除功能仍可调用：${command}`);
  }
  const screenshot = path.join(root, 'personal-startup.png');
  fs.writeFileSync(screenshot, (await window.webContents.capturePage()).toPNG());
  const evidence = { platform: process.platform, arch: process.arch, profile, ui, memoryEnabled: memory.enabled,
    conversations: snapshot.conversations?.length ?? null, screenshot, result: 'passed',
    scope: '全新独立资料的无模型桌面启动；不包含真实模型、工具或完整首版验收' };
  fs.writeFileSync(path.join(root, 'personal-startup.json'), JSON.stringify(evidence, null, 2));
  console.log('[personal-check]', JSON.stringify(evidence));
  return evidence;
}

module.exports = { checkPersonalStartup };
