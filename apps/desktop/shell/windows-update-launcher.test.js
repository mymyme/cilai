const test = require('node:test');
const assert = require('node:assert/strict');
const { mkdtemp,writeFile,readFile,mkdir }=require('node:fs/promises');
const { spawnSync }=require('node:child_process');
const {tmpdir}=require('node:os');const path=require('node:path');
test('Windows 原生启动器：父进程退出后隐藏助手继续执行，支持中文空格路径',{skip:process.platform!=='win32',timeout:25000},async()=>{
  const root=await mkdtemp(path.join(tmpdir(),'cilai-launch-lifetime-'));const helper=path.join(root,'后台 助手.ps1');const handoff=path.join(root,'交接 文件.json');
  const started=path.join(root,'started');const done=path.join(root,'done');
  const program=path.join(root,'旧版 程序');await mkdir(program);
  await writeFile(handoff,JSON.stringify({started,done}));
  await writeFile(helper,'\ufeffparam([string]$Handoff)\n$Request=Get-Content -LiteralPath $Handoff -Raw -Encoding UTF8 | ConvertFrom-Json\n[IO.File]::WriteAllText($Request.started,"started")\nStart-Sleep -Seconds 2\n$Result=@{status="survived";cwd=[Environment]::CurrentDirectory;location=$PWD.Path}|ConvertTo-Json -Compress\n[IO.File]::WriteAllText($Request.done,$Result)\n');
  const entry=path.join(root,'parent.cjs');
  await writeFile(entry,`const {spawn}=require('node:child_process');const fs=require('node:fs');
const {portableHelperInvocation}=require(${JSON.stringify(path.join(__dirname,'windows-update-launcher.js'))});
const call=portableHelperInvocation(${JSON.stringify(helper)},${JSON.stringify(handoff)});
const child=spawn(call.command,call.args,call.options);child.once('spawn',()=>child.unref());
const deadline=Date.now()+15000;const tick=setInterval(()=>{if(fs.existsSync(${JSON.stringify(started)})){clearInterval(tick);process.exit(0);}if(Date.now()>deadline)process.exit(1);},50);`);
  const result=spawnSync(process.execPath,[entry],{cwd:program,encoding:'utf8',windowsHide:true,timeout:18000});
  assert.equal(result.status,0,result.stderr||result.error?.message);
  let value;for(let i=0;i<80;i++){value=await readFile(done,'utf8').catch(()=>null);if(value)break;await new Promise(r=>setTimeout(r,100));}
  const outcome=JSON.parse(value);
  assert.equal(outcome.status,'survived');
  assert.equal(path.resolve(outcome.cwd).toLowerCase(),path.resolve(root).toLowerCase());
  assert.equal(path.resolve(outcome.location).toLowerCase(),path.resolve(root).toLowerCase());
});
