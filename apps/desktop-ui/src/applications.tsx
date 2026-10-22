import { useCallback, useEffect, useRef, useState } from "react";
import { PersonalCapabilityEditor } from "./personal-capability-editor";
import { PersonalCapabilityManager } from "./personal-capability-manager";
import { PersonalApplication } from "./personal-application";
import { PersonalWebApplication } from "./personal-web-application";
import { PersonalCapabilityRunner } from "./personal-capability-runner";
import { invoke,listen } from "./host";
import type {
  CapabilityCreationListResult,
  CapabilityCreationPreviewResult, CapabilityCreationProject, PersonalApplicationOpenResult,
} from "./generated/protocol";
import { rpc } from "./app-server-rpc";
import type { MessageSink } from "./app-message";
import { ApplicationTile } from "./application-tile";
import { ApplicationsIcon, CloseIcon, PlusIcon, RefreshIcon } from "./icons";
import { RichMenuItem } from "./rich-menu";
import { SearchField } from "./search-field";
import { describeServerError } from "./server-error";
import { ConfirmDialog } from "./confirm-dialog";
import { PersonalPackageImport } from "./personal-package-import";
import type { PersonalPackageImportTarget } from "./generated/protocol";
import "./applications.css";
import { PageVisibilityContext, usePageActive } from "./page-visibility";

type PersonalTab = {
  applicationId: string;
  name: string;
  creationProjectId: string;
  installed: boolean;
  targetType: string;
  packageDigest: string | null;
  html: string;
  contentDigest: string;
};

export type ApplicationSearchOpenIntent = {
  token: string; kind: "application"; applicationId: string;
  objectType: string; objectId: string; target: string;
};

export function ApplicationsPage(props: {
  active?: boolean; profileId?: string; onMessage: MessageSink; onCreate?: () => void;
  onOpenModels?: () => void;
  openIntent?: ApplicationSearchOpenIntent | null;
  onOpenIntentConsumed?: (token: string) => void;
  onActiveApplicationChange?: (applicationId: string | null) => void;
  browser?: { available: boolean; open: boolean; onToggle: () => void };
}) {
  const applicationSurface = useRef<HTMLElement>(null);
  const parentActive = usePageActive();
  const active = (props.active ?? true) && parentActive;
  const [query, setQuery] = useState("");
  const [projects, setProjects] = useState<CapabilityCreationProject[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [tabs, setTabs] = useState<PersonalTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [editing, setEditing] = useState<CapabilityCreationProject | null>(null);
  const [managing, setManaging] = useState<CapabilityCreationProject | null>(null);
  const [running, setRunning] = useState<CapabilityCreationProject | null>(null);
  const [installing, setInstalling] = useState<CapabilityCreationProject | null>(null);
  const [importing, setImporting] = useState<{ target: PersonalPackageImportTarget; title?: string } | null>(null);
  const [closeToken,setCloseToken]=useState<string|null>(null);
  const generation = useRef(0);
  const openedIntent = useRef<string | null>(null);
  useEffect(()=>{
    let active=true;let dispose:(()=>void)|undefined;
    void listen<{token:string}>("personal-applications-close",message=>{
      if(!active||typeof message?.token!=="string"||!/^[-a-f0-9]{36}$/.test(message.token))return;
      setTabs([]);setActiveTabId(null);setRunning(null);setEditing(null);setCloseToken(message.token);
    }).then(remove=>{if(active)dispose=remove;else remove();}).catch(()=>{if(active)props.onMessage("个人应用关闭确认不可用。","error");});
    return()=>{active=false;dispose?.();};
  },[]);
  useEffect(()=>{
    if(!closeToken||tabs.length||running||editing)return;
    const token=closeToken;setCloseToken(null);
    void invoke("personal_applications_close_ack",{token}).catch(()=>props.onMessage("个人应用关闭状态尚未确认。","error"));
  },[closeToken,tabs.length,running,editing]);

  const load = useCallback(async () => {
    const request = ++generation.current;
    setLoading(true); setError(null);
    try {
      const result = await rpc<CapabilityCreationListResult>("capabilityCreation/list");
      if (request !== generation.current) return false;
      setProjects(result.projects);
      const available = new Set(result.projects.map(project => project.creationProjectId));
      setTabs(current => current.filter(tab => available.has(tab.creationProjectId)));
      setActiveTabId(current => current && !available.has(current.slice("personal:".length)) ? null : current);
      return true;
    } catch (reason) {
      if (request !== generation.current) return false;
      setError(describeServerError(reason));
      return false;
    } finally {
      if (request === generation.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    setProjects(null); setTabs([]); setActiveTabId(null);
    setEditing(null); setManaging(null); setRunning(null); setInstalling(null); setImporting(null);
    openedIntent.current = null;
    setLoading(false); setError(null); setBusyId(null);
    return () => { generation.current += 1; };
  }, [props.profileId]);
  useEffect(() => {
    if (active && projects === null && !loading && !error) void load();
  }, [active, projects, loading, error, load]);
  useEffect(() => {
    props.onActiveApplicationChange?.(active ? activeTabId : null);
  }, [active, activeTabId, props.onActiveApplicationChange]);

  const open = async (project: CapabilityCreationProject) => {
    if (!matchesPersonalPreview(project)) return;
    const applicationId = `personal:${project.creationProjectId}`;
    const existing = tabs.find(tab => tab.applicationId === applicationId);
    if (existing) { setActiveTabId(applicationId); return; }
    setBusyId(applicationId);
    const request = generation.current;
    try {
      const preview = project.status === "installed"
        ? await invoke<PersonalApplicationOpenResult>("personal_application_open", { creationProjectId: project.creationProjectId })
        : await rpc<CapabilityCreationPreviewResult>("capabilityCreation/preview", { creationProjectId: project.creationProjectId });
      if (request !== generation.current) return;
      const tab: PersonalTab = {
        applicationId, name: preview.title, creationProjectId: preview.creationProjectId,
        installed: project.status === "installed", targetType: project.targetType,
        packageDigest: project.packageDigest, html: preview.html, contentDigest: preview.contentDigest,
      };
      setTabs(current => current.some(item => item.applicationId === applicationId) ? current : [...current, tab]);
      setActiveTabId(applicationId);
    } catch (reason) {
      if (request === generation.current) props.onMessage(`打开失败：${describeServerError(reason)}`, "error");
    } finally {
      if (request === generation.current) setBusyId(null);
    }
  };

  const install = async (project: CapabilityCreationProject) => {
    if (project.status !== "packaged" || !project.packageDigest) return;
    setBusyId(`install:${project.creationProjectId}`);
    const request = generation.current;
    try {
      const result = await invoke<{ project: CapabilityCreationProject }>("personal_capability_upgrade", { request: {
        creationProjectId: project.creationProjectId, expectedRevision: project.stateRevision,
        packageDigest: project.packageDigest, confirmedPermissions: project.requestedPermissions,
        clientOperationId: `personal-install-${crypto.randomUUID()}`,
      } });
      if (request !== generation.current) return;
      setProjects(current => current?.map(item => item.creationProjectId === result.project.creationProjectId ? result.project : item) ?? null);
      setInstalling(null);
      props.onMessage(`「${project.title}」已安装。`);
    } catch (reason) {
      if (request !== generation.current) return;
      setInstalling(null);
      props.onMessage(`安装未确认：${describeServerError(reason)}`, "error");
    }
    finally { if (request === generation.current) setBusyId(null); }
  };

  const exportPackage = async (project: CapabilityCreationProject) => {
    setBusyId(`export:${project.creationProjectId}`);
    try {
      const saved = await invoke<boolean>("personal_capability_export", { creationProjectId: project.creationProjectId });
      if (saved) props.onMessage("安装包已导出。", "info");
    } catch (reason) { props.onMessage(`导出失败：${describeServerError(reason)}`, "error"); }
    finally { setBusyId(null); }
  };

  const close = (applicationId: string) => {
    setTabs(current => {
      const index = current.findIndex(tab => tab.applicationId === applicationId);
      const remaining = current.filter(tab => tab.applicationId !== applicationId);
      setActiveTabId(selected => selected === applicationId ? remaining[Math.min(index, remaining.length - 1)]?.applicationId ?? null : selected);
      return remaining;
    });
  };

  const refreshTab = async () => {
    const tab = tabs.find(item => item.applicationId === activeTabId);
    if (!tab) return;
    try {
      const preview = tab.installed
        ? await invoke<PersonalApplicationOpenResult>("personal_application_open", { creationProjectId: tab.creationProjectId })
        : await rpc<CapabilityCreationPreviewResult>("capabilityCreation/preview", { creationProjectId: tab.creationProjectId });
      setTabs(current => current.map(item => item.applicationId === tab.applicationId ? { ...item, html: preview.html, contentDigest: preview.contentDigest } : item));
    } catch (reason) { props.onMessage(`刷新失败：${describeServerError(reason)}`, "error"); }
  };

  useEffect(() => {
    const intent = props.openIntent;
    if (!intent || !projects || openedIntent.current === intent.token) return;
    openedIntent.current = intent.token;
    if (intent.objectType === "personal-page-notification" && intent.target === "existing-document") {
      const tab = tabs.find(item => item.creationProjectId === intent.applicationId && item.installed);
      const frame = [...(applicationSurface.current?.querySelectorAll<HTMLIFrameElement>("iframe") ?? [])].find(item => item.getAttribute("src") === intent.objectId
        && item.closest("[data-personal-project]")?.getAttribute("data-personal-project") === intent.applicationId);
      if (tab && frame) setActiveTabId(tab.applicationId);
      else props.onMessage("通知对应的页面已关闭。", "error");
      props.onOpenIntentConsumed?.(intent.token); return;
    }
    const project = projects.find(item => `personal:${item.creationProjectId}` === intent.applicationId || item.creationProjectId === intent.applicationId);
    if (project && matchesPersonalPreview(project)) void open(project);
    else props.onMessage("应用不可用。", "error");
    props.onOpenIntentConsumed?.(intent.token);
  }, [props.openIntent, projects, tabs]);

  const changed = (project: CapabilityCreationProject, closeRunning: boolean) => {
    setProjects(current => current?.map(item => item.creationProjectId === project.creationProjectId ? project : item) ?? null);
    if (closeRunning) {
      close(`personal:${project.creationProjectId}`);
      setRunning(current => current?.creationProjectId === project.creationProjectId ? null : current);
    }
  };

  return <PageVisibilityContext.Provider value={active}><section className="applications-page" ref={applicationSurface} hidden={!active}>
    {tabs.length ? <section className={`application-tabs-shell${activeTabId ? " active" : ""}`}>
      <nav aria-label="已打开应用" className="application-tabs">
        <button aria-current={activeTabId === null ? "page" : undefined} className="application-catalog-tab" onClick={() => { setActiveTabId(null); void load(); }} type="button"><ApplicationsIcon/><span>应用</span></button>
        {tabs.map(tab => <div className={activeTabId === tab.applicationId ? "active" : ""} key={tab.applicationId}>
          <button aria-current={activeTabId === tab.applicationId ? "page" : undefined} onClick={() => setActiveTabId(tab.applicationId)} type="button"><span>{tab.name}</span></button>
          <button aria-label={`关闭${tab.name}`} onClick={() => close(tab.applicationId)} title="关闭" type="button"><CloseIcon/></button>
        </div>)}
        {activeTabId && props.browser?.available ? <button type="button" aria-pressed={props.browser.open} onClick={props.browser.onToggle}>浏览器</button> : null}
        <button aria-label="刷新当前应用" className="application-tab-reload" disabled={!activeTabId} onClick={() => void refreshTab()} title="刷新" type="button"><RefreshIcon/></button>
      </nav>
      <div aria-label="应用内容" className="application-webview-host" hidden={!activeTabId}>
        {tabs.map(tab => tab.installed && tab.targetType === "web_mcp_application" && tab.packageDigest
          ? <PersonalApplication key={`${tab.applicationId}:${tab.contentDigest}`} projectId={tab.creationProjectId} packageDigest={tab.packageDigest} html={tab.html} title={tab.name} active={active && activeTabId === tab.applicationId} onOpenModels={props.onOpenModels}/>
          : tab.installed && tab.targetType === "web_application" && tab.packageDigest
            ? <PersonalWebApplication key={`${tab.applicationId}:${tab.contentDigest}`} projectId={tab.creationProjectId} packageDigest={tab.packageDigest} html={tab.html} title={tab.name} active={active && activeTabId === tab.applicationId} onOpenModels={props.onOpenModels}/>
            : <iframe className="personal-capability-preview" hidden={activeTabId !== tab.applicationId} key={`${tab.applicationId}:${tab.contentDigest}`} sandbox="" srcDoc={securedPersonalPreview(tab.html)} title={tab.name}/>)}
      </div>
    </section> : null}
    {!activeTabId ? <div className="applications-catalog">
      <header className="applications-hero"><h2>应用</h2><div className="applications-header-actions">
        <button className="applications-create" disabled={busyId !== null} onClick={() => setImporting({ target: { mode: "new" } })} type="button"><PackageImportIcon/><span>导入</span></button>
        {props.onCreate ? <button className="applications-create" onClick={props.onCreate} type="button"><PlusIcon/><span>创建</span></button> : null}
        <button aria-label="刷新应用" disabled={loading || busyId !== null} onClick={() => void load()} title="刷新" type="button"><RefreshIcon/></button>
      </div></header>
      <div className="applications-toolbar"><SearchField ariaLabel="搜索应用" clearable onChange={setQuery} placeholder="搜索应用" value={query}/></div>
      <PersonalProjects onEdit={setEditing} onManage={setManaging} onRun={setRunning} onExport={project => void exportPackage(project)} busyId={busyId}
        error={error} loading={loading} onInstall={setInstalling} onPreview={project => void open(project)} onRetry={() => void load()} projects={projects ?? []} query={query}/>
    </div> : null}
    {editing ? <PersonalCapabilityEditor projectId={editing.creationProjectId} onClose={() => setEditing(null)} onChanged={changed}/> : null}
    {managing ? <PersonalCapabilityManager projectId={managing.creationProjectId} onClose={() => setManaging(null)} onChanged={({ project, enabled },versionChanged) => changed(project, !enabled||!!versionChanged)} onImportUpgrade={project => {
      setManaging(null); setImporting({ target: { mode: "upgrade", creationProjectId: project.creationProjectId, expectedRevision: project.stateRevision }, title: project.title });
    }}/> : null}
    {running ? <PersonalCapabilityRunner projectId={running.creationProjectId} title={running.title} onClose={() => setRunning(null)}/> : null}
    {installing ? <ConfirmDialog title={`安装「${installing.title}」？`} confirmLabel="安装" busy={busyId !== null}
      description={<div><p>{installing.requestedPermissions.length ? "此安装包请求：" : "未申请文件与成果权限。"}</p>
        <p>将停止受管任务，以独立数据副本安装。原版本与数据保留。</p>
        {installing.requestedPermissions.length ? <ul>{installing.requestedPermissions.map(permission => <li key={permission}>{personalPermissionLabel(permission)}</li>)}</ul> : null}
      </div>}
      onCancel={() => setInstalling(null)} onConfirm={() => { void install(installing); }}/> : null}
    {importing ? <PersonalPackageImport target={importing.target} targetTitle={importing.title} profileId={props.profileId} onClose={() => setImporting(null)}
      onChanged={async projectId => {
        if (projectId) {
          close(`personal:${projectId}`);
          setRunning(previous => previous?.creationProjectId === projectId ? null : previous);
        }
        if (!await load()) throw new Error("personal_package_import_list_refresh_failed");
      }}/> : null}
  </section></PageVisibilityContext.Provider>;
}

function personalPermissionLabel(permission: string) {
  const labels: Record<string, string> = { "personal_file.read": "读取选定文件", "personal_file.write_confirmed": "确认后写入文件", "workbench_artifact.create": "创建成果" };
  return Object.hasOwn(labels, permission) ? labels[permission] : permission;
}

function PackageImportIcon() {
  return <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3 7.5 10 4l7 3.5v8L10 19l-7-3.5v-8Zm0 0 7 3.5 7-3.5M10 11v8M10 1v6m-2-2 2 2 2-2" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round"/></svg>;
}

function PersonalProjects(props: {
  onEdit: (project: CapabilityCreationProject) => void;
  onManage: (project: CapabilityCreationProject) => void;
  onExport: (project: CapabilityCreationProject) => void;
  onRun: (project: CapabilityCreationProject) => void;
  busyId: string | null;
  error: string | null;
  loading: boolean;
  onInstall: (project: CapabilityCreationProject) => void;
  onPreview: (project: CapabilityCreationProject) => void;
  onRetry: () => void;
  projects: CapabilityCreationProject[];
  query: string;
}) {
  if (props.loading) {
    return <div className="applications-state" role="status"><span className="applications-spinner" /><strong>正在读取</strong></div>;
  }
  if (props.error) {
    return <ApplicationFailure message={props.error} onRetry={props.onRetry} />;
  }
  if (!props.projects.length) {
    return <div className="applications-state"><EmptyAppsIcon /><strong>还没有应用</strong></div>;
  }
  const keyword = props.query.trim().toLocaleLowerCase();
  const projects = keyword
    ? props.projects.filter((project) => `${project.title}\n${project.businessGoal}`.toLocaleLowerCase().includes(keyword))
    : props.projects;
  if (!projects.length) {
    return <div className="applications-no-match"><strong>没有匹配的应用</strong></div>;
  }
  return (
    <section className="personal-capabilities" aria-label="应用">
      <header><h3>应用</h3><span>{projects.length}</span></header>
      <div className="application-grid">
        {projects.map((project) => {
          const installed = project.status === "installed";
          const packaged = project.status === "packaged";
          const hasWeb = matchesWebTarget(project.targetType);
          const busy = props.busyId !== null;
          const primaryLabel = installed ? (hasWeb ? "打开" : "运行") : packaged ? "安装" : "编辑";
          const primaryAction = () => installed
            ? (hasWeb ? props.onPreview(project) : props.onRun(project))
            : packaged ? props.onInstall(project) : props.onEdit(project);
          const progressLabel = props.busyId === `install:${project.creationProjectId}` ? "正在安装…"
            : props.busyId === `personal:${project.creationProjectId}` ? "正在打开…" : null;
          const fileCount = project.sourceFileCount ? `${project.sourceFileCount} 个文件` : "无源码";
          return (
            <ApplicationTile
              actionLabel={progressLabel ?? primaryLabel}
              busy={busy}
              description={`${capabilityTargetLabel(project.targetType)} · ${fileCount}`}
              key={project.creationProjectId}
              name={project.title}
              onOpen={primaryAction}
              status={progressLabel ?? capabilityStatusLabel(project.status)}
              statusTone={project.status}
              menu={<>
                {installed || packaged ? <RichMenuItem label="编辑" disabled={busy} onSelect={() => props.onEdit(project)} /> : null}
                {packaged && hasWeb ? <RichMenuItem label="预览" disabled={busy} onSelect={() => props.onPreview(project)} /> : null}
                {installed ? <RichMenuItem label="管理" disabled={busy} onSelect={() => props.onManage(project)} /> : null}
                {installed || packaged ? <RichMenuItem label="导出" disabled={busy} onSelect={() => props.onExport(project)} /> : null}
              </>}
            />
          );
        })}
      </div>
    </section>
  );
}

function matchesPersonalPreview(project: CapabilityCreationProject) {
  return matchesWebTarget(project.targetType) && ["packaged", "installed"].includes(project.status);
}

function matchesWebTarget(targetType: string) {
  return targetType === "web_application" || targetType === "web_mcp_application";
}

function securedPersonalPreview(html: string) {
  const policy = "default-src 'none'; connect-src 'none'; img-src data: blob:; media-src data: blob:; font-src data:; style-src 'unsafe-inline'; script-src 'none'; object-src 'none'; frame-src 'none'; child-src 'none'; base-uri 'none'; form-action 'none'";
  return `<!doctype html><meta http-equiv="Content-Security-Policy" content="${policy}">${html}`;
}

function capabilityTargetLabel(value: string) {
  if (value === "local_mcp") return "本地 MCP";
  if (value === "web_application") return "Web";
  if (value === "web_mcp_application") return "Web + MCP";
  return "个人能力";
}

function capabilityStatusLabel(value: string) {
  if (value === "planned") return "待开发";
  if (value === "developing") return "开发中";
  if (value === "validating") return "验证中";
  if (value === "preview_ready") return "可预览";
  if (value === "packaged") return "已打包";
  if (value === "installed") return "已安装";
  if (value === "failed") return "失败";
  return value;
}

function ApplicationFailure(props: { message: string; onRetry: () => void }) {
  return (
    <div className="applications-state applications-failure">
      <EmptyAppsIcon />
      <strong>无法读取应用</strong>
      <p>{props.message}</p>
      <button onClick={props.onRetry} type="button">
        重试
      </button>
    </div>
  );
}
function EmptyAppsIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 48 48">
      <rect x="7" y="7" width="14" height="14" rx="4" />
      <rect x="27" y="7" width="14" height="14" rx="4" />
      <rect x="7" y="27" width="14" height="14" rx="4" />
      <path d="M34 27v14M27 34h14" />
    </svg>
  );
}

