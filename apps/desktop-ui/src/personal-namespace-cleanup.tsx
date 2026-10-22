import {useEffect,useRef,useState} from "react";
import type {PersonalNamespaceStorageKind,PersonalNamespaceStorageOperation} from "./generated/protocol";
import type {PersonalNamespaceInventory} from "./personal-namespace-storage";
import {invoke} from "./host";
import {Select} from "./select";

const labels:Record<PersonalNamespaceStorageKind,string>={host_service_logs:"宿主日志",test_diagnostics_logs:"测试日志",browser_http_cache:"浏览器 HTTP 缓存"};
const states:Record<string,string>={prepared:"待确认",applying:"处理中",completed:"已完成",incomplete:"未完成",cancelled:"已取消"};
const bytes=(n:number|null)=>n!==null&&Number.isSafeInteger(n)&&n>=0?`${n.toLocaleString()} 字节`:"未确认";
type Envelope={operation:PersonalNamespaceStorageOperation;maintenanceError:string|null};

export function PersonalNamespaceCleanup({profileId,inventory}:{profileId:string;inventory:PersonalNamespaceInventory}) {
  const [selected,setSelected]=useState<PersonalNamespaceStorageKind[]>([]);
  const [segment,setSegment]=useState("");
  const [operation,setOperation]=useState<PersonalNamespaceStorageOperation|null>(null);
  const [records,setRecords]=useState<PersonalNamespaceStorageOperation[]>([]);
  const [busy,setBusy]=useState(false);
  const [unknown,setUnknown]=useState(false);
  const [error,setError]=useState("");
  const generation=useRef(0);
  const cacheAvailable=inventory.coverage.some(item=>item.scope==="browser/http-cache"&&item.state==="covered");
  const valid=(value:PersonalNamespaceStorageOperation)=>{
    if(!value||value.profileId!==profileId||typeof value.operationId!=="string"||typeof value.previewDigest!=="string"||!Array.isArray(value.actions)||!(value.state in states))throw new Error("personal_namespace_response_invalid");
    return value;
  };
  const readHistory=async()=>{
    const value=await invoke<PersonalNamespaceStorageOperation[]>("personal_namespace_storage_list",{request:{}});
    if(!Array.isArray(value))throw new Error("personal_namespace_response_invalid");
    return value.map(valid);
  };
  useEffect(()=>{
    const id=++generation.current;let active=true;
    setOperation(null);setSelected([]);setSegment("");setRecords([]);setUnknown(false);setError("");setBusy(false);
    void readHistory().then(value=>{if(active&&generation.current===id)setRecords(value);}).catch(()=>{if(active&&generation.current===id)setError("清理历史未读回。");});
    return()=>{active=false;generation.current++;};
  },[profileId]);
  const refresh=async()=>{
    const id=generation.current;setBusy(true);setError("");
    try{
      const values=await readHistory();
      if(id!==generation.current)return;
      setRecords(values);
      if(operation){const found=values.find(item=>item.operationId===operation.operationId);if(found)setOperation(found);}
      setUnknown(values.some(item=>item.state==="applying"));
    }catch{if(id===generation.current)setError("结果尚未核对，未重新执行。");}
    finally{if(id===generation.current)setBusy(false);}
  };
  const prepare=async()=>{
    const id=generation.current;setBusy(true);setError("");
    try{
      const value=valid(await invoke<PersonalNamespaceStorageOperation>("personal_namespace_storage_prepare",{request:{clientOperationId:crypto.randomUUID(),itemKinds:selected,logSegmentId:segment||null}}));
      if(id===generation.current){setOperation(value);setUnknown(false);}
    }catch{if(id===generation.current){setUnknown(true);setError("预览未确认；保留的日志段可刷新核对。");}}
    finally{if(id===generation.current)setBusy(false);}
  };
  const mutate=async(command:string,envelope=false)=>{
    if(!operation)return;
    const id=generation.current;setBusy(true);setError("");
    try{
      const value=await invoke<PersonalNamespaceStorageOperation|Envelope>(command,{request:{operationId:operation.operationId,previewDigest:operation.previewDigest}});
      const wrapped=envelope?value as Envelope:null;
      const next=valid(wrapped?wrapped.operation:value as PersonalNamespaceStorageOperation);
      if(id===generation.current){setOperation(next);setUnknown(Boolean(wrapped?.maintenanceError)||next.state==="applying");if(wrapped?.maintenanceError)setError("清理回执已保留，维护占用尚未解除。");}
    }catch{if(id===generation.current){setUnknown(true);setError("结果未确认，未自动重做。请核对历史或中断状态。");}}
    finally{if(id===generation.current)setBusy(false);}
  };
  const frozen=busy||unknown||operation?.state==="prepared"||operation?.state==="applying";
  return <div aria-label="宿主存储清理">
    <fieldset disabled={Boolean(frozen)}><legend>清理范围</legend>
      {(Object.keys(labels) as PersonalNamespaceStorageKind[]).map(kind=><label key={kind}><input type="checkbox" checked={selected.includes(kind)} disabled={kind==="browser_http_cache"&&!cacheAvailable}
        onChange={event=>{setOperation(null);setSelected(values=>event.target.checked?[...values,kind]:values.filter(item=>item!==kind));}}/>{labels[kind]}</label>)}
      <Select value={segment} ariaLabel="日志来源" disabled={Boolean(frozen)||!selected.some(kind=>kind!=="browser_http_cache")}
        options={[{value:"",label:"当前日志"},...(inventory.retainedLogSegments??[]).map(item=>({value:item.segmentId,label:`保留段 ${item.segmentId.slice(0,8)} · ${bytes(item.logicalBytes)}`,disabled:!item.cleanupAllowed}))]}
        onChange={value=>{setSegment(value);setOperation(null);}}/>
    </fieldset>
    <p>预览会轮换所选日志；后续新日志保留。取消后日志段仍保留。缓存清理会停止页面请求，保留登录状态。</p>
    <button type="button" disabled={Boolean(frozen)||selected.length===0} onClick={()=>void prepare()}>预览清理</button>
    <button type="button" disabled={busy} onClick={()=>void refresh()}>核对清理历史</button>
    {error?<p role="status">{error}</p>:null}
    {operation?<section aria-label="宿主清理预览"><h4>{states[operation.state]}</h4>
      <ul>{operation.actions.map(action=><li key={action.actionId}>{labels[action.kind]}：{action.state==="acknowledged"?(action.outcome==="succeeded"?"已确认清理":"失败，删除量未确认"):action.state==="unconfirmed"?"执行结果未确认":action.state==="started"?"执行未确认":"待确认"}
        {action.kind==="browser_http_cache"&&action.afterCacheBytes!==null?` · 当前缓存 ${bytes(action.afterCacheBytes)}`:""}</li>)}</ul>
      {operation.logFiles.some(file=>file.exists)?<p>本段日志：{bytes(operation.logFiles.filter(file=>file.exists).reduce((n,file)=>n+(file.logicalBytes??0),0))}；仅清理该段。</p>:null}
      {operation.browserSession?<p>预览缓存：{bytes(operation.browserSession.cacheBytes)}。</p>:null}
      {operation.blockers.length?<p role="status">仍有未解除的占用或未完成核对，不能作为完整成功。</p>:null}
      {operation.state==="prepared"?<><button type="button" disabled={busy||unknown||operation.blockers.length>0} onClick={()=>void mutate("personal_namespace_storage_execute",true)}>确认清理</button><button type="button" disabled={busy||unknown} onClick={()=>void mutate("personal_namespace_storage_cancel")}>取消清理</button></>:null}
      {operation.state==="applying"||operation.state==="incomplete"?<><button type="button" disabled={busy} onClick={()=>void mutate("personal_namespace_storage_resolve_interrupted")}>核对中断</button><button type="button" disabled={busy} onClick={()=>void mutate("personal_namespace_storage_finish",true)}>结束本次维护</button></>:null}
      {operation.state==="completed"?<button type="button" disabled={busy} onClick={()=>void mutate("personal_namespace_storage_finish",true)}>核对维护</button>:null}
      <p>实际磁盘空间释放：未提供。</p>
    </section>:null}
    {records.length?<ul aria-label="宿主清理历史">{records.map(item=><li key={item.operationId}><button type="button" disabled={busy} onClick={()=>{setOperation(item);setUnknown(item.state==="applying");}}>{states[item.state]} · {new Date(item.createdAt).toLocaleString()}</button></li>)}</ul>:null}
  </div>;
}
