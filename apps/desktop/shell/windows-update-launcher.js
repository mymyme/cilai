const path = require('node:path');
// Windows PowerShell 5.1 需要独立控制台；DETACHED_PROCESS 会提前退出，继承控制台会随旧客户端关闭。
const BOOTSTRAP = `$arguments = @('-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',('"' + $env:CILAI_UPDATE_HELPER + '"'),'-Handoff',('"' + $env:CILAI_UPDATE_HANDOFF + '"')); Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList $arguments -WorkingDirectory ([IO.Path]::GetDirectoryName($env:CILAI_UPDATE_HANDOFF)) -WindowStyle Hidden`;
function portableHelperInvocation(helper, handoff, environment = process.env) {
  return {
    command: path.join(environment.SystemRoot, 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe'),
    args: ['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand',Buffer.from(BOOTSTRAP,'utf16le').toString('base64')],
    // Both launch stages must leave the old installation before it is renamed.
    options: { cwd: path.dirname(handoff), detached: false, windowsHide: true, stdio: 'ignore', env: { ...environment, CILAI_UPDATE_HELPER: helper, CILAI_UPDATE_HANDOFF: handoff } },
  };
}
module.exports = { portableHelperInvocation };
