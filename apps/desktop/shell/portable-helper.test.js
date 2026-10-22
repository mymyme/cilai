const test = require('node:test');
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const path = require('node:path');
test('Windows 原生助手：事务替换、损坏包拦截、路径防护及失败恢复', { skip: process.platform !== 'win32', timeout: 90000 }, () => {
  const result = spawnSync('powershell.exe', ['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',
    path.join(__dirname,'../scripts/portable-helper-fixture.ps1'), '-HelperPath',path.join(__dirname,'apply-portable-update.ps1')],
  { encoding:'utf8',windowsHide:true,timeout:85000,maxBuffer:1024*1024 });
  assert.equal(result.status,0,result.stderr || result.stdout || result.error?.message);
  const report=JSON.parse(result.stdout.replace(/^\ufeff/,''));
  assert.equal(report.results.length,10); assert.ok(report.results.every(r=>r.passed));
  console.log(JSON.stringify(report));
});
