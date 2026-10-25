const test = require("node:test");
const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { guardBrowserWebviews } = require("./browser-webviews");

function fixture() {
  const webContents = new EventEmitter();
  const calls = [];
  const browser = { isPartition: value => value === `persist:ai-browser-${"a".repeat(32)}`, adopt: (...args) => calls.push(args) };
  guardBrowserWebviews({ webContents },browser);
  return { webContents,calls,partition:`persist:ai-browser-${"a".repeat(32)}` };
}
test("浏览器标签按当前资料分区附着，页面不取得企业预载或Node能力", () => {
  const f = fixture();
  let refused = false;
  const preferences = { nodeIntegration:true,contextIsolation:false,preload:"/untrusted/preload.js" };
  f.webContents.emit("will-attach-webview",{ preventDefault(){refused=true;} },preferences,{partition:f.partition,src:"about:blank?cilaiTab=tab-1&cilaiConversation=conversation-1"});
  assert.equal(refused,false);
  assert.equal(preferences.nodeIntegration,false);
  assert.equal(preferences.contextIsolation,true);
  assert.equal(Object.hasOwn(preferences,"preload"),false);
  const contents = {};
  f.webContents.emit("did-attach-webview",{},contents);
  assert.deepEqual(f.calls,[["tab-1",contents,"conversation-1"]]);
});
test("旧企业分区、其他资料与缺失标签身份不登记浏览器内容", () => {
  for(const params of [
    {partition:"persist:application-old-company",src:"about:blank?cilaiTab=old"},
    {partition:`persist:ai-browser-${"b".repeat(32)}`,src:"about:blank?cilaiTab=other"},
    {partition:`persist:ai-browser-${"a".repeat(32)}`,src:"about:blank"},
  ]) {
    const f=fixture();let refused=false;
    f.webContents.emit("will-attach-webview",{preventDefault(){refused=true;}},{preload:"/untrusted"},params);
    f.webContents.emit("did-attach-webview",{},{});
    assert.equal(refused,true);assert.deepEqual(f.calls,[]);
  }
});
test("一次标签附着不会把身份复用到下次内容进程", () => {
  const f=fixture();
  f.webContents.emit("will-attach-webview",{preventDefault(){throw Error("unexpected refusal");}},{},{partition:f.partition,src:"about:blank?cilaiTab=only-once"});
  f.webContents.emit("did-attach-webview",{},{});f.webContents.emit("did-attach-webview",{},{});
  assert.equal(f.calls.length,1);
});
test("主界面预载保留受控Host和更新接口，移除旧容器单向通道", () => {
  const interfaces = new Map();const sends=[];
  vm.runInNewContext(readFileSync(path.join(__dirname,"preload.js"),"utf8"),{
    require(name){assert.equal(name,"electron");return {contextBridge:{exposeInMainWorld:(name,value)=>interfaces.set(name,value)},ipcRenderer:{on(){},invoke:(...args)=>{sends.push(args);return Promise.resolve(null);},removeListener(){}}};},window:{addEventListener(){}},console,
  });
  assert.equal(interfaces.has("__CILAI_CONTAINER__"),false);
  assert.equal(typeof interfaces.get("__CILAI_HOST__").invoke,"function");
  assert.equal(typeof interfaces.get("__CILAI_UPDATER__").getState,"function");
  interfaces.get("__CILAI_HOST__").invoke("personal_application_open",{id:"personal"});
  assert.equal(sends[0][0],"host:invoke");
});

test('真实Electron浏览器挂载不执行旧预载并保持实际截图',{skip:process.env.CILAI_PERSONAL_BROWSER_NATIVE!=='1',timeout:50_000},async t=>{
  const fs=require('node:fs'),{spawnSync}=require('node:child_process');
  const parent=path.join(require('./personal-isolation').personalBuildRoot(),'browser-webview-fixtures');fs.mkdirSync(parent,{recursive:true});
  const root=fs.mkdtempSync(path.join(fs.realpathSync(parent),'probe-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const executable=require('electron');assert.ok(fs.realpathSync(executable).startsWith(require('./personal-isolation').personalBuildRoot()+path.sep));
  for(const name of ['home','tmp','userData','session','logs'])fs.mkdirSync(path.join(root,name));
  const hostile=path.join(root,'unapproved-preload.cjs');fs.writeFileSync(hostile,'require(\'electron\').contextBridge.exposeInMainWorld(\'__unapproved_preload__\',true);');
  const config={root,browserModule:path.join(__dirname,'browser.js'),guardModule:path.join(__dirname,'browser-webviews.js'),hostileUrl:require('node:url').pathToFileURL(hostile).href,partition:'persist:ai-browser-'+'a'.repeat(32)};
  const script=path.join(root,'probe.cjs');
  fs.writeFileSync(script,`const {app,BrowserWindow}=require('electron');const path=require('node:path');
const config=${JSON.stringify(config)};for(const[key,folder]of[['userData','userData'],['sessionData','session'],['logs','logs']])app.setPath(key,path.join(config.root,folder));
app.setName('CilaiPersonalBrowserBoundaryProbe');app.disableHardwareAcceleration();app.on('window-all-closed',()=>{});
app.whenReady().then(async()=>{const {BrowserRuntime}=require(config.browserModule);const browser=new BrowserRuntime({});browser.setProfile('a'.repeat(32));
let resolve;const ready=new Promise(r=>resolve=r);const adopt=browser.adopt.bind(browser);browser.adopt=(...args)=>{adopt(...args);resolve(args[1]);};
const window=new BrowserWindow({show:false,width:900,height:700,webPreferences:{webviewTag:true,sandbox:true,contextIsolation:true}});require(config.guardModule).guardBrowserWebviews(window,browser);
const html='<html><body><webview style=width:800px;height:600px partition='+config.partition+' preload='+config.hostileUrl+' src=about:blank?cilaiTab=actual-tab&amp;cilaiConversation=actual-conversation></webview></body></html>';
await window.loadURL('data:text/html;charset=utf-8,'+encodeURIComponent(html));
const contents=await Promise.race([ready,new Promise((_,reject)=>setTimeout(()=>reject(Error('browser_attach_unconfirmed')),10000))]);
if(contents.isLoadingMainFrame())await new Promise(r=>contents.once('did-finish-load',r));
const boundary=await contents.executeJavaScript('({host:typeof window.__CILAI_HOST__,unapproved:typeof window.__unapproved_preload__,node:typeof require})');
if(Object.values(boundary).some(value=>value!=='undefined'))throw Error('browser_privilege_boundary_invalid');
await contents.executeJavaScript('document.body.textContent=123456;document.body.style.backgroundColor=String.fromCharCode(119,104,105,116,101)');
const shot=await browser.screenshot('actual-tab','actual-conversation',10000);if(shot.format!=='jpeg'||Buffer.from(shot.data,'base64').length<100)throw Error('browser_screenshot_missing');
const preferences=contents.getLastWebPreferences();if(preferences.nodeIntegration!==false||preferences.contextIsolation!==true||preferences.preload)throw Error('browser_preferences_invalid');
console.log('BROWSER_BOUNDARY_PROBE '+JSON.stringify({electron:process.versions.electron,boundary,screenshotBytes:Buffer.from(shot.data,'base64').length,format:shot.format}));browser.close();window.destroy();app.quit();
}).catch(error=>{console.error(error.message);app.exit(1)});`);
  new vm.Script(fs.readFileSync(script,'utf8'));
  const result=spawnSync(executable,[script,'--user-data-dir='+path.join(root,'userData')],{cwd:root,timeout:40_000,encoding:'utf8',maxBuffer:1024*1024,env:{HOME:path.join(root,'home'),USERPROFILE:path.join(root,'home'),TMPDIR:path.join(root,'tmp'),TMP:path.join(root,'tmp'),TEMP:path.join(root,'tmp'),PATH:'/usr/bin:/bin:/usr/sbin:/sbin',LANG:'en_US.UTF-8'}});
  assert.equal(result.status,0,result.stderr);const marker=result.stdout.split('\n').find(line=>line.startsWith('BROWSER_BOUNDARY_PROBE '));assert.ok(marker);console.log(marker);
});
