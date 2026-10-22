const test = require("node:test");
const assert = require("node:assert/strict");
const {createPersonalApplicationClose} = require("./personal-application-close");

function fixture(options = {}) {
  const frames = [{url:"app://localhost/",detached:false},{url:"app://personal-capability/page",detached:false}];
  const owner = {isDestroyed:()=>false,mainFrame:{framesInSubtree:frames}};
  let cleared = 0, sent;
  const closer = createPersonalApplicationClose({getOwner:()=>owner,documents:{clear:()=>cleared++, frames:()=>frames.slice(1)},
    emit:(event,args)=>{sent={event,...args};return 1;},timeoutMs:30,detachTimeoutMs:30,...options});
  return {frames,owner,closer,sent:()=>sent,cleared:()=>cleared};
}
test("没有个人页面时直接释放文档登记，不创建页面",async()=>{
  const f=fixture();f.frames.pop();await f.closer.close();assert.equal(f.sent(),undefined);assert.equal(f.cleared(),1);
});
test("收到当前窗口一次性确认且页面实际脱离后才释放",async()=>{
  const f=fixture();const closing=f.closer.close();assert.equal(f.sent().event,"personal-applications-close");assert.equal(f.cleared(),0);
  f.frames.pop();f.closer.acknowledge({token:f.sent().token},f.owner);await closing;assert.equal(f.cleared(),1);
  assert.throws(()=>f.closer.acknowledge({token:f.sent().token},f.owner),/ack_invalid/);
});
test("伪造窗口或token不能结束维护前关闭确认",async()=>{
  const f=fixture();const closing=f.closer.close();
  assert.throws(()=>f.closer.acknowledge({token:"other"},f.owner),/ack_invalid/);
  assert.throws(()=>f.closer.acknowledge({token:f.sent().token},{...f.owner}),/ack_invalid/);
  f.frames.pop();f.closer.acknowledge({token:f.sent().token},f.owner);await closing;assert.equal(f.cleared(),1);
});
test("界面声称关闭但原生页面仍在时明确拒绝",async()=>{
  const f=fixture();const closing=f.closer.close();f.closer.acknowledge({token:f.sent().token},f.owner);
  await assert.rejects(closing,/personal_application_frames_active/);assert.equal(f.cleared(),0);
});
test("未确认和无监听均不被当作关闭完成",async()=>{
  const f=fixture();await assert.rejects(f.closer.close(),/close_unconfirmed/);assert.equal(f.cleared(),0);
  const missing=fixture({emit:()=>0});await assert.rejects(missing.closer.close(),/listener_unavailable/);assert.equal(missing.cleared(),0);
});
test("并发关闭被拒绝，原操作继续核对",async()=>{
  const f=fixture();const closing=f.closer.close();await assert.rejects(f.closer.close(),/close_busy/);
  f.frames.pop();f.closer.acknowledge({token:f.sent().token},f.owner);await closing;
});

test("真实Electron frame脱离确认，不以渲染层回执代替实际退出",{skip:process.env.CILAI_PERSONAL_CLOSE_NATIVE!=="1",timeout:50_000},async t=>{
  const fs=require("node:fs"),path=require("node:path"),{spawnSync}=require("node:child_process");
  const parent=path.resolve(__dirname,"../../../../../.personal-build/application-close-fixtures");fs.mkdirSync(parent,{recursive:true});
  const root=fs.mkdtempSync(path.join(fs.realpathSync(parent),"probe-"));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const executable=require("electron");assert.ok(fs.realpathSync(executable).startsWith(path.resolve(__dirname,"../../../../..")+path.sep));
  for(const name of ["home","tmp","userData","session","logs"])fs.mkdirSync(path.join(root,name));
  const script=path.join(root,"probe.cjs");
  fs.writeFileSync(script,`const {app,BrowserWindow,protocol}=require('electron');const path=require('node:path');
const root=${JSON.stringify(root)};for(const [key,folder]of [['userData','userData'],['sessionData','session'],['logs','logs']])app.setPath(key,path.join(root,folder));
app.setName('CilaiPersonalApplicationCloseProbe');app.disableHardwareAcceleration();app.on('window-all-closed',()=>{});
protocol.registerSchemesAsPrivileged([{scheme:'app',privileges:{standard:true,secure:true,supportFetchAPI:true}}]);
app.whenReady().then(async()=>{protocol.handle('app',request=>new Response(request.url.includes('localhost')?'<iframe sandbox="allow-scripts" src="app://personal-capability/canary"></iframe>':'<p>personal canary</p>',{headers:{'Content-Type':'text/html',...(request.url.includes('personal-capability')?{'Content-Security-Policy':require(${JSON.stringify(path.join(__dirname,"personal-documents.js"))}).POLICY}:{})}}));
const window=new BrowserWindow({show:false,webPreferences:{sandbox:true,contextIsolation:true}});await window.loadURL('app://localhost/');
const owner=window.webContents;const count=()=>owner.mainFrame.framesInSubtree.filter(f=>!f.detached&&f.url.startsWith('app://personal-capability/')).length;
if(count()!==1)throw Error('native_frame_missing');
const personal=owner.mainFrame.framesInSubtree.find(f=>f.url.startsWith('app://personal-capability/'));
const storage=await personal.executeJavaScript("(async()=>{let local;try{localStorage.setItem('personal-probe','value');local='opened'}catch(e){local=e.name}let indexed;try{indexed=await new Promise(resolve=>{const request=indexedDB.open('personal-probe');request.onsuccess=()=>{request.result.close();resolve('opened')};request.onerror=()=>resolve(request.error.name)})}catch(e){indexed=e.name}return {localStorage:local,indexedDB:indexed}})()");
if(storage.localStorage!=='SecurityError'||storage.indexedDB!=='SecurityError')throw Error('personal_origin_storage_not_isolated');let cleared=0;const factory=require(${JSON.stringify(path.join(__dirname,"personal-application-close.js"))}).createPersonalApplicationClose;
let first;first=factory({getOwner:()=>owner,documents:{clear:()=>cleared++,frames:()=>owner.mainFrame.framesInSubtree.filter(f=>!f.detached&&f.url.startsWith('app://personal-capability/'))},detachTimeoutMs:50,emit:(_event,{token})=>{queueMicrotask(()=>first.acknowledge({token},owner));return 1;}});
let refused=false;try{await first.close()}catch(e){refused=e.message==='personal_application_frames_active'}if(!refused||cleared!==0)throw Error('false_ack_accepted');
let final;final=factory({getOwner:()=>owner,documents:{clear:()=>cleared++,frames:()=>owner.mainFrame.framesInSubtree.filter(f=>!f.detached&&f.url.startsWith('app://personal-capability/'))},emit:(_event,{token})=>{void owner.executeJavaScript("document.querySelector('iframe').remove()").then(()=>final.acknowledge({token},owner));return 1;}});await final.close();
if(count()!==0||cleared!==1)throw Error('actual_detach_unconfirmed');console.log('PERSONAL_CLOSE_PROBE '+JSON.stringify({electron:process.versions.electron,falseAckRejected:refused,remainingPersonalFrames:count(),documentsCleared:cleared,storage}));window.destroy();app.quit();
}).catch(()=>{console.error('personal_close_native_probe_failed');app.exit(1)});`);
  const result=spawnSync(executable,[script,`--user-data-dir=${path.join(root,"userData")}`],{cwd:root,timeout:40_000,encoding:"utf8",maxBuffer:1024*1024,
    env:{HOME:path.join(root,"home"),USERPROFILE:path.join(root,"home"),TMPDIR:path.join(root,"tmp"),TMP:path.join(root,"tmp"),TEMP:path.join(root,"tmp"),PATH:"/usr/bin:/bin:/usr/sbin:/sbin",LANG:"en_US.UTF-8"}});
  assert.equal(result.status,0,result.stderr);const marker=result.stdout.split("\n").find(line=>line.startsWith("PERSONAL_CLOSE_PROBE "));assert.ok(marker);console.log(marker);
});
