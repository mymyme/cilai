const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const {spawnSync} = require("node:child_process");
const {createPersonalNamespaceCleanup}=require("./personal-namespace-cleanup");

function componentFixture(t, kind) {
  const project=path.resolve(__dirname,"../../../../..");
  const base=path.join(project,".personal-build/namespace-cleanup-components");fs.mkdirSync(base,{recursive:true});
  const root=fs.mkdtempSync(path.join(base,"probe-"));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const profileId="a".repeat(32),userData=path.join(root,"cn.cilai.personal.workbench"),segmentId="b".repeat(32);
  const segment=path.join(userData,"personal-storage-log-segments",segmentId);fs.mkdirSync(segment,{recursive:true});
  fs.writeFileSync(path.join(segment,"segment.json"),JSON.stringify({schemaVersion:1,segmentId,profileId,createdAt:1}));
  fs.writeFileSync(path.join(segment,"host-service.log"),"original frozen log");
  const paths={userData,logs:path.join(userData,"logs"),sessionData:path.join(userData,"browser")};
  const calls=[];let cleanup,operation,owner,ackFailure=false,maintenanceError=null,resumes=0,clears=0;
  const session={storagePath:path.join(paths.sessionData,"Partitions",`ai-browser-${profileId}`),getCacheSize:async()=>0,clearCache:async()=>{clears++;}};
  fs.mkdirSync(session.storagePath,{recursive:true});
  const browser={suspendForStorage(){return {drain:async()=>{},resume(){resumes++;}};}};
  const receipt=(request,state)=>({actionId:request.actionId,kind,state,outcome:state==="acknowledged"?request.outcome:null,reasonCode:request.reasonCode??null,afterCacheBytes:request.afterCacheBytes??null,startedAt:1,acknowledgedAt:state==="acknowledged"?2:null});
  const service={async call(command,{request}){
    calls.push({command,request});
    if(command==="personal_namespace_storage_prepare_internal"){
      owner={pid:process.pid,instanceId:request.shellInstanceId,startFingerprint:"synthetic-component-fingerprint"};
      operation={operationId:"component-operation",profileId,namespacePath:userData,shellOwner:owner,state:"prepared",previewDigest:"sha256:component",logSegmentId:request.logSegmentId,logFiles:request.logFiles,browserSession:request.browserSession,actions:[{actionId:"action",kind,state:"planned"}]};return operation;
    }
    if(command==="personal_namespace_storage_action_begin_internal")return receipt(request,"started");
    if(command==="personal_namespace_storage_action_ack_internal"){
      if(ackFailure){ackFailure=false;throw Error("ack_transport_unconfirmed");}
      operation.actions=[receipt(request,"acknowledged")];return operation.actions[0];
    }
    if(command==="personal_namespace_storage_execute_internal"){
      await cleanup.perform({operation,permits:[{actionId:"action",kind,permitToken:"component-once",shellOwner:owner}]});
      return {operation:{...operation,state:"completed"},maintenanceError};
    }
    if(command==="personal_namespace_storage_finish")return {operation:{...operation,state:"completed"},maintenanceError};
    if(command==="personal_namespace_storage_segment_sync_internal")return {synced:true};
    throw Error("unexpected component command");
  }};
  cleanup=createPersonalNamespaceCleanup({app:{getAppMetrics:()=>[{pid:process.pid}]},browser,service,closeApplications:async()=>{},
    context:()=>({profileId,paths}),existingSession:()=>session});
  return {cleanup,calls,segment,segmentId,clearCount:()=>clears,resumeCount:()=>resumes,
    setAckFailure:()=>{ackFailure=true;},setMaintenanceError:value=>{maintenanceError=value;},
    prepare:()=>cleanup.prepare({request:{clientOperationId:"component-client",itemKinds:[kind],logSegmentId:kind==="browser_http_cache"?null:segmentId}})};
}

test("复核第二个日志时第一个被改，删除前再核并保留变化内容",async t=>{
  const f=componentFixture(t,"host_service_logs"),operation=await f.prepare();
  const promises=require("node:fs/promises"),original=promises.lstat;let changed=false;
  t.mock.method(promises,"lstat",async function(target,...args){
    if(String(target)===path.join(f.segment,"host-service.log.1")&&!changed){changed=true;fs.writeFileSync(path.join(f.segment,"host-service.log"),"new content after initial check");}
    return original.call(this,target,...args);
  });
  await f.cleanup.execute({request:{operationId:operation.operationId,previewDigest:operation.previewDigest}});
  assert.equal(fs.readFileSync(path.join(f.segment,"host-service.log"),"utf8"),"new content after initial check");
  const ack=f.calls.find(item=>item.command==="personal_namespace_storage_action_ack_internal");
  assert.equal(ack.request.outcome,"failed");
});

test("维护结束未确认时不恢复浏览器，明确finish成功才恢复",async t=>{
  const f=componentFixture(t,"browser_http_cache"),operation=await f.prepare();
  f.setMaintenanceError("release_unconfirmed");
  await f.cleanup.execute({request:{operationId:operation.operationId,previewDigest:operation.previewDigest}});
  assert.equal(f.clearCount(),1);assert.equal(f.resumeCount(),0);
  f.setMaintenanceError(null);
  await f.cleanup.finish({request:{operationId:operation.operationId,previewDigest:operation.previewDigest}});
  assert.equal(f.clearCount(),1);assert.equal(f.resumeCount(),1);
});

test("实际清理结束后回执丢失，仅补记原回执且不再次clearCache",async t=>{
  const f=componentFixture(t,"browser_http_cache"),operation=await f.prepare();
  f.setAckFailure();
  await assert.rejects(f.cleanup.execute({request:{operationId:operation.operationId,previewDigest:operation.previewDigest}}),/ack_transport_unconfirmed/);
  assert.equal(f.clearCount(),1);assert.equal(f.resumeCount(),0);
  await f.cleanup.finish({request:{operationId:operation.operationId,previewDigest:operation.previewDigest}});
  assert.equal(f.clearCount(),1);assert.equal(f.resumeCount(),1);
  const acks=f.calls.filter(item=>item.command==="personal_namespace_storage_action_ack_internal");
  assert.equal(acks.length,2);assert.deepEqual(acks[0].request,acks[1].request);
});

test("真实Electron执行冻结日志及HTTP缓存清理，保留登录与后续日志",{skip:process.env.CILAI_PERSONAL_NAMESPACE_CLEANUP_NATIVE!=="1",timeout:110000},()=>{
  const project=path.resolve(__dirname,"../../../../..");
  const fixtures=path.join(project,".personal-build/namespace-cleanup-fixtures");fs.mkdirSync(fixtures,{recursive:true});
  const root=fs.mkdtempSync(path.join(fs.realpathSync(fixtures),"probe-"));
  const sidecar=path.join(root,"sidecar");fs.mkdirSync(sidecar);
  const source=path.join(project,".personal-build/target-live/debug");
  const binaries={};
  for(const [name,target]of[["cilai-host-service","cilai-host-service"],["contexthub-app-server","cilai-app-server"]]){
    const input=path.join(source,name);assert.ok(fs.statSync(input).isFile());assert.equal(fs.lstatSync(input).isSymbolicLink(),false);
    fs.copyFileSync(input,path.join(sidecar,target));binaries[target]=crypto.createHash("sha256").update(fs.readFileSync(input)).digest("hex");
  }
  const evidence=path.join(project,"07测试/实施证据");
  const packageRecord=JSON.parse(fs.readFileSync(path.join(evidence,"T0-T2-r4-最终包内容及身份清单-20261021.json"),"utf8"));
  const vendor=path.join(packageRecord.verifiedExtractedBundle,"Contents/Resources/sidecar");
  fs.cpSync(path.join(vendor,"codex-runtime"),path.join(sidecar,"codex-runtime"),{recursive:true});
  fs.copyFileSync(path.join(vendor,"cilai-node"),path.join(sidecar,"cilai-node"));
  const output=path.join(root,"result.json"),config=path.join(root,"config.json");
  fs.writeFileSync(config,JSON.stringify({root,shell:__dirname,host:path.join(sidecar,"cilai-host-service"),output}),{mode:0o600});
  for(const name of["private-home","tmp"])fs.mkdirSync(path.join(root,name));
  const environment=Object.fromEntries(Object.entries(process.env).filter(([key])=>["PATH","LANG","SystemRoot","WINDIR"].includes(key)));
  Object.assign(environment,{HOME:path.join(root,"private-home"),USERPROFILE:path.join(root,"private-home"),TMPDIR:path.join(root,"tmp")});
  const electron=require("electron");assert.ok(fs.realpathSync(electron).startsWith(project+path.sep));
  const moduleSha256=Object.fromEntries(["personal-namespace-cleanup.js","personal-namespace-storage.js","browser.js","service-log.js","test-diagnostics.js","personal-application-close.js"].map(name=>[name,crypto.createHash("sha256").update(fs.readFileSync(path.join(__dirname,name))).digest("hex")]));
  const probeScriptSha256=crypto.createHash("sha256").update(fs.readFileSync(path.join(evidence,"T5-执行脚本/electron-namespace-probe.cjs"))).digest("hex");
  const result=spawnSync(electron,[path.join(evidence,"T5-执行脚本/electron-namespace-probe.cjs"),config],{env:environment,encoding:"utf8",timeout:100000,maxBuffer:8*1024*1024});
  fs.writeFileSync(path.join(root,"process.log"),(result.stdout??"")+(result.stderr??""),{mode:0o600});
  const record=fs.existsSync(output)?JSON.parse(fs.readFileSync(output,"utf8")):{result:"failed",error:"no actual Electron result"};
  if(record.result==="passed"){
    const business=path.join(root,"cn.cilai.personal.workbench/business");
    const profile=JSON.parse(fs.readFileSync(path.join(business,"local-profile.json"),"utf8"));
    assert.match(profile.profileId,/^[a-f0-9]{32}$/);
    const {DatabaseSync}=require("node:sqlite");
    const database=new DatabaseSync(path.join(business,"profiles",profile.profileId,"contexthub.db"),{readOnly:true});
    try{record.schemaMigrationVersion=database.prepare("SELECT MAX(version) AS version FROM schema_migrations").get().version;}
    finally{database.close();}
  }
  if(result.status!==0)record.result="failed";
  Object.assign(record,{fixtureRoot:root,binaries,moduleSha256,probeScriptSha256,exitStatus:result.status,signal:result.signal,scopeBoundary:"development Electron actual version only; not r4 bundle or Windows; source module checkpoint separate from subsequent changes"});
  fs.writeFileSync(path.join(evidence,`T5-namespace真实Electron-${path.basename(root)}-20261022.json`),JSON.stringify(record,null,2)+"\n");
  assert.equal(result.status,0,JSON.stringify({error:result.error?.message,record,log:path.join(root,"process.log")}));
  assert.equal(record.result,"passed",JSON.stringify(record));
  assert.equal(record.schemaMigrationVersion,76);
  assert.equal(Object.keys(record.checks).length,7);
});
