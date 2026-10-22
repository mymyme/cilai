import {useEffect, useState} from "react";
import type {PersonalDataCategory, PersonalDataIssue, PersonalDataOperation, PersonalMaintenanceFence, PersonalStorageOperation, PersonalMemoryResetOperation, PersonalApplicationDataOperation, PersonalNamespaceStorageOperation, WorkspaceListResult, WorkspaceSummary} from "./generated/protocol";
import {invoke} from "./host";
import {rpc} from "./app-server-rpc";
import {ConfirmDialog} from "./confirm-dialog";
import {PersonalStorageSettings} from "./personal-storage";
import "./personal-data.css";

type MaintenanceResult = {operation:PersonalDataOperation; maintenanceError:string|null; restartRequired:boolean};
const categoryLabels:Record<PersonalDataCategory,string> = {business_database:"工作台记录",native_history:"原生历史",native_memory:"本机记忆",skills:"技能",applications:"个人应用",application_data:"应用数据",attachments:"附件",configuration:"配置",workspace_originals:"工作区原文"};
const stateLabels:Record<PersonalDataOperation["state"],string> = {prepared:"待确认",capturing:"备份尚未确认",incomplete:"未完成",ready_to_apply:"待离线恢复",applying:"恢复尚未确认",completed:"已完成",cancelled:"已取消"};
const newId = () => crypto.randomUUID().replaceAll("-", "");

export function personalDataError(reason:unknown) {
  const text = reason instanceof Error ? reason.message : String(reason);
  const tokens = new Set(text.match(/[a-z][a-z0-9_]+/g) ?? []);
  const codes:Record<string,string> = {
    local_credentials_and_keys_excluded:"凭据与加密密钥不在归档内。",
    system_credentials_excluded:"凭据与加密密钥不在归档内。", browser_sessions_excluded:"浏览器登录状态不在归档内。",
    native_auth_and_environment_excluded:"原生认证与运行环境不在归档内。", workspace_originals_not_included:"工作区原文未包含，原文件保持原位。",
    restored_connections_require_authorization:"连接需重新认证。",restored_automations_paused:"自动化保持暂停。",restored_tools_require_confirmation:"工具需重新确认权限。",
    native_runtime_private_file_excluded:"原生运行时私有文件未导出。",
    native_history_paths_require_relocation:"原生历史路径需重新定位，继续会话尚待验证。",
    local_recovery_or_transient_copy_excluded:"旧恢复及临时副本未导出，原副本保持原位。",
    recovery_and_temporary_copy_excluded:"旧恢复及临时副本未导出，原副本保持原位。",
    personal_data_quiescence_unconfirmed:"尚未取得完整静止证明，操作未完成。",
    personal_data_workspace_writes_unconfirmed:"工作区仍可能被其他程序写入，备份未完成。",
    personal_data_writer_coverage_unconfirmed:"资料写入覆盖范围尚未确认，备份未完成。",
    personal_data_profile_files_unclassified:"存在尚未分类的资料文件，备份未完成。",
    personal_data_symlink_denied:"资料中包含越界或不支持的链接，操作已停止。",
    personal_data_archive_digest_mismatch:"归档摘要不一致，恢复未开始。",
    personal_data_manifest_invalid:"备份清单或版本不兼容，恢复未开始。",
    personal_data_restore_profile_reference_unknown:"归档中存在无法核对的资料归属，恢复未执行。",
    personal_data_manifest_switch_unconfirmed:"资料切换回执尚未确认，请重启核对，原资料保留。",
    personal_maintenance_owner_alive:"原维护进程仍在运行，不能解除占用。",
    personal_writer_process_alive:"仍有受管进程运行，不能解除占用。",
    personal_writer_process_unconfirmed:"受管进程退出尚未确认，占用继续保留。",
    personal_data_writer_active:"仍有资料写入，维护未完成。",
    personal_maintenance_drain_unconfirmed:"仍有运行、上传或回执未确认，维护未完成。",
    personal_maintenance_ledger_unconfirmed:"历史保存尚未确认。",
    personal_writer_registry_changed:"资料写入清单已变化，请重新核对。",
    personal_maintenance_release_unconfirmed:"维护结束尚未确认，请核对维护状态。",
    personal_data_maintenance_release_unconfirmed:"维护结束尚未确认，请核对维护状态。",
    personal_maintenance_owner_changed:"维护归属已变化，请重新核对。",
    personal_maintenance_owner_active:"原维护进程仍在运行，不能解除占用。",
    personal_maintenance_profile_invalid:"资料范围无法核对。", personal_maintenance_profile_changed:"当前资料已变化。",
    personal_maintenance_native_home_invalid:"原生历史目录无法核对。",
    personal_data_restart_required:"恢复操作已移交，请重启后核对资料。",
    personal_data_restore_result_unconfirmed:"恢复回执未确认，请重启后核对；不会自动重做。",
    personal_data_restore_requires_owned_service:"当前窗口未独占本地服务，不能执行恢复。",
    app_server_other_clients_active:"其他实例正在使用本地服务，请关闭后重新核对。",
    app_server_lifecycle_ownership_required:"当前实例不拥有本地服务，不能执行独占维护。",
    personal_data_owned_service_exit_unconfirmed:"本地服务退出尚未确认，恢复未开始。",
    personal_namespace_storage_browser_transport_unconfirmed:"缓存执行结果或完整进程退出尚未确认，占用继续保留。",
    personal_namespace_action_unconfirmed:"宿主清理结果尚未确认，不会自动重做。",
    personal_data_artifact_changed:"备份内容已变化，未导出。",
    personal_data_export_failed:"备份未能保存，原目标文件已保留。",
    personal_data_archive_invalid:"备份文件无效。", personal_data_backup_invalid:"备份校验未通过。",
    personal_data_preview_changed:"预览已变化，请重新核对。",
    personal_data_quiescence_required:"尚未取得完整静止证明，备份未完成。",
    personal_data_workspace_quiescence_unconfirmed:"工作区原文仍可能被其他程序修改，备份未完成。",
    workspace_index_policy_excluded:"索引排除的目录未导出。",
    personal_data_workspace_full_index_required:"请先完成工作区完整索引，再准备备份。",
    personal_data_workspace_index_unready:"工作区索引尚未就绪，备份未完成。",
    personal_data_workspace_hash_missing:"部分文件尚未完成内容校验，请重新完整索引。",
    personal_data_workspace_snapshot_changed:"工作区内容或索引已变化，请重新核对备份。",
    personal_data_workspace_destination_required:"请先选择原文恢复位置。",
    personal_data_workspace_destination_invalid:"恢复位置无效。",
    personal_data_workspace_destination_protected:"请选择个人资料目录之外的位置。",
    personal_data_workspace_destination_changed:"恢复位置已变化，请重新准备恢复。",
    personal_data_restore_target_exists:"恢复目标已存在，原文件保持原位。",
    personal_data_storage_failed:"资料处理失败，请保留当前资料并查看诊断。",
    local_profile_changed:"当前资料已变化，请重启后核对。",
  };
  for (const [code,label] of Object.entries(codes)) if(tokens.has(code)) return label;
  return "操作回执未确认，请先刷新核对。";
}

function Issues(props:{items:PersonalDataIssue[]}) {
  return props.items.length ? <ul className="personal-data-issues">{props.items.map((issue,index)=><li key={index}>
    {issue.category ? `${categoryLabels[issue.category]}：` : ""}{personalDataError(issue.code)}
    {issue.relativePath ? <code>{issue.relativePath}</code> : null}
  </li>)}</ul> : null;
}

export function PersonalMaintenanceRecovery(props:{onRecovered?:()=>void}) {
  const [fences,setFences] = useState<PersonalMaintenanceFence[]>([]);
  const [operations,setOperations] = useState<{operationId:string;previewDigest:string;resolveCommand:string}[]>([]);
  const [busy,setBusy] = useState(true);
  const [error,setError] = useState("");
  const [confirm,setConfirm] = useState<{command:string; fence:PersonalMaintenanceFence}|null>(null);
  const [notice,setNotice] = useState("");
  const load = async () => {
    const [items,records,storage,memory,applications,namespace] = await Promise.all([invoke<PersonalMaintenanceFence[]>("personal_maintenance_status",{request:{}}),invoke<PersonalDataOperation[]>("personal_data_list",{request:{}}),invoke<PersonalStorageOperation[]>("personal_storage_list",{request:{}}),invoke<PersonalMemoryResetOperation[]>("personal_memory_reset_list",{request:{}}),invoke<PersonalApplicationDataOperation[]>("personal_application_data_list",{request:{}}),invoke<PersonalNamespaceStorageOperation[]>("personal_namespace_storage_list",{request:{}})]);
    if(!Array.isArray(items)||!Array.isArray(records)||!Array.isArray(storage)||!Array.isArray(memory)||!Array.isArray(applications)||!Array.isArray(namespace))throw new Error("personal_maintenance_response_invalid");
    setFences(items.filter(item=>item.state==="draining"));
    setOperations([
      ...records.map(item=>({...item,resolveCommand:"personal_data_resolve_interrupted"})),
      ...storage.map(item=>({...item,resolveCommand:"personal_storage_resolve_interrupted"})),
      ...memory.map(item=>({...item,resolveCommand:"personal_memory_reset_resolve_interrupted"})),
      ...applications.map(item=>({...item,resolveCommand:"personal_application_data_resolve_interrupted"})),
      ...namespace.map(item=>({...item,resolveCommand:"personal_namespace_storage_resolve_interrupted"})),
    ]);
  };
  useEffect(()=>{let active=true; void load().catch(reason=>{if(active)setError(personalDataError(reason));}).finally(()=>{if(active)setBusy(false);});return()=>{active=false;};},[]);
  const refresh = async () => {setBusy(true);setError("");try{await load();}catch(reason){setError(personalDataError(reason));}finally{setBusy(false);}};
  const perform = async () => {
    if(!confirm)return;
    setBusy(true);setError("");setNotice("");
    try {
      if(confirm.command==="personal_data_resolve_interrupted") {
        const operation=operations.find(item=>item.operationId===confirm.fence.operationId);
        if(!operation)throw new Error("personal_data_operation_missing");
        await invoke(operation.resolveCommand,{request:{operationId:operation.operationId,previewDigest:operation.previewDigest}});
        setNotice("已核对中断操作，请再核对维护占用。");
      } else {
        await invoke(confirm.command,{request:{fenceId:confirm.fence.fenceId}});
        setNotice("维护占用已解除。");props.onRecovered?.();
      }
      await load();
    }catch(reason){setError(personalDataError(reason));}
    finally{setConfirm(null);setBusy(false);}
  };
  return <section className="personal-maintenance" aria-label="维护恢复"><header><h2>维护状态</h2><button type="button" disabled={busy} onClick={()=>void refresh()}>核对</button></header>
    {error?<p role="alert">{error}</p>:null}{notice?<p role="status">{notice}</p>:null}
    {!busy&&!error&&!fences.length?<p>没有未结束的维护。</p>:null}
    {fences.map(fence=><article key={fence.fenceId}><p>操作 <code>{fence.operationId}</code></p><div className="personal-data-actions">
      <button type="button" disabled={busy} onClick={()=>setConfirm({command:"personal_maintenance_cancel",fence})}>结束当前维护</button>
      <button type="button" disabled={busy} onClick={()=>setConfirm({command:"personal_data_resolve_interrupted",fence})}>核对中断操作</button>
      <button type="button" disabled={busy} onClick={()=>setConfirm({command:"personal_maintenance_recover",fence})}>解除已退出进程的占用</button>
    </div></article>)}
    {confirm?<ConfirmDialog title={confirm.command==="personal_data_resolve_interrupted"?"核对中断操作？":"结束维护占用？"}
      description="只核对已有结果与进程归属，不重新执行操作。运行状态未知时保留占用，未完成记录保持原状。"
      confirmLabel="确认" busy={busy} onCancel={()=>setConfirm(null)} onConfirm={()=>void perform()}/>:null}
  </section>;
}

export function PersonalRestoreStatus(props:{applying:boolean}) {
  const [busy,setBusy]=useState(false);
  const [error,setError]=useState("");
  return <section className="personal-data-settings" aria-label="资料恢复状态"><h1>{props.applying?"正在恢复资料":"恢复操作已结束"}</h1>
    <p>{props.applying?"正在核对并写入新资料，请保持工作台运行。":"重启后核对当前资料和恢复回执。原资料仍保留在原目录。"}</p>
    {!props.applying?<button type="button" disabled={busy} onClick={()=>{setBusy(true);void invoke("personal_data_restart").catch(reason=>{setError(personalDataError(reason));setBusy(false);});}}>重启</button>:null}
    {error?<p role="alert">{error}</p>:null}
  </section>;
}

export function PersonalDataSettings(props:{profileId?:string}) {
  const [operations,setOperations]=useState<PersonalDataOperation[]>([]);
  const [workspaces,setWorkspaces]=useState<WorkspaceSummary[]>([]);
  const [selection,setSelection]=useState<string[]>([]);
  const [busy,setBusy]=useState(true);
  const [unknown,setUnknown]=useState(false);
  const [error,setError]=useState("");
  const [notice,setNotice]=useState("");
  const [confirm,setConfirm]=useState<PersonalDataOperation|null>(null);
  const [restartRequired,setRestartRequired]=useState(false);
  const load=async()=>{
    const [items,workspaceList]=await Promise.all([invoke<PersonalDataOperation[]>("personal_data_list",{request:{}}),rpc<WorkspaceListResult>("workspace/list")]);
    if(!Array.isArray(items)||!Array.isArray(workspaceList?.workspaces))throw new Error("personal_data_response_invalid");
    setOperations(items);setWorkspaces(workspaceList.workspaces);setUnknown(false);
  };
  useEffect(()=>{let active=true;void load().catch(reason=>{if(active){setUnknown(true);setError(personalDataError(reason));}}).finally(()=>{if(active)setBusy(false);});return()=>{active=false;};},[]);
  const refresh=async()=>{setBusy(true);setError("");try{await load();}catch(reason){setUnknown(true);setError(personalDataError(reason));}finally{setBusy(false);}};
  const prepare=async(kind:"backup"|"restore")=>{
    setBusy(true);setError("");setNotice("");
    try{
      const operation=await invoke<PersonalDataOperation|null>(`personal_data_${kind}_prepare`,{request:kind==="backup"?{selection:{includeWorkspaceFiles:selection},clientOperationId:newId()}:{clientOperationId:newId()}});
      if(operation){setOperations(previous=>[operation,...previous.filter(item=>item.operationId!==operation.operationId)]);setConfirm(operation);}
    }catch(reason){setUnknown(true);setError(personalDataError(reason));}
    finally{setBusy(false);}
  };
  const apply=async()=>{
    if(!confirm||!props.profileId||unknown||confirm.state!=="prepared"
      ||(confirm.kind==="restore"&&(!confirm.restorePreview||confirm.restorePreview.currentProfileId!==props.profileId
        ||(confirm.restorePreview.workspaces.some(item=>item.originalFilesIncluded)&&!confirm.restorePreview.workspaceDestination))))return;
    const operation=confirm;setConfirm(null);setBusy(true);setError("");setNotice("");
    try{
      const request={operationId:operation.operationId,previewDigest:operation.previewDigest,clientOperationId:newId(),...(operation.kind==="restore"?{expectedCurrentProfileId:props.profileId}:{})};
      const result=await invoke<MaintenanceResult>(operation.kind==="backup"?"personal_data_backup_capture":"personal_data_restore_apply",{request});
      if(!result?.operation||result.operation.operationId!==operation.operationId)throw new Error("personal_data_response_invalid");
      setOperations(previous=>[result.operation,...previous.filter(item=>item.operationId!==operation.operationId)]);
      setRestartRequired(result.restartRequired);
      if(result.maintenanceError){setUnknown(true);setError(personalDataError(result.maintenanceError));}
      else setNotice(result.operation.state==="completed"?(operation.kind==="backup"?"备份已生成，尚未导出。":"恢复已处理，请重启后核对。"):"操作未完成，请核对原因。");
    }catch(reason){setUnknown(true);setError(personalDataError(reason));}
    finally{setBusy(false);}
  };
  const chooseWorkspaceDestination=async()=>{
    if(!confirm||confirm.kind!=="restore"||confirm.state!=="prepared"||unknown||confirm.restorePreview?.workspaceDestination)return;
    const operation=confirm;setBusy(true);setError("");
    try{
      const result=await invoke<PersonalDataOperation|null>("personal_data_restore_workspace_plan",{request:{operationId:operation.operationId,previewDigest:operation.previewDigest,clientOperationId:newId()}});
      if(result===null)return;
      if(!result||result.operationId!==operation.operationId||result.profileId!==props.profileId||result.kind!=="restore"
        ||result.state!=="prepared"||!result.restorePreview?.workspaceDestination||result.restorePreview.currentProfileId!==props.profileId)throw new Error("personal_data_response_invalid");
      setConfirm(result);setOperations(previous=>[result,...previous.filter(item=>item.operationId!==result.operationId)]);
    }catch(reason){setUnknown(true);setError(personalDataError(reason));}
    finally{setBusy(false);}
  };
  const cancel=async(operation:PersonalDataOperation)=>{
    setBusy(true);setError("");
    try{await invoke("personal_data_cancel",{request:{operationId:operation.operationId,previewDigest:operation.previewDigest}});await load();}
    catch(reason){setUnknown(true);setError(personalDataError(reason));}finally{setBusy(false);}
  };
  const exportBackup=async(operation:PersonalDataOperation)=>{
    setBusy(true);setError("");setNotice("");
    try{const result=await invoke<{saved:boolean}>("personal_data_backup_export",{request:{operationId:operation.operationId}});if(result.saved)setNotice("备份已导出。");}
    catch(reason){setError(personalDataError(reason));}finally{setBusy(false);}
  };
  const disabled=busy||unknown||!props.profileId||restartRequired;
  if(restartRequired)return <PersonalRestoreStatus applying={false}/>;
  return <section className="personal-data-settings" aria-label="资料管理"><header><h1>数据</h1><button disabled={busy} type="button" onClick={()=>void refresh()}>刷新</button></header>
    {busy?<p role="status">正在处理…</p>:null}{error?<p role="alert">{error}</p>:null}{notice?<p role="status">{notice}</p>:null}
    <section><h2>备份</h2><p>默认包含工作台记录与本地资料。凭据、加密密钥、浏览器登录状态和日志不导出。</p>
      {workspaces.length?<fieldset disabled={disabled}><legend>包含工作区原文</legend>{workspaces.map(workspace=><label key={workspace.workspaceId}><input type="checkbox" checked={selection.includes(workspace.workspaceId)} onChange={event=>setSelection(previous=>event.target.checked?[...previous,workspace.workspaceId]:previous.filter(id=>id!==workspace.workspaceId))}/>{workspace.displayName}<small>{workspace.localRoot}</small></label>)}</fieldset>:null}
      <div className="personal-data-actions"><button type="button" disabled={disabled} onClick={()=>void prepare("backup")}>准备备份</button><button type="button" disabled={disabled} onClick={()=>void prepare("restore")}>选择备份</button></div>
    </section>
    <section aria-label="资料操作记录"><h2>操作记录</h2>{!busy&&!error&&!operations.length?<p>尚无备份或恢复记录。</p>:null}
      {operations.map(operation=><article key={operation.operationId}><header><h3>{operation.kind==="backup"?"备份":"恢复"}</h3><span>{stateLabels[operation.state]}</span></header><code>{operation.operationId}</code>
        <Issues items={operation.blockers}/>
        {operation.backupManifest?<><p>{operation.backupManifest.complete?"归档完整性已核对":"归档未完成"} · {operation.backupManifest.files.length} 个文件</p><Issues items={operation.backupManifest.exclusions}/>
          <ul>{operation.backupManifest.workspaces.map(workspace=><li key={workspace.workspaceId}>{workspace.displayName}：{workspace.originalFilesIncluded?"包含原文":"仅工作台索引，原文仍在原目录"}</li>)}</ul></>:null}
        {operation.result?<><p>已核对：{operation.result.verifiedCategories.map(item=>categoryLabels[item]).join("、")||"无"}</p>
          {operation.result.pendingCategories.length?<p>待验证：{operation.result.pendingCategories.map(item=>categoryLabels[item]).join("、")}</p>:null}
          {operation.result.retainedCurrentRelativePath?<p>原资料保留：<code>{operation.result.retainedCurrentRelativePath}</code></p>:null}<Issues items={operation.result.issues}/></>:null}
        {operation.result?.restoredWorkspaces?.length?<ul>{operation.result.restoredWorkspaces.map(workspace=><li key={workspace.workspaceId}><code>{workspace.sourcePath}</code> → <code>{workspace.restoredPath}</code></li>)}</ul>:null}
        <div className="personal-data-actions">{operation.state==="prepared"?<><button type="button" disabled={disabled} onClick={()=>setConfirm(operation)}>核对并继续</button><button type="button" disabled={disabled} onClick={()=>void cancel(operation)}>取消</button></>:null}
          {operation.kind==="backup"&&operation.state==="completed"&&operation.backupManifest?.complete&&operation.result?.artifactDigest?<button type="button" disabled={disabled} onClick={()=>void exportBackup(operation)}>导出</button>:null}</div>
      </article>)}
    </section>
    <PersonalStorageSettings profileId={props.profileId}/>
    <PersonalMaintenanceRecovery/>
    {confirm?<ConfirmDialog title={confirm.kind==="backup"?"开始备份？":"恢复到新资料？"} confirmLabel={confirm.kind==="backup"?"备份":"恢复"} busy={busy}
      confirmDisabled={unknown||!props.profileId||confirm.state!=="prepared"||(confirm.kind==="restore"&&(!confirm.restorePreview||confirm.restorePreview.currentProfileId!==props.profileId||(confirm.restorePreview.workspaces.some(item=>item.originalFilesIncluded)&&!confirm.restorePreview.workspaceDestination)))}
      description={<><p>将关闭个人应用页面并停止当前资料的受管任务。无法确认资料静止时明确标记未完成。</p>
        {confirm.kind==="backup"?<p>{confirm.selection?.includeWorkspaceFiles.length?"原文与完整索引逐项核对；文件变化或无法核对时，备份未完成。请核对所选原文中的敏感内容。":"工作区原文不在本次归档内。"}</p>:confirm.restorePreview?<><dl>
          <dt>备份资料</dt><dd>{confirm.restorePreview.sourceProfile.profileId}</dd><dt>当前资料</dt><dd>{confirm.restorePreview.currentProfileId}</dd><dt>新资料</dt><dd>{confirm.restorePreview.targetProfileId}</dd>
          <dt>原资料保留</dt><dd>profiles/{confirm.restorePreview.currentProfileId}</dd><dt>内容</dt><dd>{confirm.restorePreview.fileCount} 个文件 · {confirm.restorePreview.totalBytes.toLocaleString()} 字节</dd></dl>
          {confirm.restorePreview.workspaces.some(item=>item.originalFilesIncluded)?<div>
            {confirm.restorePreview.workspaceDestination?<p>原文恢复位置：<code>{confirm.restorePreview.workspaceDestination.targetPath}</code></p>:<button type="button" disabled={busy||unknown} onClick={()=>void chooseWorkspaceDestination()}>选择位置</button>}
            <p>原文写入新目录，原项目保留。</p>
          </div>:null}
          <p>连接需重新认证，工具需重新授权，自动化与记忆保持停用。外部路径需要重新定位，原生历史继续状态另行验证。</p><Issues items={confirm.restorePreview.warnings}/></>:<p>恢复预览不可用。</p>}</>}
      onCancel={()=>setConfirm(null)} onConfirm={()=>void apply()}/>:null}
  </section>;
}
