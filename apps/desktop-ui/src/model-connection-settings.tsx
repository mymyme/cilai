import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { aiErrorMessage } from "./ai-runtime";
import { hasWorkbenchCredentialLocation } from "./credential-location";
import { ConfirmDialog } from "./confirm-dialog";
import { Select } from "./select";
import { randomUuid } from "./uuid";
import { useModal } from "./use-modal";
import { createPortal } from "./page-portal";
import { ModelConnectionAuth } from "./model-connection-auth";
import { ModelCredentialCleanup } from "./model-credential-cleanup";
import { ConnectionModels } from "./model-connection-models";
import { ModelNetworkSettings, NetworkCredentialsInputs, type NetworkCredentialsHandle, type NetworkSave, type NetworkSaveHandle, type ModelNetworkSecret } from "./model-network-settings";
import type { ModelConnection, ModelConnectionChange, ModelConnectionConfig, ModelConnectionKind,
  ModelConnectionPreflight, ModelCredentialAction, ModelDescriptor, ModelPurpose, ModelPurposeBinding } from "./generated/protocol";
import { abortModelConnection, cleanupModelCredential, deleteModelConnection, loadConnectionModels,
  loadModelConnectionChanges, loadModelConnections, loadModelPurposes, preflightModelConnection,
  recoverModelExecutionLeases, refreshLocalModelCatalog, resumeModelConnection, saveModelConnection, saveModelPurpose, setModelConnectionEnabled } from "./model-connections";
import "./model-connection-settings.css";

const kindNames: Record<ModelConnectionKind, string> = { openai_key: "OpenAI API Key", chatgpt: "ChatGPT", custom_responses: "自定义 Responses", local: "本地模型" };
const purposeNames: Record<ModelPurpose, string> = { foreground: "前台对话", automation: "自动化", review: "自动审核", memory: "后台记忆", page_ai: "页面 AI" };
const defaultConfig = (): ModelConnectionConfig => ({ name: "", kind: "openai_key", baseUrl: null, localRuntime: null,
  authMode: "api_key", enabled: true, network: { mode: "direct", proxyUrl: null, proxySecretRef: null, proxyAuth: "none", serviceAuth: "none", headerNames: [] } });

export function ModelConnectionSettings() {
  const [connections, setConnections] = useState<ModelConnection[]>([]);
  const [changes, setChanges] = useState<ModelConnectionChange[]>([]);
  const [purposes, setPurposes] = useState<ModelPurposeBinding[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [view, setView] = useState<"connections" | "purposes" | "credentials">("connections");
  const [search, setSearch] = useState("");
  const [adding, setAdding] = useState(false);
  const [addingAdvanced, setAddingAdvanced] = useState(false);
  const [editingAdvanced, setEditingAdvanced] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [impact, setImpact] = useState<ModelConnectionPreflight | null>(null);
  const [recoveryNote, setRecoveryNote] = useState("");
  const [catalogWarning, setCatalogWarning] = useState(false);
  const lifecycle = useRef(0);
  const reload = useCallback(async () => {
    const epoch = ++lifecycle.current;
    setLoading(true);
    try {
      const [items, pending, bindings] = await Promise.all([loadModelConnections(), loadModelConnectionChanges(), loadModelPurposes()]);
      if (epoch === lifecycle.current) {
        const available = items.filter(item => !item.deleted);
        setConnections(available);
        setSelected(previous => available.some(item => item.connectionId === previous) ? previous : available[0]?.connectionId ?? null);
        setChanges(pending.filter(item => item.state === "prepared" || item.state === "aborted" && (item.credentialAction === "replace" || item.networkCredentialAction === "replace") && ["pending", "unconfirmed"].includes(item.credentialCleanup)));
        setPurposes(bindings); setLoaded(true); setError(null);
      }
    } catch (reason) { if (epoch === lifecycle.current) { setLoaded(false); setError(aiErrorMessage(reason)); } }
    finally { if (epoch === lifecycle.current) setLoading(false); }
  }, []);
  useEffect(() => { void reload(); return () => { lifecycle.current += 1; }; }, [reload]);
  const perform = async (action: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true); setError(null);
    try {
      const result = await action(); await reload();
      if (result && typeof result === "object" && "catalogRefreshError" in result) setCatalogWarning(Boolean(result.catalogRefreshError));
    } catch (reason) { setError(aiErrorMessage(reason)); }
    finally { setBusy(false); }
  };
  const current = connections.find(item => item.connectionId === selected) ?? null;
  const checkImpact = async (connection: ModelConnection, action: "delete" | "disable") => {
    setBusy(true); setError(null); setRecoveryNote("");
    try { setImpact(await preflightModelConnection(connection.connectionId, action)); }
    catch (reason) { setError(aiErrorMessage(reason)); }
    finally { setBusy(false); }
  };
  const save = (connection:ModelConnection|null) => (config:ModelConnectionConfig, credentialAction:ModelCredentialAction, secret?:string, network?:NetworkSave) => perform(async()=>{
    const result=await saveModelConnection({operationId:randomUuid().replaceAll("-",""),connection,config,credentialAction,secret,network});
    setSelected(result.result.connectionId);setAdding(false);setAddingAdvanced(false);return result;
  });
  const visible=connections.filter(item=>`${item.config.name} ${kindNames[item.config.kind]}`.toLocaleLowerCase().includes(search.toLocaleLowerCase().trim()));
  return <section aria-label="个人模型连接" className="personal-model-settings">
    <nav className="personal-model-tabs" aria-label="模型设置分区">{([['connections','连接'],['purposes','用途'],['credentials','凭据']] as const).map(([value,label])=><button key={value} type="button" aria-pressed={view===value} disabled={busy} onClick={()=>setView(value)}>{label}{value==='credentials'&&changes.length?<span>{changes.length}</span>:null}</button>)}</nav>
    {error&&!adding&&!editingAdvanced?<p role="alert" className="settings-error personal-model-alert">{error}<button type="button" disabled={busy} onClick={()=>void reload()}>刷新</button></p>:null}
    {catalogWarning?<p className="settings-error personal-model-alert" role="alert">更改已保存，模型目录尚未刷新。<button type="button" disabled={busy} onClick={()=>void perform(async()=>{await refreshLocalModelCatalog();setCatalogWarning(false);})}>刷新目录</button></p>:null}
    {view==='connections'?<div className="personal-model-layout">
      <aside className="personal-model-list" aria-label="连接列表">
        <label className="personal-model-search"><SearchIcon/><input aria-label="搜索连接" type="search" placeholder="搜索连接…" value={search} onChange={event=>setSearch(event.target.value)}/></label>
        <div className="personal-model-list-scroll">
          {loading?<p role="status">正在读取…</p>:null}
          {!loading&&loaded&&!connections.length?<p>尚未添加连接</p>:null}
          {!loading&&connections.length&&!visible.length?<p>没有匹配的连接</p>:null}
          {visible.map(item=><button className="personal-model-item" key={item.connectionId} type="button" disabled={busy} aria-label={`${item.config.name} ${kindNames[item.config.kind]} · ${item.config.enabled?'启用':'停用'}`} aria-pressed={item.connectionId===selected} onClick={()=>setSelected(item.connectionId)}>
            <ConnectionMark config={item.config}/><span><strong>{item.config.name}</strong><small>{kindNames[item.config.kind]}</small></span><i className="personal-model-status" data-enabled={item.config.enabled} aria-label={item.config.enabled?'已启用':'已停用'}/>
          </button>)}
        </div>
        <button className="personal-model-add" type="button" disabled={busy||loading||!loaded} onClick={()=>{setError(null);setAddingAdvanced(false);setAdding(true);}}><PlusIcon/>添加连接</button>
      </aside>
      <main className="personal-model-detail">
        {current?<>
          <ConnectionEditor key={`editor:${current.connectionId}:${current.configRevision}`} connection={current} busy={busy||loading} unavailable={!loaded} error={error} onAdvancedChange={setEditingAdvanced} onSave={save(current)} onToggle={()=>{if(current.config.enabled)void checkImpact(current,'disable');else void perform(()=>setModelConnectionEnabled(current,true));}} onDelete={()=>void checkImpact(current,'delete')}/>
          {current.config.kind==='chatgpt'?<ModelConnectionAuth key={current.connectionId} connectionId={current.connectionId} disabled={busy||!loaded||!current.config.enabled} onChanged={reload}/>:null}
          <ConnectionModels key={`models:${current.connectionId}:${current.configRevision}`} connection={current} disabled={busy||!loaded} onChanged={reload}/>
        </>:!loading&&loaded?<div className="personal-model-empty"><ConnectionIcon/><h2>添加第一个连接</h2><p>连接模型后即可开始对话。</p><button className="primary" type="button" onClick={()=>setAdding(true)}><PlusIcon/>添加连接</button></div>:null}
      </main>
    </div>:null}
    {view==='purposes'?<div className="personal-model-section-scroll"><PurposeSettings connections={connections} bindings={purposes} disabled={busy||loading||!loaded} onSave={(binding,revision)=>perform(()=>saveModelPurpose(binding,revision))}/></div>:null}
    {view==='credentials'?<div className="personal-model-section-scroll">
      {changes.length?<section className="personal-model-pending" aria-label="未完成的连接操作"><h2>未完成的操作</h2>{changes.map(change=>change.state==='aborted'?<div key={change.operationId}><strong>{change.connection.config.name}</strong><p>连接已取消，凭据清理待确认。</p>{localPendingCleanup(change)?<button type="button" disabled={busy} onClick={()=>void perform(()=>cleanupModelCredential(change.operationId))}>清理残留凭据</button>:<p>旧系统凭据需手工清理，尚未完成。</p>}</div>:<PendingChange key={change.operationId} change={change} busy={busy||loading||!loaded} onContinue={(secret,networkSecret)=>perform(async()=>{const result=await resumeModelConnection(change.operationId,secret,networkSecret);setSelected(result.result.connectionId);return result;})} onAbort={()=>perform(()=>abortModelConnection(change.operationId))}/>)}</section>:null}
      <ModelCredentialCleanup connections={connections} disabled={busy||loading||!loaded} onChanged={reload}/>
    </div>:null}
    {adding?<ModelSettingsDialog title="添加连接" active={!addingAdvanced} busy={busy} onClose={()=>setAdding(false)}><ConnectionEditor connection={null} busy={busy} error={error} onSave={save(null)} onAdvancedChange={setAddingAdvanced} onToggle={()=>{}} onDelete={()=>{}} onCancel={()=>setAdding(false)}/>{error&&!addingAdvanced?<p role="alert" className="settings-error">{error}</p>:null}</ModelSettingsDialog>:null}
    {impact?<ConfirmDialog title={impact.action==='delete'?'删除连接？':'停用连接？'} description={impact.activeLeaseIds.length?<><p>仍有 {impact.activeLeaseIds.length} 个任务占用此连接，请先停止或等待完成。</p><button type="button" disabled={busy} onClick={()=>void perform(async()=>{const result=await recoverModelExecutionLeases(impact.connectionId);const released=result.leases.filter(lease=>lease.state==='released').length;setRecoveryNote(`已核对：${released} 项已释放，${result.leases.length-released} 项仍存活或无法确认。`);setImpact(await preflightModelConnection(impact.connectionId,impact.action));})}>核对已退出任务</button>{recoveryNote?<p role="status">{recoveryNote}</p>:null}</>:`关联 ${impact.references.length} 项设置；历史会保留，关联用途不会自动换用其他连接。`} danger busy={busy} confirmDisabled={impact.activeLeaseIds.length>0} confirmLabel={impact.action==='delete'?'删除':'停用'} onCancel={()=>setImpact(null)} onConfirm={()=>void perform(async()=>{if(impact.activeLeaseIds.length)return;const connection=connections.find(item=>item.connectionId===impact.connectionId);if(!connection)throw new Error('model_connection_not_found');const result=impact.action==='delete'?await deleteModelConnection(impact):await setModelConnectionEnabled(connection,false,impact);if(impact.action==='delete')setSelected(null);setImpact(null);return result;})}/>:null}
  </section>;
}

function ConnectionEditor(props:{connection:ModelConnection|null;busy:boolean;unavailable?:boolean;error?:string|null;onSave:(config:ModelConnectionConfig,action:ModelCredentialAction,secret?:string,network?:NetworkSave)=>Promise<void>;onToggle:()=>void;onDelete:()=>void;onCancel?:()=>void;onAdvancedChange?:(open:boolean)=>void}) {
  useEffect(()=>()=>props.onAdvancedChange?.(false),[props.onAdvancedChange]);
  const disabled=props.busy||Boolean(props.unavailable);
  const [config,setConfig]=useState<ModelConnectionConfig>(()=>props.connection?.config??defaultConfig());
  const localKey=hasWorkbenchCredentialLocation(props.connection?.secretRef,"model");
  const legacyKey=Boolean(props.connection?.secretRef)&&!localKey;
  const [replaceKey,setReplaceKey]=useState(!localKey);
  const replacingKey=replaceKey||!localKey;
  const [advanced,setAdvanced]=useState(false);
  const baseline=useRef(config);
  const form=useRef<HTMLFormElement>(null);
  const advancedFields=useRef<HTMLDivElement>(null);
  const secretInput=useRef<HTMLInputElement>(null);
  const networkInput=useRef<NetworkSaveHandle>(null);
  const setKind=(kind:ModelConnectionKind)=>{setConfig({...config,kind,baseUrl:null,localRuntime:kind==='local'?'ollama':null,authMode:kind==='chatgpt'?'runtime_managed_chatgpt':kind==='local'?'none':'api_key',network:{...config.network,serviceAuth:'none',headerNames:kind==='chatgpt'?[]:config.network.headerNames}});setReplaceKey(true);if(secretInput.current)secretInput.current.value='';};
  const submit=()=>{
    if(disabled||!form.current?.reportValidity())return;
    if(advanced&&[...(advancedFields.current?.querySelectorAll<HTMLInputElement|HTMLSelectElement|HTMLTextAreaElement>('input,select,textarea')??[])].some(input=>!input.reportValidity()))return;
    const value=secretInput.current?.value;if(secretInput.current)secretInput.current.value='';
    const action:ModelCredentialAction=config.authMode==='api_key'?replacingKey?'replace':'keep':props.connection?.config.kind==='chatgpt'&&config.kind==='chatgpt'?'keep':'clear';
    const network=networkInput.current?.take();void props.onSave(config,action,action==='replace'?value:undefined,network);
  };
  const closeAdvanced=()=>{setConfig(baseline.current);setAdvanced(false);props.onAdvancedChange?.(false);};
  const openAdvanced=()=>{if(advanced||disabled)return;baseline.current=structuredClone(config);setAdvanced(true);props.onAdvancedChange?.(true);};
  return <form ref={form} className={`personal-model-editor${props.connection?'':' personal-model-new'}`} aria-label={props.connection?'编辑连接':'添加连接'} onSubmit={event=>{event.preventDefault();submit();}}>
    {props.connection?<header className="personal-model-connection-heading"><div><ConnectionMark config={config}/><h2>{props.connection.config.name}</h2><span className="personal-model-kind">{kindNames[config.kind]}</span></div><button className="personal-model-switch" type="button" role="switch" aria-label="启用连接" aria-checked={props.connection.config.enabled} disabled={disabled} onClick={props.onToggle}><span/></button></header>:null}
    <fieldset disabled={disabled}>
      {!props.connection?<><label>名称<input autoFocus value={config.name} required maxLength={120} placeholder="连接名称" onChange={event=>setConfig({...config,name:event.target.value})}/></label><label>类型<Select ariaLabel="类型" disabled={disabled} value={config.kind} onChange={value=>setKind(value as ModelConnectionKind)} options={Object.entries(kindNames).map(([value,label])=>({value,label}))}/></label></>:null}
      {config.kind==='local'?<label>服务<Select ariaLabel="服务" disabled={disabled||Boolean(props.connection)} value={config.localRuntime??'ollama'} onChange={value=>setConfig({...config,localRuntime:value as 'ollama'|'lmstudio'})} options={[{value:'ollama',label:'Ollama'},{value:'lmstudio',label:'LM Studio'}]}/></label>:null}
      {(config.kind==='custom_responses'||config.kind==='local')&&!props.connection?<label>认证<Select ariaLabel="认证" disabled={disabled} value={config.authMode} onChange={value=>setConfig({...config,authMode:value as 'none'|'api_key',network:{...config.network,serviceAuth:'none'}})} options={[{value:'none',label:'无认证'},{value:'api_key',label:'API Key'}]}/></label>:null}
      {config.authMode==='api_key'?<div className="personal-model-field-wide"><div className="personal-model-label-row"><label htmlFor={`key-${props.connection?.connectionId??'new'}`}>API 密钥</label>{localKey?<button type="button" className="personal-model-text-action" disabled={disabled} onClick={()=>{setReplaceKey(value=>!value);if(secretInput.current)secretInput.current.value='';}}>{replaceKey?'取消更换':'更换密钥'}</button>:null}</div>{legacyKey?<p className="settings-footnote">需重新保存到工作台</p>:null}{replacingKey?<input id={`key-${props.connection?.connectionId??'new'}`} ref={secretInput} aria-label="API 密钥" type="password" autoComplete="new-password" spellCheck={false} maxLength={16_384} required placeholder="输入 API 密钥"/>:<div className="personal-model-stored-key"><KeyIcon/><span>本地凭据已登记</span><small>工作台本地加密</small></div>}</div>:null}
      {config.kind==='custom_responses'||config.kind==='local'?<label className="personal-model-field-wide">Responses 基础地址<input aria-label="Responses 基础地址" aria-describedby={config.kind==='custom_responses'?`endpoint-help-${props.connection?.connectionId??'new'}`:undefined} type="url" required value={config.baseUrl??''} placeholder="包含 /v1 的服务地址" autoComplete="off" onChange={event=>{setConfig({...config,baseUrl:event.target.value});if(config.authMode==='api_key'&&event.target.value!==props.connection?.config.baseUrl)setReplaceKey(true);}}/>{config.kind==='custom_responses'?<small id={`endpoint-help-${props.connection?.connectionId??'new'}`}>基础地址不含 /responses，例如 https://api.example.com/v1</small>:null}</label>:config.kind==='openai_key'?<label className="personal-model-field-wide">API 地址<input aria-label="API 地址" readOnly value="https://api.openai.com/v1"/></label>:null}
      {config.kind==='chatgpt'?<p className="settings-footnote">使用官方设备授权，各隔离环境分别登录。</p>:null}
      <footer><button className="personal-model-config-action" type="button" onClick={openAdvanced}><SettingsIcon/>连接设置</button>{props.connection?<small>修订 {props.connection.configRevision}</small>:null}<span/>{props.onCancel?<button type="button" onClick={props.onCancel}>取消</button>:null}<button className="primary" type="submit">{props.busy?'处理中…':'保存'}</button></footer>
    </fieldset>
    {advanced?<ModelSettingsDialog title="连接设置" drawer busy={props.busy} onClose={closeAdvanced}><div ref={advancedFields} className="personal-model-advanced"><fieldset disabled={disabled}>
      <label>名称<input autoFocus value={config.name} required maxLength={120} onChange={event=>setConfig({...config,name:event.target.value})}/></label>
      <label>类型<input className="personal-model-readonly" value={kindNames[config.kind]} readOnly/></label>
      <label>认证<input className="personal-model-readonly" value={config.authMode==='api_key'?'API Key':config.authMode==='none'?'无认证':'官方设备授权'} readOnly/></label>
      <h3>网络</h3><label>网络<Select ariaLabel="网络" disabled={disabled} value={config.network.mode} onChange={value=>setConfig({...config,network:{...config.network,mode:value as 'direct'|'system_proxy'|'custom_proxy',proxyUrl:null,proxyAuth:value==='direct'?'none':config.network.proxyAuth}})} options={[{value:'direct',label:'直接连接'},{value:'system_proxy',label:'系统代理'},{value:'custom_proxy',label:'指定代理'}]}/></label>
      {config.network.mode==='custom_proxy'?<label>代理地址<input type="url" required autoComplete="off" value={config.network.proxyUrl??''} onChange={event=>setConfig({...config,network:{...config.network,proxyUrl:event.target.value}})}/></label>:null}
      <ModelNetworkSettings key={config.kind} config={config} original={props.connection?.config??null} disabled={disabled} onChange={setConfig} inputRef={networkInput}/>
      <label className="personal-model-field-wide">CA 证书（PEM）<textarea autoComplete="off" spellCheck={false} maxLength={131_072} rows={4} value={config.network.caCertificatePem??''} placeholder="公开 PEM 证书" onChange={event=>setConfig({...config,network:{...config.network,caCertificatePem:event.target.value||undefined}})}/></label>
      <p className="settings-footnote">仅信任此连接的证书来源，不接受私钥。</p>
    </fieldset>{props.error?<p role="alert" className="settings-error">{props.error}</p>:null}</div><div className="personal-model-dialog-actions">{props.connection?<button type="button" className="danger personal-model-delete" disabled={disabled} onClick={()=>{closeAdvanced();props.onDelete();}}>删除连接</button>:null}<button type="button" disabled={props.busy} onClick={closeAdvanced}>取消</button><button type="button" className="primary" disabled={disabled} onClick={submit}>保存</button></div></ModelSettingsDialog>:null}
  </form>;
}

function ModelSettingsDialog(props:{title:string;busy:boolean;drawer?:boolean;active?:boolean;onClose:()=>void;children:ReactNode}) {
  const ref=useModal<HTMLDivElement>({onClose:props.onClose,busy:props.busy,active:props.active});
  return createPortal(<div ref={ref} className={`personal-model-overlay${props.drawer?' personal-model-overlay-drawer':''}`} role="dialog" aria-modal="true" aria-label={props.title} onMouseDown={event=>{if(event.target===event.currentTarget&&!props.busy)props.onClose();}}><section className="personal-model-dialog"><header><h2>{props.title}</h2><button type="button" aria-label="关闭" disabled={props.busy} onClick={props.onClose}><CloseIcon/></button></header><div className="personal-model-dialog-content">{props.children}</div></section></div>,document.body);
}
function ConnectionMark({config}:{config:ModelConnectionConfig}) {
  let router=false;try{router=config.kind==='custom_responses'&&new URL(config.baseUrl??'').hostname==='openrouter.ai';}catch{router=false;}
  return <span className="personal-model-mark" data-kind={router?'router':config.kind}><svg aria-hidden="true" viewBox="0 0 24 24">{config.kind==='local'?<><rect x="4" y="4" width="16" height="6" rx="2"/><rect x="4" y="14" width="16" height="6" rx="2"/><path d="M8 7h.01M8 17h.01"/></>:router?<><path d="M4 12h11m-4-5 5 5-5 5M17 7h3v10h-3"/><circle cx="5" cy="12" r="2"/></>:config.kind==='chatgpt'?<><path d="m12 3 8 5v8l-8 5-8-5V8z"/><path d="m12 7 4 3v4l-4 3-4-3v-4z"/></>:<><circle cx="12" cy="12" r="8"/><path d="M4 12h16M12 4c5 5 5 11 0 16-5-5-5-11 0-16"/></>}</svg></span>;
}
function SearchIcon(){return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 4 4"/></svg>;}
function SettingsIcon(){return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m12 3 2 3 3 .5.5 3 2 2-2 2-.5 3-3 .5-2 3-2-3-3-.5-.5-3-2-2 2-2 .5-3 3-.5z"/><circle cx="12" cy="12" r="3"/></svg>;}
function KeyIcon(){return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="8" cy="8" r="4"/><path d="m11 11 9 9m-4-4 2-2m-5-1 2-2"/></svg>;}
function CloseIcon(){return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m6 6 12 12M6 18 18 6"/></svg>;}

function localPendingCleanup(change: ModelConnectionChange): boolean {
  return (change.credentialAction !== "replace" || hasWorkbenchCredentialLocation(change.connection.secretRef, "model"))
    && (change.networkCredentialAction !== "replace" || hasWorkbenchCredentialLocation(change.connection.config.network.proxySecretRef, "model-network"));
}

function PendingChange(props: { change: ModelConnectionChange; busy: boolean; onContinue: (secret?: string, networkSecret?: ModelNetworkSecret) => Promise<void>; onAbort: () => Promise<void> }) {
  const input = useRef<HTMLInputElement>(null);
  const network = useRef<NetworkCredentialsHandle>(null);
  const legacyKeep = props.change.credentialAction === "keep" && props.change.connection.config.authMode === "api_key"
    && !hasWorkbenchCredentialLocation(props.change.connection.secretRef, "model")
    || props.change.networkCredentialAction === "keep" && Boolean(props.change.connection.config.network.proxySecretRef)
    && !hasWorkbenchCredentialLocation(props.change.connection.config.network.proxySecretRef, "model-network");
  return <form onSubmit={(event) => { event.preventDefault(); if (props.busy || legacyKeep) return; const secret = input.current?.value; if (input.current) input.current.value = ""; const networkSecret = network.current?.take(); void props.onContinue(secret, networkSecret); }}>
    <strong>{props.change.connection.config.name}</strong><small>尚未提交 · {props.change.operationId}</small>
    {props.change.credentialAction === "replace" ? <label>重新输入密钥<input aria-label={`继续${props.change.connection.config.name}的密钥`} ref={input} type="password" autoComplete="new-password" required disabled={props.busy}/></label> : null}
    {props.change.networkCredentialAction === "replace" ? <NetworkCredentialsInputs network={props.change.connection.config.network} ref={network} disabled={props.busy}/> : null}
    {legacyKeep?<p>需重新保存到工作台。请取消此操作后重新输入凭据。</p>:null}
    <div><button type="submit" disabled={props.busy||legacyKeep}>继续</button><button type="button" disabled={props.busy} onClick={() => void props.onAbort()}>取消操作</button></div>
  </form>;
}

function PurposeSettings(props: { connections: ModelConnection[]; bindings: ModelPurposeBinding[]; disabled: boolean;
  onSave: (binding: ModelPurposeBinding, expectedRevision: number | null) => Promise<void> }) {
  return <section className="personal-model-purposes" aria-label="模型用途"><h2>用途</h2>{(Object.keys(purposeNames) as ModelPurpose[]).map((purpose) => {
    const binding = props.bindings.find((item) => item.purpose === purpose && item.scopeId === null);
    return <PurposeEditor key={`${purpose}:${binding?.revision ?? 0}`} purpose={purpose} binding={binding ?? null} connections={props.connections} disabled={props.disabled} onSave={props.onSave}/>;
  })}</section>;
}

function PurposeEditor(props: { purpose: ModelPurpose; binding: ModelPurposeBinding | null; connections: ModelConnection[]; disabled: boolean;
  onSave: (binding: ModelPurposeBinding, expectedRevision: number | null) => Promise<void> }) {
  const [connectionId, setConnectionId] = useState(props.binding?.selection?.connectionId ?? "");
  const [modelId, setModelId] = useState(props.binding?.selection?.modelId ?? "");
  const [models, setModels] = useState<ModelDescriptor[]>([]);
  const [enabled, setEnabled] = useState(props.binding?.enabled ?? false);
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const needsConsent = props.purpose !== "foreground";
  const historyScope = { foreground: "本次对话", review: "本次执行的审批请求、工具参数与必要对话上下文",
    memory: "当前资料中已允许用于记忆的历史对话", automation: "启用计划的任务输入、指定上下文及允许承接的历史",
    page_ai: "当前应用获准提交的页面上下文与本次页面对话" }[props.purpose];
  const selected = props.connections.find((item) => item.connectionId === connectionId);
  const purposeAuthGeneration = selected?.config.kind === "chatgpt"
    ? selected.authScopes.find((scope) => scope.runtimeKind === (props.purpose === "page_ai" ? "application" : "workbench") && scope.state === "active")?.authGeneration
    : selected?.authGeneration;
  useEffect(() => { setConfirmed(false); }, [connectionId, selected?.configRevision, purposeAuthGeneration]);
  useEffect(() => {
    let cancelled = false; setModels([]);
    if (connectionId) void loadConnectionModels(connectionId).then((items) => { if (!cancelled) { setModels(items); setError(null); } }).catch((reason) => { if (!cancelled) setError(aiErrorMessage(reason)); });
    return () => { cancelled = true; };
  }, [connectionId]);
  return <form className="personal-model-purpose" aria-label={purposeNames[props.purpose]} onSubmit={(event) => {
    event.preventDefault();
    if (enabled && needsConsent && !confirmed) return;
    void props.onSave({ bindingId: props.binding?.bindingId ?? `profile:${props.purpose}`, purpose: props.purpose,
      scopeId: null, policy: "explicit", selection: connectionId && modelId ? { connectionId, modelId, reasoningEffort: null } : null,
      enabled, consent: enabled && needsConsent ? { serviceName: selected?.config.name ?? "", historyScope, mayIncurCost: true, confirmedAt: Math.floor(Date.now() / 1000) } : null,
      revision: props.binding?.revision ?? 0 }, props.binding?.revision ?? null);
  }}><h3>{purposeNames[props.purpose]}</h3><fieldset disabled={props.disabled}>
    <label>连接<Select disabled={props.disabled} ariaLabel="连接" value={connectionId} onChange={(value) => { setConnectionId(value); setModelId(""); setConfirmed(false); }} options={[{ value: "", label: "未选择" }, ...props.connections.map((item) => ({ value: item.connectionId, label: `${item.config.name}${item.config.enabled ? "" : "（停用）"}`, disabled: !item.config.enabled }))]}/></label>
    <label>模型<Select disabled={props.disabled} ariaLabel="模型" value={modelId} onChange={(value) => { setModelId(value); setConfirmed(false); }} options={[{ value: "", label: "未选择" }, ...(modelId && !models.some((item) => item.modelId === modelId) ? [{ value: modelId, label: `${modelId}（待核对）`, disabled: true }] : []), ...models.map((item) => ({ value: item.modelId, label: item.displayName }))]}/></label>
    <label className="personal-model-check"><input type="checkbox" checked={enabled} onChange={(event) => { setEnabled(event.target.checked); setConfirmed(false); }}/>启用</label>
    {needsConsent && enabled ? <label className="personal-model-consent"><input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)}/><span>使用“{selected?.config.name ?? "未选择"}”。会发送{historyScope}。可能产生服务费用。</span></label> : null}
    <button type="submit" disabled={enabled && (!connectionId || !modelId || (needsConsent && !confirmed))}>保存用途</button>
  </fieldset>{error ? <p className="settings-error" role="alert">{error}</p> : null}</form>;
}

function PlusIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>; }
function ConnectionIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M8 4v5m8-5v5M6 9h12v3a6 6 0 0 1-6 6v3m0-3a6 6 0 0 1-6-6"/></svg>; }
