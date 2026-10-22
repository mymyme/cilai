import {useEffect,useRef,useState} from "react";
import type {PersonalStorageCoverage,PersonalStorageItem} from "./generated/protocol";
import {invoke} from "./host";
import {PersonalNamespaceCleanup} from "./personal-namespace-cleanup";

export type PersonalNamespaceInventory={profileId:string;scannedAt:number;observation:"live_metadata";sizeComplete:boolean;knownLogicalBytes:number;coverage:PersonalStorageCoverage[];items:PersonalStorageItem[];issues:string[];retainedLogSegments?:{segmentId:string;logicalBytes:number|null;fileCount:number|null;cleanupAllowed:boolean;reason:string|null}[]};
const scopes:Record<string,string>={"namespace/logs":"宿主日志","namespace/test-diagnostics":"测试日志","browser/http-cache":"浏览器 HTTP 缓存"};
const bytes=(value:number|null)=>value!==null&&Number.isSafeInteger(value)&&value>=0?`${value.toLocaleString()} 字节`:"大小未知";
function reasonLabel(reason:string|null) {
  if(reason==="personal_storage_browser_not_initialized")return "尚未初始化，未统计。";
  if(reason==="personal_storage_inventory_changed")return "内容正在变化，大小未知。";
  if(reason==="personal_storage_link_not_scanned")return "包含链接，未继续扫描。";
  if(reason)return "统计未完成，大小未知。";
  return "";
}

export function PersonalNamespaceStorage({profileId}:{profileId?:string}) {
  const [inventory,setInventory]=useState<PersonalNamespaceInventory|null>(null);
  const [busy,setBusy]=useState(false);
  const [error,setError]=useState("");
  const generation=useRef(0);
  useEffect(()=>{
    const request=++generation.current;
    let active=true;
    setInventory(null);setError("");
    if(!profileId)return;
    setBusy(true);
    void read(profileId).then(value=>{if(active&&request===generation.current)setInventory(value);}).catch(()=>{if(active&&request===generation.current)setError("统计未完成，请刷新核对。");}).finally(()=>{if(active&&request===generation.current)setBusy(false);});
    return()=>{active=false;};
  },[profileId]);
  const refresh=async()=>{
    if(!profileId)return;
    const request=++generation.current;
    setBusy(true);setInventory(null);setError("");
    try{const value=await read(profileId);if(request===generation.current)setInventory(value);}catch{if(request===generation.current)setError("统计未完成，请刷新核对。");}finally{if(request===generation.current)setBusy(false);}
  };
  if(!profileId)return null;
  return <section aria-label="宿主存储"><header><h3>日志与浏览器缓存</h3><button type="button" disabled={busy} onClick={()=>void refresh()}>刷新宿主占用</button></header>
    <p>日志属于当前个人安装；浏览器缓存属于当前资料。此处为实时元数据。</p>
    {busy?<p role="status">正在统计…</p>:null}{error?<p role="status">{error}</p>:null}
    {inventory?<><p>{inventory.sizeComplete?"本组占用":"本组已知占用"}：{bytes(inventory.knownLogicalBytes)}</p>
      <ul>{inventory.items.map(item=>{const coverage=inventory.coverage.find(value=>value.scope===item.displayPath);return <li key={item.itemId}>{scopes[item.displayPath]}：{bytes(item.logicalBytes)}{coverage?.state==="not_scanned"?" · 未统计":""}{coverage?.reason?<p>{reasonLabel(coverage.reason)}</p>:null}</li>;})}</ul>
      {inventory.retainedLogSegments?.length?<p>保留日志段：{inventory.retainedLogSegments.length} 段。</p>:null}
      <PersonalNamespaceCleanup key={profileId} profileId={profileId} inventory={inventory}/></>:null}
  </section>;
}

async function read(profileId:string) {
  const value=await invoke<PersonalNamespaceInventory>("personal_namespace_storage_inventory",{});
  const expected=Object.keys(scopes);
  if(!value||value.profileId!==profileId||value.observation!=="live_metadata"||!Number.isSafeInteger(value.knownLogicalBytes)||value.knownLogicalBytes<0
    ||!Array.isArray(value.items)||value.items.length!==expected.length||new Set(value.items.map(item=>item.displayPath)).size!==expected.length
    ||value.items.some(item=>!expected.includes(item.displayPath)||item.cleanupAllowed||item.managedRelativePath!==null||item.snapshotDigest!==null)
    ||!Array.isArray(value.coverage))throw new Error("personal_storage_response_invalid");
  return value;
}
