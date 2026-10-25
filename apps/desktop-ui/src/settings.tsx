import { lazy, useEffect, useState } from "react";
import type { WorkspaceSummary } from "./generated/protocol";
import type { AiProjectOption } from "./ai-workbench";
import type { MessageSink } from "./app-message";
import { aiErrorMessage, loadAiMemorySettings, saveAiMemorySettings, type AiMemorySettings } from "./ai-runtime";
import {PersonalMemoryReset} from "./personal-memory-reset";
import { ArchiveIcon, RefreshIcon, RunsIcon, SettingsIcon } from "./icons";
import { DesktopUpdateSettingsPage } from "./desktop-update-settings";
import { ConfirmDialog } from "./confirm-dialog";
import { MemoryErrorNotice } from "./memory-error";
import { TestDiagnosticsSettings } from "./test-diagnostics";
import { RetainedPage } from "./retained-page";
import { usePageActive } from "./page-visibility";
import { MacosDiskAccessSettings } from "./macos-disk-access";
import "./settings.css";

const AiSettingsPage = lazy(() => import("./ai-settings").then((module) => ({ default: module.AiSettingsPage })));
const PersonalUsageSettings = lazy(() => import("./personal-usage").then((module) => ({ default: module.PersonalUsageSettings })));
const McpConnectionSettings = lazy(() => import("./mcp-connections").then((module) => ({ default: module.McpConnectionSettings })));
const PersonalNotificationSettingsPanel = lazy(() => import("./personal-notification-settings").then((module) => ({ default: module.PersonalNotificationSettingsPanel })));
const PersonalDataSettings = lazy(() => import("./personal-data").then((module) => ({ default: module.PersonalDataSettings })));
const ArchivedConversationsPage = lazy(() => import("./archived-conversations").then((module) => ({ default: module.ArchivedConversationsPage })));
export type SettingsSection = "memory" | "agents" | "archived" | "updates" | "diagnostics" | "disk-access" | "usage" | "tools" | "data" | "notifications";

function NotificationIcon() {
  return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M10 21h4"/></svg>;
}

function MemoryIcon() {
  return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M12 5a4 4 0 0 0-7-1 4 4 0 0 0-2 7 4 4 0 0 0 2 7 4 4 0 0 0 7 1V5Zm0 0a4 4 0 0 1 7-1 4 4 0 0 1 2 7 4 4 0 0 1-2 7 4 4 0 0 1-7 1M7 8c3 0 5 2 5 4M17 8c-3 0-5 2-5 4M7 16c3 0 5-2 5-4M17 16c-3 0-5-2-5-4"/></svg>;
}

export function SettingsPage(props: {
  section: SettingsSection;
  onSectionChange: (section: SettingsSection) => void;
  workspace: WorkspaceSummary | null;
  hostPlatform: string | null;
  onMessage: MessageSink;
  projects: AiProjectOption[];
  archiveOpenIntent?: { conversationId: string; token: number } | null;
  profileId?: string;
}) {
  const sections = [
    { id: "memory" as const, label: "记忆", icon: <MemoryIcon/> },
    { id: "agents" as const, label: "模型设置", icon: <SettingsIcon/> },
    { id: "tools" as const, label: "工具连接", icon: <SettingsIcon/> },
    ...(props.hostPlatform === "macos" ? [{ id: "disk-access" as const, label: "磁盘访问", icon: <SettingsIcon/> }] : []),
    { id: "archived" as const, label: "已归档会话", icon: <ArchiveIcon/> },
    { id: "updates" as const, label: "软件更新", icon: <RefreshIcon/> },
    { id: "diagnostics" as const, label: "诊断", icon: <RunsIcon/> },
    { id: "usage" as const, label: "使用量", icon: <RunsIcon/> },
    { id: "data" as const, label: "数据", icon: <ArchiveIcon/> },
    { id: "notifications" as const, label: "通知", icon: <NotificationIcon/> },
  ];
  return <div className="workbench-settings">
    <aside className="settings-sidebar">
      <h1>设置</h1>
      <nav aria-label="设置导航">{sections.map((section) => <button key={section.id} type="button"
        aria-current={section.id === props.section ? "page" : undefined}
        onClick={() => props.onSectionChange(section.id)}>{section.icon}<span>{section.label}</span></button>)}</nav>
    </aside>
    <div className="settings-content">
      <RetainedPage settings active={props.section === "memory"}><MemorySettingsPage onOpenModels={() => props.onSectionChange("agents")}/></RetainedPage>
      {props.hostPlatform === "macos" ? <RetainedPage settings active={props.section === "disk-access"}><MacosDiskAccessSettings/></RetainedPage> : null}
      {props.hostPlatform !== "macos" && props.section === "disk-access" ? <section className="disk-access-settings-page"><p>{props.hostPlatform ? "磁盘访问仅适用于 macOS。" : "正在读取系统…"}</p></section> : null}
      <RetainedPage settings active={props.section === "agents"}><AiSettingsPage workspace={props.workspace} hostPlatform={props.hostPlatform} onMessage={props.onMessage}/></RetainedPage>
      <RetainedPage settings active={props.section === "updates"}><DesktopUpdateSettingsPage/></RetainedPage>
      <RetainedPage settings active={props.section === "diagnostics"}><TestDiagnosticsSettings/></RetainedPage>
      <RetainedPage settings active={props.section === "usage"}><PersonalUsageSettings/></RetainedPage>
      <RetainedPage settings active={props.section === "tools"}><McpConnectionSettings hostPlatform={props.hostPlatform}/></RetainedPage>
      <RetainedPage settings active={props.section === "data"}><PersonalDataSettings profileId={props.profileId}/></RetainedPage>
      <RetainedPage settings active={props.section === "notifications"}><PersonalNotificationSettingsPanel profileId={props.profileId}/></RetainedPage>
      <RetainedPage settings active={props.section === "archived"}><ArchivedConversationsPage projects={props.projects} onMessage={props.onMessage} openIntent={props.archiveOpenIntent}/></RetainedPage>
    </div>
  </div>;
}

export function MemorySettingsPage(props: { onOpenModels: () => void }) {
  const pageActive = usePageActive();
  const [settings, setSettings] = useState<AiMemorySettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [saveUnknown,setSaveUnknown] = useState(false);
  const [enablePreview,setEnablePreview] = useState<{enabled:boolean;fromToolChats:boolean;service:NonNullable<AiMemorySettings["service"]>}|null>(null);
  const [notice, setNotice] = useState("");
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    void loadAiMemorySettings().then((value) => { if (!cancelled) {setSettings(value);setSaveUnknown(false);} })
      .catch((reason) => { if (!cancelled) {setSaveUnknown(true);setError(aiErrorMessage(reason));} })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [retry]);
  useEffect(() => {
    if (!settings || busy || !pageActive) return;
    let cancelled = false;
    const timer = window.setInterval(() => {
      void loadAiMemorySettings().then((value) => { if (!cancelled) setSettings(value); })
        .catch((reason) => { if (!cancelled) setError(aiErrorMessage(reason)); });
    }, 3000);
    return () => { cancelled = true; window.clearInterval(timer); };
  }, [settings?.running, busy, pageActive]);
  const save = async (change: Partial<Pick<AiMemorySettings, "enabled" | "fromToolChats">>, confirmation?:{consentDigest:string;fromToolChats:boolean;confirmedAt:number}) => {
    if (!settings || busy || saveUnknown) return;
    const next={enabled:settings.enabled,fromToolChats:settings.fromToolChats,...change};
    if(next.enabled&&(!settings.enabled||(!settings.fromToolChats&&next.fromToolChats))&&!confirmation) {
      if(!settings.service){setError(aiErrorMessage(settings.serviceError??"model_memory_purpose_not_enabled"));return;}
      setEnablePreview({...next,service:settings.service});return;
    }
    setBusy(true); setError(null); setNotice("");
    try {
      setSettings(await saveAiMemorySettings({...next,...(confirmation?{confirmation}:{})}));
      setEnablePreview(null);
      setNotice("已保存，下次运行生效");
    } catch (reason) { setEnablePreview(null);setSaveUnknown(true);setError(aiErrorMessage(reason)); }
    finally { setBusy(false); }
  };
  return <section aria-label="记忆设置" className="memory-settings-page">
    <header><h1>记忆</h1><span className="settings-local-badge"><svg aria-hidden="true" viewBox="0 0 24 24"><rect x="4" y="4" width="16" height="12" rx="2"/><path d="M8 20h8M12 16v4"/></svg>本机</span></header>
    <div className="settings-memory-status" role="status"><span>工作台 </span><span>{loading ? "正在读取…" : settings ? ({ available: "已有记忆", empty: "尚无记忆", disabled: "记忆已关闭" })[settings.phase] : ""}</span></div>
    {settings?.lastUpdatedAt != null ? <p className="settings-footnote">记忆文件更新：{new Date(settings.lastUpdatedAt * 1000).toLocaleString()}</p> : null}
    {settings?.lastError ? <MemoryErrorNotice error={settings.lastError}/> : null}
    {settings?.serviceError ? <p role="alert" className="settings-error">{aiErrorMessage(settings.serviceError)}</p> : null}
    <div className="settings-group">
      <div className="settings-row"><div><label id="memory-enabled-label">启用记忆</label><p>记住偏好与工作约定，用于后续对话。</p></div><button aria-labelledby="memory-enabled-label" aria-checked={settings?.enabled ?? false} role="switch" className="settings-switch" type="button" disabled={loading || busy || saveUnknown || !settings} onClick={() => void save({ enabled: !settings?.enabled })}><span/></button></div>
      <div className="settings-row"><div><label id="memory-tools-label">从工具会话中提炼</label><p>包含使用应用工具与网页搜索的对话。</p></div><button aria-labelledby="memory-tools-label" aria-checked={settings?.fromToolChats ?? false} role="switch" className="settings-switch" type="button" disabled={loading || busy || saveUnknown || !settings?.enabled} onClick={() => void save({ fromToolChats: !settings?.fromToolChats })}><span/></button></div>
      <PersonalMemoryReset disabled={loading||busy||saveUnknown||!settings} onBusy={setBusy} onChanged={()=>setRetry(value=>value+1)} onUnconfirmed={()=>setSaveUnknown(true)}/>
    </div>
    <p className="settings-footnote">仅用于当前本地资料，不跨设备同步。</p>
    <section className="memory-model-section"><h2>后台模型</h2><button className="settings-model-link" type="button" onClick={props.onOpenModels}><span><strong>记忆模型</strong><small>{settings?.modelLabel ?? "前往配置用途"}</small></span><svg aria-hidden="true" viewBox="0 0 24 24"><path d="m9 5 7 7-7 7"/></svg></button></section>
    {error ? <p className="settings-error" role="alert">{error}{saveUnknown&&settings?<button type="button" onClick={()=>setRetry(value=>value+1)}>刷新状态</button>:null}{!settings ? <button type="button" onClick={() => setRetry((value) => value + 1)}>重试</button> : null}</p> : null}
    <p aria-live="polite" className="settings-save-notice">{notice}</p>
    {enablePreview?<ConfirmDialog title={settings?.enabled?"扩大记忆范围？":"启用后台记忆？"} confirmLabel="启用" busy={busy} onCancel={()=>setEnablePreview(null)}
      description={<><dl><dt>服务</dt><dd>{enablePreview.service.serviceName}{enablePreview.service.baseUrl?` · ${enablePreview.service.baseUrl}`:""}</dd><dt>连接与模型</dt><dd>{enablePreview.service.connectionName} · {enablePreview.service.modelId}</dd><dt>发送范围</dt><dd>{enablePreview.fromToolChats?enablePreview.service.toolChatsHistoryScope:enablePreview.service.historyScope}</dd></dl><p>{enablePreview.service.mayIncurCost?"可能产生服务费用。":"费用以所选服务为准。"}</p></>}
      onConfirm={()=>void save({enabled:enablePreview.enabled,fromToolChats:enablePreview.fromToolChats},{consentDigest:enablePreview.service.consentDigest,fromToolChats:enablePreview.fromToolChats,confirmedAt:Math.floor(Date.now()/1000)})}/>:null}
  </section>;
}
