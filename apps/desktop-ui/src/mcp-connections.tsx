import { hasWorkbenchCredentialLocation } from "./credential-location";
import {useEffect,useRef,useState} from "react";
import type {McpConnection,McpConnectionChange,McpConnectionConfig,McpPurpose,McpValidationRecord} from "./generated/protocol";
import {invoke} from "./host";
import {ConfirmDialog} from "./confirm-dialog";
import "./mcp-connections.css";
import {Select} from "./select";

function sourceLabel(connection:McpConnection) {
  return connection.config.source.kind==="remote_http"?connection.config.source.url:connection.config.source.program;
}

function McpSourcePreview(props:{connection:McpConnection}) {
  const source=props.connection.config.source;
  return <dl className="mcp-source-preview"><div><dt>{source.kind==="remote_http"?"服务":"程序"}</dt><dd>{sourceLabel(props.connection)}</dd></div>
    {source.kind==="local_command"?<><div><dt>参数</dt><dd>{JSON.stringify(source.args)}</dd></div><div><dt>工作目录</dt><dd>{source.cwd}</dd></div>
      <div><dt>只读目录</dt><dd>{source.permissions.readRoots.join("；")||"未授予"}</dd></div><div><dt>可写目录</dt><dd>{source.permissions.writeRoots.join("；")||"未授予"}</dd></div>
      <div><dt>联网</dt><dd>{source.permissions.network?"允许":"禁止"}</dd></div><div><dt>子进程</dt><dd>{source.permissions.childProcesses?"允许":"禁止"}</dd></div></>:null}
    <div><dt>网络方式</dt><dd>{{direct:"直连",system_proxy:"系统代理",custom_proxy:"指定代理"}[props.connection.config.network.mode]}</dd></div>
    <div><dt>认证</dt><dd>{{none:"无",bearer:"Bearer",environment:"独立环境凭据"}[props.connection.config.authMode]}</dd></div>
    {props.connection.config.environmentVariables.length?<div><dt>凭据变量</dt><dd>{props.connection.config.environmentVariables.join("、")}</dd></div>:null}
    {props.connection.config.network.proxyUrl?<div><dt>代理地址</dt><dd>{props.connection.config.network.proxyUrl}</dd></div>:null}
  </dl>;
}

export function mcpError(reason:unknown) {
  const text=reason instanceof Error?reason.message:String(reason);
  const codes:Record<string,string>={
    mcp_credentials_required:"请填写此工具连接的凭据。",mcp_credentials_invalid:"凭据格式无效。",
    mcp_credentials_reentry_required:"程序、服务或认证方式已变化，请重新输入此连接的凭据。",
    mcp_credential_reentry_required:"请重新保存到工作台本地凭据库。",
    mcp_credential_legacy_cleanup_required:"旧系统凭据需手工清理，工作台未确认删除。",
    mcp_credential_write_failed:"工作台本地凭据库未能保存，连接未提交。",mcp_credential_read_failed:"无法读取工作台本地凭据库。",
    mcp_credential_readback_failed:"本地凭据未能读回确认，连接未提交。",mcp_credential_pending_value_conflict:"待保存凭据与本次输入不同，请核对原操作。",
    mcp_credential_scope_invalid:"凭据归属无效，操作已停止。",mcp_credential_delete_failed:"本地凭据尚未删除，请核对后继续清理。",
    mcp_credential_delete_unconfirmed:"本地凭据删除尚未确认。",mcp_credential_cleanup_in_use:"此凭据仍被引用，暂不能清理。",
    mcp_credential_operation_in_use:"另一个窗口正在处理凭据，请稍后核对。",mcp_connection_changed:"连接已变化，请刷新后重新确认。",
    mcp_revision_conflict:"连接已变化，请刷新后重新确认。",mcp_validation_required:"请先验证此连接。",
    mcp_source_consent_required:"请先确认程序或服务来源。",mcp_catalog_changed:"工具清单已变化，请重新验证并确认权限。",
    mcp_connection_in_use:"仍有任务使用此连接，请先停止或核对任务。",mcp_change_aborted:"此保存已取消。",
    mcp_config_invalid:"工具连接配置无效。",mcp_transport_failed:"工具服务连接失败。",mcp_transport_timeout:"工具服务未在限定时间内响应。",
    mcp_authentication_failed:"工具服务拒绝认证，请检查此连接的凭据。",mcp_protocol_invalid:"工具服务返回了不受支持的协议。",
    mcp_managed_storage_invalid:"工作台受管资料归属无法核对，程序未启动。",
    mcp_managed_storage_protected:"所选范围包含工作台受管资料，请选择独立项目或具体文件目录。",
    mcp_child_processes_not_supported:"当前平台尚不能核对本地工具子进程的完整退出，请取消子进程权限后重新确认。",
    mcp_local_proxy_not_supported:"本地命令不能由工作台强制使用连接代理，请明确选择直连或改用远程服务。",
    mcp_file_permissions_invalid:"本地工具文件权限范围无效。",mcp_file_permissions_changed:"本地路径或链接已变化，请重新核对权限。",
    mcp_original_home_permissions_protected:"所选范围包含受保护的原始配置目录，程序未启动。",mcp_original_home_protection_changed:"受保护目录边界已变化，程序未启动。",
    mcp_source_changed:"本地程序或入口文件已变化，旧工具授权已撤销，请重新验证。",
    mcp_source_arguments_outside_permissions:"入口文件不在已确认的读取范围内，程序未启动。",
    mcp_local_exit_unconfirmed:"无法确认本地工具已退出，请核对任务占用。",mcp_local_timeout:"本地工具响应超时，结果需核对。",
    mcp_local_spawn_failed:"本地工具程序未能启动。",mcp_local_supervision_failed:"本地工具进程归属未确认，执行已停止。",
    mcp_stdio_write_unconfirmed:"工具可能已接收请求，结果未确认。不会自动重做。",mcp_stdio_closed:"本地工具通道已关闭。",
    mcp_tool_call_already_started:"此工具调用已经受理，请核对原回执。",
    mcp_local_isolation_not_supported:"当前平台尚不能隔离此本地命令，程序未启动。",
    mcp_response_contains_credentials:"服务回传了敏感凭据，已拒绝保存此结果。",
    mcp_tool_outcome_unconfirmed:"工具可能已接收操作，结果未确认。不会自动重做。",
    mcp_network_failed:"工具服务网络连接失败。", mcp_http_failed:"工具服务拒绝请求。",
    mcp_protocol_not_supported:"工具服务协议不受支持。", mcp_configuration_invalid:"工具连接配置无效。",
    mcp_tool_authorization_revoked:"工具权限已撤销。", mcp_revision_changed:"连接已变化，请刷新后重新确认。",
    mcp_launch_failed:"本地工具程序未能启动。",mcp_permission_not_supported:"当前平台无法提供所选权限，程序未启动。",
  };
  const tokens=new Set(text.match(/[a-z][a-z0-9_]+/g)??[]);
  for(const [code,label] of Object.entries(codes)) if(tokens.has(code)) return label;
  return "操作未确认，请刷新状态后核对。";
}

function McpEditor(props:{connection?:McpConnection;change?:McpConnectionChange;busy:boolean;blocked?:boolean;onClose:()=>void;
  onSave:(config:McpConnectionConfig,secrets:Record<string,string>|null)=>void}) {
  const initial=props.connection?.config;
  const [name,setName]=useState(initial?.name??"");
  const [kind,setKind]=useState<"local_command"|"remote_http">(initial?.source.kind??"remote_http");
  const [url,setUrl]=useState(initial?.source.kind==="remote_http"?initial.source.url:"");
  const [program,setProgram]=useState(initial?.source.kind==="local_command"?initial.source.program:"");
  const [args,setArgs]=useState(initial?.source.kind==="local_command"?JSON.stringify(initial.source.args):"[]");
  const [cwd,setCwd]=useState(initial?.source.kind==="local_command"?initial.source.cwd:"");
  const [readRoots,setReadRoots]=useState(initial?.source.kind==="local_command"?initial.source.permissions.readRoots.join("\n"):"");
  const [writeRoots,setWriteRoots]=useState(initial?.source.kind==="local_command"?initial.source.permissions.writeRoots.join("\n"):"");
  const [network,setNetwork]=useState(initial?.source.kind==="local_command"?initial.source.permissions.network:false);
  const [children,setChildren]=useState(initial?.source.kind==="local_command"?initial.source.permissions.childProcesses:false);
  const [auth,setAuth]=useState<McpConnectionConfig["authMode"]>(initial?.authMode??"none");
  const [variables,setVariables]=useState(initial?.environmentVariables.join("\n")??"");
  const [networkMode,setNetworkMode]=useState<McpConnectionConfig["network"]["mode"]>(initial?.network.mode??"direct");
  const [proxy,setProxy]=useState(initial?.network.proxyUrl??"");
  const [error,setError]=useState("");
  const form=useRef<HTMLFormElement>(null);
  const fields=(text:string)=>text.split("\n").map(value=>value.trim()).filter(Boolean);
  const environment=fields(variables);
  const frozen=Boolean(props.change);
  const localReference=hasWorkbenchCredentialLocation(props.connection?.credentialRef,"mcp");
  const legacyReference=Boolean(props.connection?.credentialRef)&&!localReference;
  const legacyKeep=frozen&&props.change?.credentialAction==="keep"&&auth!=="none"&&!localReference;
  const requiresInput=auth!=="none"&&(!localReference||props.change?.credentialAction==="replace");
  const submit=(event:React.FormEvent)=>{
    event.preventDefault();setError("");
    if(props.busy||props.blocked||legacyKeep)return;
    let parsed:unknown;
    try {parsed=JSON.parse(args);}catch {setError("参数需填写 JSON 字符串数组。");return;}
    if(!Array.isArray(parsed)||parsed.some(value=>typeof value!=="string")){setError("参数需填写 JSON 字符串数组。");return;}
    const secrets:Record<string,string>=Object.create(null) as Record<string,string>;
    for(const input of form.current?.querySelectorAll<HTMLInputElement>('input[type="password"]')??[]) {
      if(input.value)secrets[input.dataset.key??""]=input.value;
      input.value="";
    }
    const config:McpConnectionConfig={name:name.trim(),source:kind==="remote_http"?{kind,url:url.trim()}:
      {kind,program:program.trim(),args:parsed as string[],cwd:cwd.trim(),permissions:{readRoots:fields(readRoots),writeRoots:fields(writeRoots),network,childProcesses:children}},
      authMode:auth,environmentVariables:auth==="environment"?environment:[],network:{mode:networkMode,proxyUrl:networkMode==="custom_proxy"?proxy.trim():null}};
    props.onSave(config,Object.keys(secrets).length?secrets:null);
  };
  return <section className="mcp-editor" aria-label={frozen?"继续保存工具连接":"编辑工具连接"}><header><h2>{frozen?"继续保存":"工具连接"}</h2><button disabled={props.busy} onClick={props.onClose} type="button">关闭</button></header>
    <form ref={form} onSubmit={submit}>
      <fieldset disabled={props.busy||props.blocked||frozen}>
        <label>名称<input required value={name} onChange={event=>setName(event.target.value)}/></label>
        <label>来源<Select ariaLabel="来源" disabled={props.busy||props.blocked||frozen} value={kind} onChange={value=>{setKind(value as typeof kind);setAuth("none");}} options={[{value:"remote_http",label:"远程 HTTP"},{value:"local_command",label:"本地命令"}]}/></label>
        {kind==="remote_http"?<label>服务地址<input required value={url} onChange={event=>setUrl(event.target.value)} placeholder="https://example.com/mcp"/></label>:<>
          <label>程序<input required value={program} onChange={event=>setProgram(event.target.value)} placeholder="绝对路径"/></label>
          <label>参数<textarea value={args} onChange={event=>setArgs(event.target.value)} rows={2}/></label>
          <label>工作目录<input required value={cwd} onChange={event=>setCwd(event.target.value)} placeholder="绝对路径"/></label>
          <label>只读目录<textarea value={readRoots} onChange={event=>setReadRoots(event.target.value)} rows={2}/></label>
          <label>可写目录<textarea value={writeRoots} onChange={event=>setWriteRoots(event.target.value)} rows={2}/></label>
          <div className="mcp-inline"><label><input type="checkbox" checked={network} onChange={event=>setNetwork(event.target.checked)}/>允许联网</label><label><input type="checkbox" checked={children} onChange={event=>setChildren(event.target.checked)}/>允许子进程</label></div>
        </>}
        <label>认证<Select ariaLabel="认证" disabled={props.busy||props.blocked||frozen} value={auth} onChange={value=>setAuth(value as typeof auth)} options={[{value:"none",label:"无"},kind==="remote_http"?{value:"bearer",label:"Bearer"}:{value:"environment",label:"环境变量"}]}/></label>
        {auth==="environment"?<label>凭据变量名<textarea value={variables} onChange={event=>setVariables(event.target.value)} rows={2}/></label>:null}
        <label>网络<Select ariaLabel="网络" disabled={props.busy||props.blocked||frozen} value={networkMode} onChange={value=>setNetworkMode(value as typeof networkMode)} options={[{value:"direct",label:"直连"},...(kind==="remote_http"?[{value:"system_proxy",label:"系统代理"},{value:"custom_proxy",label:"指定代理"}]:networkMode!=="direct"?[{value:networkMode,label:`${networkMode==="system_proxy"?"系统代理":"指定代理"}（不支持）`,disabled:true}]:[])]}/></label>
        {kind==="local_command"?<p>联网由程序使用，工作台不为本地命令设置代理。</p>:null}
        {networkMode==="custom_proxy"?<label>代理地址<input value={proxy} onChange={event=>setProxy(event.target.value)}/></label>:null}
      </fieldset>
      {auth!=="none"?<fieldset disabled={props.busy||props.blocked||(frozen&&props.change?.credentialAction!=="replace")} className="mcp-credentials"><legend>{frozen&&props.change?.credentialAction==="replace"?"重新输入原操作凭据":"凭据"}</legend>
        {(auth==="bearer"?["token"]:environment).map(key=><label key={key}>{key}<input type="password" data-key={key} required={requiresInput} autoComplete="new-password" spellCheck={false}/></label>)}
        {legacyReference?<p>需重新保存到工作台</p>:null}
        <p>{legacyKeep?"请取消此操作后重新输入凭据。":frozen&&props.change?.credentialAction!=="replace"?"此操作保留本地凭据，无需重新输入。":localReference&&!frozen?"本地凭据已登记。留空保留原凭据。":"保存到工作台本地加密凭据库。"}</p>
      </fieldset>:null}
      {error?<p role="alert">{error}</p>:null}
      <footer><button type="submit" disabled={props.busy||props.blocked||legacyKeep}>{props.busy?"处理中…":frozen?"继续保存":"保存"}</button></footer>
    </form>
  </section>;
}

function McpGrant(props:{connection:McpConnection;busy:boolean;onGrant:(names:string[],purposes:McpPurpose[])=>void}) {
  const [names,setNames]=useState<string[]>([]);
  const [purposes,setPurposes]=useState<McpPurpose[]>([]);
  const validation=props.connection.validation;
  if(!validation||validation.state!=="passed")return null;
  const select=(name:string,checked:boolean)=>setNames(previous=>checked?[...previous,name]:previous.filter(value=>value!==name));
  return <div className="mcp-grant" aria-label="工具权限"><p>{validation.serverName??"服务名称未提供"} · {validation.protocolVersion??"协议未提供"}</p>
    {validation.tools.length?validation.tools.map(tool=><div key={tool.name}><label><input type="checkbox" checked={names.includes(tool.name)} disabled={props.busy} onChange={event=>select(tool.name,event.target.checked)}/>{tool.name}</label>
      {tool.description?<p>{tool.description}</p>:null}<details><summary>输入格式</summary><pre>{JSON.stringify(tool.inputSchema,null,2)}</pre></details></div>):<p>服务未提供工具。</p>}
    <div className="mcp-inline">{(["foreground","automation"] as McpPurpose[]).map(purpose=><label key={purpose}><input type="checkbox" disabled={props.busy} checked={purposes.includes(purpose)} onChange={event=>setPurposes(previous=>event.target.checked?[...previous,purpose]:previous.filter(value=>value!==purpose))}/>{purpose==="foreground"?"前台对话":"自动化"}</label>)}</div>
    <p>仅授权所选工具与用途。外部发送、删除和修改仍需对应执行权限。</p>
    <button type="button" disabled={props.busy||!names.length||!purposes.length} onClick={()=>props.onGrant(names,purposes)}>确认并启用</button>
  </div>;
}

function cleanupConfirmed(change:McpConnectionChange,reference:McpConnectionChange["cleanupReferences"][number],value:unknown):boolean {
  const result=value as McpConnectionChange|null;
  const matches=(item:McpConnectionChange["cleanupReferences"][number])=>item?.service===reference.service&&item?.account===reference.account;
  return Boolean(result&&result.operationId===change.operationId&&result.state===change.state
    &&result.connection?.profileId===change.connection.profileId&&result.connection.connectionId===change.connection.connectionId
    &&result.connection.authGeneration===change.connection.authGeneration&&Array.isArray(result.cleanupReferences)
    &&result.cleanupReferences.some(matches)&&Array.isArray(result.cleanedReferences)&&result.cleanedReferences.some(matches));
}

export function McpConnectionSettings() {
  const [connections,setConnections]=useState<McpConnection[]>([]);
  const [changes,setChanges]=useState<McpConnectionChange[]>([]);
  const [busy,setBusy]=useState(true);
  const [error,setError]=useState("");
  const [notice,setNotice]=useState("");
  const [editor,setEditor]=useState<{connection?:McpConnection;change?:McpConnectionChange}|null>(null);
  const [confirm,setConfirm]=useState<{action:"validate"|"delete";connection:McpConnection}|null>(null);
  const [cleanupConfirm,setCleanupConfirm]=useState<{change:McpConnectionChange;reference:McpConnectionChange["cleanupReferences"][number]}|null>(null);
  const [unknown,setUnknown]=useState(false);
  const load=async()=>{
    const [items,operations]=await Promise.all([invoke<McpConnection[]>("mcp_connection_list",{request:{}}),invoke<McpConnectionChange[]>("mcp_connection_changes",{request:{}})]);
    if(!Array.isArray(items)||!Array.isArray(operations))throw new Error("mcp_response_invalid");
    setConnections(items);setChanges(operations);setUnknown(false);
  };
  useEffect(()=>{
    let current=true;
    void Promise.all([invoke<McpConnection[]>("mcp_connection_list",{request:{}}),invoke<McpConnectionChange[]>("mcp_connection_changes",{request:{}})])
      .then(([items,operations])=>{if(current){if(!Array.isArray(items)||!Array.isArray(operations))throw new Error("mcp_response_invalid");setConnections(items);setChanges(operations);}})
      .catch(reason=>{if(current){setUnknown(true);setError(mcpError(reason));}}).finally(()=>{if(current)setBusy(false);});
    return()=>{current=false;};
  },[]);
  const refresh=async()=>{setBusy(true);setError("");try{await load();}catch(reason){setUnknown(true);setError(mcpError(reason));}finally{setBusy(false);}};
  const mutate=async(command:string,request:unknown,success:string)=>{
    setBusy(true);setError("");setNotice("");
    try {
      const result=await invoke(command,{request});
      if(command==="mcp_connection_cleanup"&&(!cleanupConfirm||!cleanupConfirmed(cleanupConfirm.change,cleanupConfirm.reference,result)))throw new Error("mcp_credential_delete_unconfirmed");
      if(command==="mcp_connection_validate"&&(result as McpValidationRecord)?.state!=="passed")setError(mcpError((result as McpValidationRecord)?.reasonCode));
      else setNotice(success);
      setConfirm(null);setCleanupConfirm(null);setEditor(null);
      try{await load();}catch{setUnknown(true);setError("操作已有回执，列表尚未刷新。请单独刷新核对。");}
    }catch(reason){setUnknown(true);setConfirm(null);setCleanupConfirm(null);setError(mcpError(reason));}
    finally{setBusy(false);}
  };
  const save=(config:McpConnectionConfig,secrets:Record<string,string>|null)=>{
    if(editor?.change){void mutate("mcp_connection_continue_save",{operationId:editor.change.operationId,secrets},"已保存，待验证。");return;}
    const connection=editor?.connection;
    const credentialAction=config.authMode==="none"?(connection?.credentialRef?"clear":"keep"):secrets?"replace":"keep";
    void mutate("mcp_connection_save",{prepare:{operationId:crypto.randomUUID().replaceAll("-",""),connectionId:connection?.connectionId??null,
      expectedRevision:connection?.revision??null,config,credentialAction},secrets},"已保存，待验证。");
  };
  const disabled=busy||unknown;
  return <section className="mcp-settings" aria-label="工具连接"><header><h1>工具连接</h1><div><button type="button" disabled={busy} onClick={()=>void refresh()}>刷新</button><button disabled={disabled} type="button" onClick={()=>setEditor({})}>添加</button></div></header>
    {busy?<p role="status">正在处理…</p>:null}{error?<p role="alert">{error}</p>:null}{notice?<p role="status">{notice}</p>:null}
    {editor?<McpEditor key={editor.change?.operationId??editor.connection?.connectionId??"new"} {...editor} busy={busy} blocked={unknown} onClose={()=>setEditor(null)} onSave={save}/>:null}
    <div className="mcp-list">{connections.filter(item=>!item.deleted).map(connection=><article key={connection.connectionId}><header><h2>{connection.config.name}</h2><span>{connection.enabled?"已启用":"已停用"}</span></header>
      <code>{sourceLabel(connection)}</code>{connection.credentialRef&&!hasWorkbenchCredentialLocation(connection.credentialRef,"mcp")?<p>需重新保存到工作台</p>:null}<p>{connection.config.source.kind==="local_command"?"本地命令":"远程服务"} · {connection.config.authMode==="none"?"无认证":"独立凭据"}</p>
      {connection.validation?.state==="failed"?<p role="alert">{mcpError(connection.validation.reasonCode)}</p>:null}
      <footer><button type="button" disabled={disabled} onClick={()=>setEditor({connection})}>编辑</button><button type="button" disabled={disabled} onClick={()=>setConfirm({action:"validate",connection})}>验证</button>
        {connection.enabled?<button type="button" disabled={disabled} onClick={()=>void mutate("mcp_connection_disable",{connectionId:connection.connectionId,expectedRevision:connection.revision},"已停用。")}>停用</button>:null}
        <button type="button" disabled={disabled} onClick={()=>setConfirm({action:"delete",connection})}>删除</button></footer>
      {!connection.enabled?<McpGrant key={connection.validation?.validationId??connection.revision} connection={connection} busy={disabled} onGrant={(names,purposes)=>void mutate("mcp_connection_grant",{
        connectionId:connection.connectionId,expectedRevision:connection.revision,validationId:connection.validation?.validationId,approvedToolNames:names,purposes,sourceConsent:true},"已启用。")}/>:null}
    </article>)}</div>
    {!busy&&!error&&!connections.some(item=>!item.deleted)?<p>尚无工具连接。</p>:null}
    {changes.some(change=>change.state==="prepared"||change.cleanupReferences.some(reference=>!change.cleanedReferences.some(cleaned=>cleaned.service===reference.service&&cleaned.account===reference.account)))?<section className="mcp-pending" aria-label="待处理工具操作"><h2>待处理操作</h2>
      {changes.map(change=>{const references=change.cleanupReferences.filter(reference=>!change.cleanedReferences.some(cleaned=>cleaned.service===reference.service&&cleaned.account===reference.account));return change.state==="prepared"||references.length?<article key={change.operationId}><strong>{change.connection.config.name}</strong><p>{change.state==="prepared"?"保存未完成":"历史凭据待清理"}</p>
        {change.state==="prepared"?<><button type="button" disabled={disabled} onClick={()=>setEditor({connection:change.connection,change})}>继续保存</button><button type="button" disabled={disabled} onClick={()=>void mutate("mcp_connection_cancel_save",{operationId:change.operationId},"已取消。待清理凭据可单独核对。")}>取消保存</button></>:null}
        {references.map(reference=>hasWorkbenchCredentialLocation(reference,"mcp")?<button type="button" disabled={disabled} key={`${reference.service}:${reference.account}`} onClick={()=>setCleanupConfirm({change,reference})}>清理历史凭据</button>:<p key={`${reference.service}:${reference.account}`}>旧系统凭据需手工清理，工作台未确认删除。</p>)}
      </article>:null;})}</section>:null}
    {confirm?<ConfirmDialog title={confirm.action==="validate"?"验证此来源？":"删除此连接？"} busy={busy} confirmLabel={confirm.action==="validate"?"开始验证":"删除"} danger={confirm.action==="delete"}
      description={confirm.action==="validate"?<><McpSourcePreview connection={confirm.connection}/><p>{confirm.connection.config.source.kind==="local_command"?"将启动该程序并读取工具清单。":"将联系该服务并读取工具清单。"}</p><p>此操作不授权业务工具调用。</p></>:<p>后续不再使用此连接。项目与会话保留，残留凭据需单独清理。</p>}
      onCancel={()=>setConfirm(null)} onConfirm={()=>void mutate(confirm.action==="validate"?"mcp_connection_validate":"mcp_connection_delete",{connectionId:confirm.connection.connectionId,expectedRevision:confirm.connection.revision,...(confirm.action==="validate"?{sourceConsent:true}:{})},confirm.action==="validate"?"验证通过，请确认工具权限。":"已删除。")}/>:null}
    {cleanupConfirm?<ConfirmDialog title="清理历史凭据？" confirmLabel="清理" danger busy={busy}
      description={<><p>{cleanupConfirm.change.connection.config.name}</p><p>只清理已无引用的工作台本地加密凭据，保留项目、会话和操作记录。</p></>}
      onCancel={()=>setCleanupConfirm(null)} onConfirm={()=>void mutate("mcp_connection_cleanup",{operationId:cleanupConfirm.change.operationId,credentialRef:cleanupConfirm.reference},"凭据清理已确认。")}/>:null}
  </section>;
}
