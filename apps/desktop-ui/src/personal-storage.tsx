import {useEffect,useState} from "react";
import type {PersonalStorageCategory,PersonalStorageInventory,PersonalStorageOperation} from "./generated/protocol";
import {invoke} from "./host";
import {ConfirmDialog} from "./confirm-dialog";
import {PersonalNamespaceStorage} from "./personal-namespace-storage";

const categories:Record<PersonalStorageCategory,string>={cache:"缓存",logs:"日志",attachments:"附件",application_data:"应用数据",skills:"技能",applications:"应用包",native_history:"原生历史",native_memory:"记忆",recovery_copies:"恢复副本",business_database:"业务记录",configuration:"配置",project_originals:"项目原文件",other_protected:"其他受保护资料"};
const states:Record<PersonalStorageOperation["state"],string>={prepared:"待确认",applying:"结果待核对",completed:"已完成",incomplete:"未完成",cancelled:"已取消"};
const bytes=(value:number|null)=>value!==null&&Number.isSafeInteger(value)&&value>=0?`${value.toLocaleString()} 字节`:"大小未知";
const newId=()=>crypto.randomUUID().replaceAll("-","");
export function storageMessage(reason:unknown) {
  const tokens=new Set((reason instanceof Error?reason.message:String(reason)).match(/[a-z][a-z0-9_]+/g)??[]);
  const labels:Record<string,string>={
    personal_storage_host_namespace_adapter_required:"个人版全局日志由宿主管理，尚未计入。",personal_storage_browser_session_excluded:"浏览器登录资料不扫描；缓存需单独统计。",personal_storage_project_originals_not_scanned:"项目原文件仅列引用，不遍历。",
    personal_storage_business_database_protected:"业务记录受保护。",personal_storage_application_source_protected:"应用源码与当前安装包受保护。",
    personal_storage_application_data_protected:"应用数据受保护。",personal_storage_user_material_protected:"用户材料受保护。",personal_storage_recovery_copy_protected:"恢复副本受保护。",
    personal_storage_mcp_private_home_protected:"MCP私有运行资料受保护。",personal_storage_credential_lock_protected:"凭据操作锁受保护。",personal_storage_instance_state_protected:"服务状态与锁受保护。",
    personal_storage_unclassified_protected:"内容尚未分类，暂不允许清理。",personal_storage_user_skill_protected:"用户技能源码受保护。",
    personal_storage_runtime_configuration_protected:"运行配置受保护。",personal_storage_native_memory_protected:"原生记忆受保护。",personal_storage_native_history_protected:"原生历史受保护。",
    personal_storage_referenced:"仍有业务引用，不能清理。",personal_storage_reference_active:"仍有业务引用，不能清理。",personal_storage_content_changed:"内容已变化，原文件保留，请重新核对。",
    personal_storage_scan_limit:"统计达到安全上限，部分大小未知。",personal_storage_operation_unconfirmed:"清理结果尚未确认，请核对原操作，不要重新删除。",
    personal_storage_regenerable_skill_snapshot:"可重新生成的技能执行快照。",personal_storage_runtime_shell_cache:"可重新生成的Shell快照。",personal_storage_runtime_log:"受管运行日志。",
    personal_storage_runtime_cache:"可重新生成的运行缓存。",personal_storage_unreferenced_skill_package:"无活动引用的旧技能包。",personal_storage_import_candidate:"未被当前安装使用的导入候选。",
    personal_storage_selected_item_protected:"所选资料受保护，未执行清理。",personal_storage_resource_referenced:"仍有业务引用，不能清理。",
    personal_storage_item_protected:"所选资料受保护，未执行清理。",personal_storage_item_changed:"所选内容已变化，请重新核对范围。",
    personal_storage_reference_changed:"资料引用已变化，未执行清理。",personal_storage_writer_active:"仍有写入占用，清理未完成。",
    personal_storage_quiescence_unconfirmed:"资料静止状态尚未确认，清理未完成。",personal_storage_preview_changed:"清理预览已变化，请重新核对。",
    personal_storage_delete_failed:"部分内容未删除，请核对逐项结果。",personal_storage_result_unconfirmed:"清理结果未知，请核对原操作，不要重复执行。",
    personal_maintenance_release_unconfirmed:"维护结束尚未确认，请核对维护状态。",personal_maintenance_drain_unconfirmed:"仍有任务或写入未结束，清理未完成。",
    current_package_referenced:"当前版本正在使用。",business_data_protected:"业务资料受保护。",external_originals_not_scanned:"外部原文件未扫描。",
    namespace_logs_not_scanned:"个人版日志尚未计入。",namespace_browser_cache_not_scanned:"浏览器缓存尚未计入。",
  };
  for(const [code,label] of Object.entries(labels))if(tokens.has(code))return label;
  return "状态尚未确认，请刷新核对。";
}

export function PersonalStorageSettings(props:{profileId?:string}) {
  const [inventory,setInventory]=useState<PersonalStorageInventory|null>(null);
  const [operations,setOperations]=useState<PersonalStorageOperation[]>([]);
  const [selected,setSelected]=useState<string[]>([]);
  const [busy,setBusy]=useState(true);
  const [unknown,setUnknown]=useState(false);
  const [error,setError]=useState("");
  const [notice,setNotice]=useState("");
  const [preview,setPreview]=useState<PersonalStorageOperation|null>(null);
  const load=async()=>{
    const [usage,records]=await Promise.all([invoke<PersonalStorageInventory>("personal_storage_inventory",{request:{}}),invoke<PersonalStorageOperation[]>("personal_storage_list",{request:{}})]);
    if(!usage||!Array.isArray(usage.items)||!Array.isArray(usage.coverage)||!Array.isArray(records))throw new Error("personal_storage_response_invalid");
    if(props.profileId&&usage.profileId!==props.profileId)throw new Error("personal_storage_profile_changed");
    setInventory(usage);setOperations(records);setUnknown(false);setSelected([]);
  };
  useEffect(()=>{let active=true;void load().catch(reason=>{if(active){setUnknown(true);setError(storageMessage(reason));}}).finally(()=>{if(active)setBusy(false);});return()=>{active=false;};},[]);
  const refresh=async()=>{setBusy(true);setError("");try{await load();}catch(reason){setUnknown(true);setError(storageMessage(reason));}finally{setBusy(false);}};
  const prepare=async()=>{
    setBusy(true);setError("");setNotice("");
    try{const operation=await invoke<PersonalStorageOperation>("personal_storage_prepare",{request:{itemIds:selected,clientOperationId:newId()}});setPreview(operation);setOperations(previous=>[operation,...previous.filter(item=>item.operationId!==operation.operationId)]);}
    catch(reason){setUnknown(true);setError(storageMessage(reason));}finally{setBusy(false);}
  };
  const execute=async()=>{
    if(!preview)return;
    const operation=preview;setPreview(null);setBusy(true);setError("");setNotice("");
    try{
      const result=await invoke<{operation:PersonalStorageOperation;maintenanceError:string|null}>("personal_storage_execute",{request:{operationId:operation.operationId,previewDigest:operation.previewDigest,clientOperationId:newId()}});
      if(!result?.operation||result.operation.operationId!==operation.operationId)throw new Error("personal_storage_response_invalid");
      setOperations(previous=>[result.operation,...previous.filter(item=>item.operationId!==operation.operationId)]);setSelected([]);
      if(result.maintenanceError){setUnknown(true);setError(storageMessage(result.maintenanceError));}
      else setNotice(result.operation.state==="completed"?"清理已核对，实际磁盘释放量未提供。":"清理未完成，请核对逐项结果。");
      try{const usage=await invoke<PersonalStorageInventory>("personal_storage_inventory",{request:{}});if(usage.profileId!==props.profileId)throw new Error("personal_storage_profile_changed");setInventory(usage);}
      catch{setUnknown(true);setError("操作已有回执，占用尚未刷新。请单独刷新核对。");}
    }catch(reason){setUnknown(true);setError(storageMessage(reason));}finally{setBusy(false);}
  };
  const settle=async(operation:PersonalStorageOperation,command:string)=>{
    setBusy(true);setError("");setNotice("");
    try{await invoke(command,{request:{operationId:operation.operationId,previewDigest:operation.previewDigest}});await load();}
    catch(reason){setUnknown(true);setError(storageMessage(reason));}finally{setBusy(false);}
  };
  const disabled=busy||unknown||!props.profileId;
  return <section aria-label="存储管理"><header><h2>存储</h2><button type="button" disabled={busy} onClick={()=>void refresh()}>刷新占用</button></header>
    {busy?<p role="status">正在核对…</p>:null}{error?<p role="alert">{error}</p>:null}{notice?<p role="status">{notice}</p>:null}
    {inventory?<><p>{inventory.sizeComplete?"当前资料占用":"已知占用"}：{bytes(inventory.knownLogicalBytes)} · {inventory.activeWriterCount} 个写入占用</p>
      <details><summary>统计范围</summary><ul>{inventory.coverage.map((item,index)=><li key={index}>{categories[item.category]} · {item.scope}：{{covered:"已统计",not_scanned:"未扫描",unavailable:"不可用",external_reference:"仅外部引用"}[item.state]}{item.reason?`；${storageMessage(item.reason)}`:""}</li>)}</ul></details>
      <p>原项目文件、当前版本、附件、历史、记忆和恢复副本受保护。清理仅作用于本次确认的可清理项。</p>
      {inventory.items.map(item=><article key={item.itemId}><label><input type="checkbox" disabled={disabled||!item.cleanupAllowed} checked={selected.includes(item.itemId)} onChange={event=>setSelected(previous=>event.target.checked?[...previous,item.itemId]:previous.filter(id=>id!==item.itemId))}/>{categories[item.category]} · {bytes(item.logicalBytes)}</label>
        <code>{item.displayPath}</code>{!item.cleanupAllowed?<p>受保护{item.references.length?` · ${item.references.length} 个引用`:""}</p>:null}
        {item.reasons.length?<ul>{item.reasons.map(reason=><li key={reason}>{storageMessage(reason)}</li>)}</ul>:null}</article>)}
      <button type="button" disabled={disabled||!selected.length} onClick={()=>void prepare()}>预览清理</button>
    </>:null}
    {operations.length?<section aria-label="清理记录"><h3>清理记录</h3>{operations.map(operation=><article key={operation.operationId}><header><code>{operation.operationId}</code><span>{states[operation.state]}</span></header>
      {operation.blockers.map(reason=><p key={reason}>{storageMessage(reason)}</p>)}
      {operation.result?<><p>已删除文件数：{operation.result.deletedFileCount===null?"未确认":operation.result.deletedFileCount} · {operation.result.deletedLogicalBytes===null?"逻辑大小未确认":bytes(operation.result.deletedLogicalBytes)}</p><p>实际磁盘释放：{operation.result.physicalSpaceReleased===null?"未提供":operation.result.physicalSpaceReleased?"已确认":"尚未确认"}</p>
        <ul>{operation.result.items.map(item=><li key={item.itemId}>{operation.selectedItems.find(selectedItem=>selectedItem.itemId===item.itemId)?.displayPath??item.itemId}：{{deleted:"已删除",retained:"已保留",unconfirmed:"未确认"}[item.outcome]}{item.reason?`；${storageMessage(item.reason)}`:""}</li>)}</ul></>:null}
      {operation.state==="prepared"?<div className="personal-data-actions"><button type="button" disabled={disabled} onClick={()=>setPreview(operation)}>核对清理</button><button type="button" disabled={disabled} onClick={()=>void settle(operation,"personal_storage_cancel")}>取消清理</button></div>:null}
      {operation.state==="applying"?<button type="button" disabled={disabled} onClick={()=>setPreview(operation)}>核对中断清理</button>:null}
    </article>)}</section>:null}
    <PersonalNamespaceStorage profileId={props.profileId}/>
    {preview?<ConfirmDialog title={preview.state==="applying"?"核对中断清理？":"清理所选内容？"} danger confirmLabel={preview.state==="applying"?"核对":"清理"} busy={busy} confirmDisabled={preview.state!=="applying"&&preview.blockers.length>0}
      description={<><p>{preview.state==="applying"?"只核对已有结果，不重新删除。原执行进程仍在时保留占用。":"将关闭个人应用页面并停止当前资料的受管任务。未确认的写入或内容变化会阻止清理。"}</p><p>{preview.selectedItems.length} 项 · {bytes(preview.selectedLogicalBytes)}</p><ul>{preview.selectedItems.map(item=><li key={item.itemId}>{item.displayPath}</li>)}</ul>{preview.blockers.map(reason=><p key={reason}>{storageMessage(reason)}</p>)}<p>逻辑文件删除量与实际磁盘释放量分别记录。</p></>}
      onCancel={()=>setPreview(null)} onConfirm={()=>{if(preview.state==="applying"){const operation=preview;setPreview(null);void settle(operation,"personal_storage_resolve_interrupted");}else void execute();}}/>:null}
  </section>;
}
