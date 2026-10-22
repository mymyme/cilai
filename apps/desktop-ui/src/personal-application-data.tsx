import {useEffect,useState} from "react";
import type {CapabilityCreationProject,PersonalApplicationDataOperation} from "./generated/protocol";
import {invoke} from "./host";
import {ConfirmDialog} from "./confirm-dialog";

export function applicationDataMessage(reason:unknown) {
  const tokens=new Set((reason instanceof Error?reason.message:String(reason)).match(/[a-z][a-z0-9_]+/g)??[]);
  const labels:Record<string,string>={personal_application_data_restored_read_only:"恢复来源的历史仅供查看，请在当前资料重新准备操作。",personal_application_data_retained_changed:"保留的旧版数据已变化，未执行回退。",personal_application_data_source_changed:"数据在复制期间发生变化，原版本保留。",personal_application_data_preview_changed:"版本或数据范围已变化，请重新核对。",personal_application_data_snapshot_limit:"数据超过本次复制限制，原版本保留。",personal_application_data_path_invalid:"应用数据路径无法核对，原版本保留。",personal_capability_state_conflict:"应用版本已变化，请刷新核对。",personal_maintenance_drain_unconfirmed:"仍有任务或写入未结束，版本切换未完成。",personal_maintenance_release_unconfirmed:"维护结束尚未确认，请核对记录。"};
  for(const [code,label] of Object.entries(labels))if(tokens.has(code))return label;
  return "版本切换结果尚未确认，请刷新核对原操作。";
}
const states:Record<PersonalApplicationDataOperation["state"],string>={prepared:"待确认",applying:"结果待核对",completed:"已完成",incomplete:"未完成",cancelled:"已取消"};
function checked(operation:PersonalApplicationDataOperation,projectId:string) {
  if(!operation||operation.creationProjectId!==projectId||typeof operation.operationId!=="string"||!operation.operationId||typeof operation.previewDigest!=="string"||!Object.hasOwn(states,operation.state))throw new Error("personal_application_data_response_invalid");
  return operation;
}
export function PersonalApplicationData({project,disabled,onBusy,onChanged}:{project:CapabilityCreationProject;disabled:boolean;onBusy:(value:boolean)=>void;onChanged:()=>Promise<void>}) {
  const [records,setRecords]=useState<PersonalApplicationDataOperation[]>([]);
  const [busy,setBusy]=useState(false);
  const [unknown,setUnknown]=useState(false);
  const [error,setError]=useState("");
  const [notice,setNotice]=useState("");
  const [preview,setPreview]=useState<PersonalApplicationDataOperation|null>(null);
  const load=async()=>{
    const items=await invoke<PersonalApplicationDataOperation[]>("personal_application_data_list",{request:{}});
    if(!Array.isArray(items))throw new Error("personal_application_data_response_invalid");
    const own=items.filter(item=>item.creationProjectId===project.creationProjectId).map(item=>checked(item,project.creationProjectId));
    setRecords(own);setUnknown(own.some(item=>item.state==="applying"&&!item.readOnlyAfterRestore));
  };
  useEffect(()=>{let active=true;void load().catch(reason=>{if(active){setUnknown(true);setError(applicationDataMessage(reason));}});return()=>{active=false;};},[project.creationProjectId]);
  const run=async(work:()=>Promise<void>)=>{
    if(busy)return;setBusy(true);onBusy(true);setError("");setNotice("");
    try{await work();}catch(reason){setPreview(null);setUnknown(true);setError(applicationDataMessage(reason));}finally{setBusy(false);onBusy(false);}
  };
  const prepare=(source:PersonalApplicationDataOperation)=>run(async()=>{
    if(!source.sourcePackageDigest||source.readOnlyAfterRestore)return;
    const result=checked(await invoke<PersonalApplicationDataOperation>("personal_application_data_prepare",{request:{creationProjectId:project.creationProjectId,expectedRevision:project.stateRevision,packageDigest:source.sourcePackageDigest,clientOperationId:crypto.randomUUID(),action:"rollback",targetOperationId:source.operationId}}),project.creationProjectId);
    if(result.action!=="rollback"||result.packageDigest!==source.sourcePackageDigest||result.readOnlyAfterRestore)throw new Error("personal_application_data_response_invalid");
    setPreview(result);setRecords(previous=>[result,...previous.filter(item=>item.operationId!==result.operationId)]);
  });
  const settle=(operation:PersonalApplicationDataOperation,command:string)=>run(async()=>{
    const result=checked(await invoke<PersonalApplicationDataOperation>(command,{request:{operationId:operation.operationId,previewDigest:operation.previewDigest}}),project.creationProjectId);
    if(result.operationId!==operation.operationId||result.previewDigest!==operation.previewDigest)throw new Error("personal_application_data_response_invalid");
    setPreview(null);setRecords(previous=>[result,...previous.filter(item=>item.operationId!==result.operationId)]);
    if(command==="personal_application_data_rollback"&&result.state==="completed"){
      if(result.activatedRevision===null)throw new Error("personal_application_data_response_invalid");
      setNotice("已切换到保留的版本与数据。");await onChanged();
    }else if(result.state==="incomplete")setNotice("操作未完成，原版本与保留数据仍可核对。");
    else if(result.state==="applying")setUnknown(true);
  });
  const locked=disabled||busy||unknown;
  return <section aria-label="应用版本与数据"><header><h4>版本与数据</h4><button type="button" disabled={busy} onClick={()=>void run(load)}>刷新记录</button></header>
    {error?<p role="status">{error}</p>:null}{notice?<p role="status">{notice}</p>:null}
    {records.map(record=><article key={record.operationId}><p>{record.action==="rollback"?"回退":"安装或升级"} · {states[record.state]}</p><code>{record.packageDigest}</code>
      {record.readOnlyAfterRestore?<p>恢复来源历史，仅供查看。</p>:null}{record.reason?<p>{applicationDataMessage(record.reason)}</p>:null}
      {!record.readOnlyAfterRestore&&record.state==="completed"&&record.sourcePackageDigest&&record.action!=="rollback"?<button type="button" disabled={locked} onClick={()=>void prepare(record)}>准备回退</button>:null}
      {!record.readOnlyAfterRestore&&record.state==="prepared"&&record.action==="rollback"?<><button type="button" disabled={locked} onClick={()=>setPreview(record)}>核对回退</button><button type="button" disabled={locked} onClick={()=>void settle(record,"personal_application_data_cancel")}>取消回退</button></>:null}
      {!record.readOnlyAfterRestore&&record.state==="applying"?<button type="button" disabled={busy} onClick={()=>setPreview(record)}>核对中断操作</button>:null}
    </article>)}
    {preview?<ConfirmDialog title={preview.state==="applying"?"核对中断操作？":"回退版本与数据？"} danger busy={busy} confirmLabel={preview.state==="applying"?"核对":"回退"}
      description={preview.state==="applying"?<p>只核对已有回执，不重新复制、安装或切换。</p>:<><p>将停止受管任务，恢复所选旧版本的应用、数据、权限与启停状态。当前版本和数据保留。</p><code>{preview.packageDigest}</code><p>外部文件保留当前内容，不随应用回退。</p></>}
      onCancel={()=>setPreview(null)} onConfirm={()=>void settle(preview,preview.state==="applying"?"personal_application_data_resolve_interrupted":"personal_application_data_rollback")}/>:null}
  </section>;
}
