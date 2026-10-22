import {useEffect,useState} from "react";
import type {PersonalMemoryResetOperation} from "./generated/protocol";
import {invoke} from "./host";
import {ConfirmDialog} from "./confirm-dialog";

const states:Record<PersonalMemoryResetOperation["state"],string>={prepared:"待确认",applying:"结果待核对",completed:"已完成",incomplete:"未完成",cancelled:"已取消"};
const scopes:Record<string,string>={connection_application:"页面 AI",connection_workbench:"工作台",default_application:"页面 AI",default_workbench:"工作台"};
export function memoryResetMessage(reason:unknown) {
  const tokens=new Set((reason instanceof Error?reason.message:String(reason)).match(/[a-z][a-z0-9_]+/g)??[]);
  if(tokens.has("personal_memory_reset_preview_changed")||tokens.has("personal_memory_reset_scope_changed"))return "记忆范围已变化，请重新核对。";
  if(tokens.has("personal_memory_reset_scope_denied")||tokens.has("personal_memory_reset_scope_unavailable"))return "部分记忆范围无法核对，原资料保留。";
  if(tokens.has("personal_memory_reset_response_unconfirmed"))return "官方清理回执尚未确认，请核对原操作。";
  if(tokens.has("personal_maintenance_owner_alive")||tokens.has("personal_memory_reset_owner_alive"))return "原维护进程仍在运行，不能核定中断结果。";
  if(tokens.has("personal_maintenance_drain_unconfirmed"))return "仍有任务或写入未结束，删除未完成。";
  return "删除结果尚未确认，请刷新核对原操作。";
}

function checked(value:PersonalMemoryResetOperation,expected?:PersonalMemoryResetOperation) {
  if(!value||!Array.isArray(value.scopes)||!Array.isArray(value.scopeResults)||!Array.isArray(value.blockers)
    ||(expected&&(value.operationId!==expected.operationId||value.profileId!==expected.profileId||value.previewDigest!==expected.previewDigest)))throw new Error("personal_memory_reset_response_invalid");
  if(value.state==="completed"&&(value.scopes.length!==value.scopeResults.length||value.scopeResults.some(item=>item.outcome!=="reset"||!item.officialResponseReceived||!item.processExitConfirmed)
    ||value.scopes.some(scope=>!value.scopeResults.some(item=>item.scopeId===scope.scopeId))))throw new Error("personal_memory_reset_response_unconfirmed");
  return value;
}

export function PersonalMemoryReset({disabled,onChanged,onBusy,onUnconfirmed}:{disabled:boolean;onChanged:()=>void;onBusy:(value:boolean)=>void;onUnconfirmed:()=>void}) {
  const [operations,setOperations]=useState<PersonalMemoryResetOperation[]>([]);
  const [busy,setBusy]=useState(false);
  const [unknown,setUnknown]=useState(false);
  const [error,setError]=useState("");
  const [notice,setNotice]=useState("");
  const [preview,setPreview]=useState<PersonalMemoryResetOperation|null>(null);
  const load=async()=>{
    const records=await invoke<PersonalMemoryResetOperation[]>("personal_memory_reset_list",{request:{}});
    if(!Array.isArray(records))throw new Error("personal_memory_reset_response_invalid");
    setOperations(records.map(item=>checked(item)));setUnknown(records.some(item=>item.state==="applying"));
  };
  useEffect(()=>{let active=true;void load().catch(reason=>{if(active){setUnknown(true);setError(memoryResetMessage(reason));}});return()=>{active=false;};},[]);
  const perform=async(work:()=>Promise<void>)=>{
    if(busy)return;setBusy(true);onBusy(true);setError("");setNotice("");
    try{await work();}catch(reason){setUnknown(true);setError(memoryResetMessage(reason));setPreview(null);onUnconfirmed();}
    finally{setBusy(false);onBusy(false);}
  };
  const prepare=()=>perform(async()=>{
    const operation=checked(await invoke<PersonalMemoryResetOperation>("personal_memory_reset_prepare",{request:{clientOperationId:crypto.randomUUID().replaceAll("-","")}}));
    setOperations(previous=>[operation,...previous.filter(item=>item.operationId!==operation.operationId)]);setPreview(operation);
  });
  const settle=(operation:PersonalMemoryResetOperation,command:string)=>perform(async()=>{
    const result=checked(await invoke<PersonalMemoryResetOperation>(command,{request:{operationId:operation.operationId,previewDigest:operation.previewDigest}}),operation);
    setOperations(previous=>[result,...previous.filter(item=>item.operationId!==result.operationId)]);setPreview(null);
    if(result.state==="completed")setNotice(result.scopes.some(scope=>scope.retainedNativePaths?.length)?"当前运行时记忆已清理，其他版本目录保留；后台记忆已关闭。":"当前运行时记忆已清理，后台记忆已关闭。");
    else if(result.state==="incomplete")setNotice("删除未完成，请核对各项结果。");
    else if(result.state==="applying"){setUnknown(true);setNotice("结果待核对，请保留原操作。");onUnconfirmed();}
    onChanged();
  });
  const locked=disabled||busy||unknown;
  return <section aria-label="记忆删除"><div className="settings-row"><div><span>删除记忆</span><p>覆盖当前资料的全部原生作用域，保留会话。</p></div><button className="settings-delete" type="button" disabled={locked} onClick={()=>void prepare()}>删除</button></div>
    <button type="button" disabled={busy} onClick={()=>void perform(async()=>{await load();onChanged();})}>刷新删除记录</button>
    {error?<p role="status">{error}</p>:null}{notice?<p role="status">{notice}</p>:null}
    {operations.length?<details><summary>删除记录</summary>{operations.map(operation=><article key={operation.operationId}><p><code>{operation.operationId}</code> · {states[operation.state]} · {operation.scopes.length} 个范围</p>
      {operation.blockers.map(reason=><p key={reason}>{memoryResetMessage(reason)}</p>)}
      {operation.scopeResults.map(result=><p key={result.scopeId}>{scopes[operation.scopes.find(scope=>scope.scopeId===result.scopeId)?.scopeKind??""]??"本机记忆"}：{{pending:"待处理",reset:"已核对删除",failed:"失败",unconfirmed:"未确认"}[result.outcome]}{result.reason?`；${memoryResetMessage(result.reason)}`:""}</p>)}
      {operation.state==="prepared"?<><button type="button" disabled={locked} onClick={()=>setPreview(operation)}>核对删除</button><button type="button" disabled={locked} onClick={()=>void settle(operation,"personal_memory_reset_cancel")}>取消删除</button></>:null}
      {operation.state==="applying"?<button type="button" disabled={busy} onClick={()=>setPreview(operation)}>核对中断删除</button>:null}
    </article>)}</details>:null}
    {preview?<ConfirmDialog title={preview.state==="applying"?"核对中断删除？":"清理本机记忆？"} danger busy={busy} confirmLabel={preview.state==="applying"?"核对":"删除"} confirmDisabled={preview.state!=="applying"&&preview.blockers.length>0}
      description={preview.state==="applying"?<p>只核对已有回执，不重新删除。进程状态未知时保留维护占用。</p>:<><p>将停止受管任务并关闭后台记忆，清理当前官方运行时在 {preview.scopes.length} 个原生范围中的记忆。会话、项目和凭据保留。</p><ul>{preview.scopes.map(scope=><li key={scope.scopeId}>{scopes[scope.scopeKind]??"本机记忆"} · {scope.visibleMemoryFileCount===null?"文件数未知":`${scope.visibleMemoryFileCount} 个可见记忆文件`}{scope.inspectionIssue?`；${memoryResetMessage(scope.inspectionIssue)}`:""}{scope.retainedNativePaths?.length?<p>保留其他版本目录：{scope.retainedNativePaths.join("、")}</p>:null}</li>)}</ul>{preview.blockers.map(reason=><p key={reason}>{memoryResetMessage(reason)}</p>)}</>}
      onCancel={()=>setPreview(null)} onConfirm={()=>void settle(preview,preview.state==="applying"?"personal_memory_reset_resolve_interrupted":"personal_memory_reset_confirm")}/>:null}
  </section>;
}
