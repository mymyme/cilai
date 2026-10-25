import type { WorkspaceProjectIdentity } from "./project-workspaces";
import { invoke } from "./host";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import type { ConflictItem, WorkspaceFileItem, WorkspaceSummary } from "./generated/protocol";
import type { MessageSink } from "./app-message";
import type { PreviewView } from "./preview";
import { FileDiffPane } from "./text-diff";
import { FileConfirmationDialog } from "./file-confirmation";
import { buildStatusChips, StatusChip, localStatusLabel, presenceLabel } from "./file-status";
import { AiIcon, BackIcon, CheckIcon, CloseIcon, CodeIcon, ConfirmIcon, DiffIcon, ExternalOpenIcon, EyeIcon, FileIcon, Folder, InfoIcon, MoreIcon, RevealIcon, SearchIcon, WorkspaceIcon } from "./icons";
import { hostPlatformUi } from "./host-platform-ui";
import { LoadFailure } from "./load-failure";

const FilePreviewPane = lazy(() => import("./preview").then(module => ({ default: module.FilePreviewPane })));


/** 这一页自己的读取状态：外壳靠它决定先画骨架还是画失败页。 */
export type LoadState = "loading" | "ready" | "error";

/**
 * 文件这一侧的界面：工作区切换器、文件预览与确认工作台，以及承载两万多个条目的虚拟
 * 文件树。它们此前和应用外壳同处一个文件，读任何一边都要先翻过另一边。
 */
export function WorkspaceSwitcher(props: { workspaces: WorkspaceSummary[]; selectedWorkspaceId: string; projectIndex: Map<string, WorkspaceProjectIdentity[]>; addWorkspaceDisabled: boolean; onAddWorkspace: () => void; onSelect: (id: string) => void }) {
  const [query, setQuery] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => { if (searchOpen) input.current?.focus(); }, [searchOpen]);
  const normalizedQuery = query.trim().toLocaleLowerCase("zh-CN");
  const rows = props.workspaces.flatMap((workspace) => {
    const projects = props.projectIndex.get(workspace.workspaceId);
    return projects?.length ? [{ workspace, project: { displayName: projects.map((project) => project.displayName).join("、") } }] : [];
  });
  const filtered = normalizedQuery
    ? rows.filter(({ project, workspace }) => `${project.displayName}\n${workspace.displayName}`.toLocaleLowerCase("zh-CN").includes(normalizedQuery))
    : rows;

  return <section aria-label="切换工作区" className="workspace-switcher" role="dialog">
    <header className="workspace-switcher-heading"><strong>切换工作区</strong>{!searchOpen ? <button aria-label="搜索项目或工作区" onClick={() => setSearchOpen(true)} title="搜索项目或工作区" type="button"><SearchIcon/></button> : null}</header>
    {searchOpen ? <div className="workspace-switcher-search" data-focus-frame="wrapper"><SearchIcon/><input aria-label="搜索项目或工作区" onChange={(event) => setQuery(event.target.value)} placeholder="搜索项目或工作区" ref={input} value={query}/></div> : null}
    <div className="workspace-switcher-list">
      {filtered.map(({ project, workspace }) => {
        const status = workspace.workspaceKind === "cloud_team" ? "旧云端来源" : workspace.localStatus === "ready" ? "本地" : localStatusLabel(workspace.localStatus);
        return <button aria-current={workspace.workspaceId === props.selectedWorkspaceId ? "true" : undefined} aria-label={`${workspace.displayName}，${project.displayName}`} key={workspace.workspaceId} onClick={() => props.onSelect(workspace.workspaceId)} title={`${workspace.displayName} · ${project.displayName}`} type="button"><span className="workspace-check">{workspace.workspaceId === props.selectedWorkspaceId ? <CheckIcon/> : <WorkspaceIcon/>}</span><span className="workspace-switcher-name"><strong>{workspace.displayName}</strong><small>{project.displayName}</small></span>{status ? <em data-status={workspace.localStatus}>{status}</em> : null}</button>;
      })}
      {filtered.length === 0 ? <p>没有匹配的项目或工作区</p> : null}
    </div>
    <footer className="workspace-switcher-footer"><button disabled={props.addWorkspaceDisabled} onClick={props.onAddWorkspace} type="button"><WorkspaceIcon/>添加工作空间</button></footer>
  </section>;
}


export function WorkspacePage(props: { hostPlatform: string | null; loadError: unknown; onRetryLoad: () => void; detailIntent: number; diffIntent: number; workspaces: WorkspaceSummary[]; selectedWorkspace: WorkspaceSummary | null; files: WorkspaceFileItem[]; selectedFile: WorkspaceFileItem | null; onFile: (id: string) => void; revision: number; conflicts: ConflictItem[]; createWorkspace: () => Promise<void>; connectingWorkspace: boolean; loadState: LoadState; onMessage: MessageSink; onOpenAi: () => void; onConfirmationChanged?: () => Promise<void> | void; deliverableBinding: { deliverableTitle: string; versionNo: number; accepted: boolean } | null }) {
  const [view, setView] = useState<"preview" | "diff">("preview");
  const [previewView, setPreviewView] = useState<PreviewView>("rendered");
  const [previewViewAvailable, setPreviewViewAvailable] = useState(false);
  const [confirmationDialogOpen, setConfirmationDialogOpen] = useState(false);
  const [panel, setPanel] = useState<"file" | null>(null);
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);
  const platformUi = hostPlatformUi(props.hostPlatform);

  useEffect(() => {
    if (!moreOpen) return;
    const closeOnOutside = (event: PointerEvent) => {
      if (!moreRef.current?.contains(event.target as Node)) setMoreOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMoreOpen(false);
    };
    window.addEventListener("pointerdown", closeOnOutside);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", closeOnOutside);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [moreOpen]);
  useEffect(() => {
    if (view === "diff" && !props.selectedFile?.baselineVersionId) setView("preview");
  }, [props.selectedFile?.baselineVersionId, view]);
  useEffect(() => {
    setView("preview");
    setPreviewView("rendered");
    setPreviewViewAvailable(false);
    setConfirmationDialogOpen(false);
  }, [props.selectedFile?.localFileId]);

  const detailIntentRef = useRef(props.detailIntent);
  useEffect(() => {
    if (props.detailIntent === detailIntentRef.current) return;
    detailIntentRef.current = props.detailIntent;
    setPanel("file");
  }, [props.detailIntent]);

  const diffIntentRef = useRef(props.diffIntent);
  useEffect(() => {
    if (props.diffIntent === diffIntentRef.current) return;
    diffIntentRef.current = props.diffIntent;
    if (props.selectedFile?.baselineVersionId) setView("diff");
  }, [props.diffIntent, props.selectedFile?.baselineVersionId]);
  useEffect(() => {
    if (panel === null) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setPanel(null);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [panel]);
  if (props.loadState === "loading") return <div className="center-state">正在连接本地服务并恢复工作区…</div>;
  // 读不出来时绝不能落进下面那个空状态：它会劝用户新建工作区，而他的工作区好端端在。
  if (props.loadState === "error") return <LoadFailure error={props.loadError} what="工作区" onRetry={props.onRetryLoad}/>;
  if (!props.selectedWorkspace) return <div className="empty-state"><Folder/><h2>连接本地文件夹</h2><p>选择后即可扫描、浏览和预览本地文件。</p><button disabled={props.connectingWorkspace} onClick={() => void props.createWorkspace()} type="button">{props.connectingWorkspace ? "正在选择…" : "连接本地目录"}</button></div>;
  if (props.selectedWorkspace.workspaceKind !== "local") return <div className="empty-state" role="alert"><h2>旧云端来源不可用</h2><p>{props.selectedWorkspace.displayName}</p><p>个人版不支持此云端来源，请明确选择本地文件。</p></div>;
  // Chips report what needs a human. States that mean "nothing to do here" are omitted rather
  // than spelled out, so a healthy file shows no chips at all instead of three saying so. The
  // confirmation chip also stands down while the 待确认 button is showing, because that button
  // already names the same state and offers the action.
  const statusChips = buildStatusChips(props.selectedFile, false, props.selectedWorkspace.workspaceKind);
  const selectedFileName = props.selectedFile ? fileNameFromRelativePath(props.selectedFile.relativePath) : "文件预览";
  const openSelectedFile = async (openMode: "default" | "finder") => {
    if (!props.selectedFile) return;
    try {
      await invoke("workspace_open_item", { localRoot: props.selectedWorkspace!.localRoot, relativePath: props.selectedFile.relativePath, openMode });
    } catch (error) {
      props.onMessage(`无法打开文件：${String(error)}`, "error");
    }
  };
  return <div className="workspace-layout">
    <section className="document-stage">
      <div className="document-toolbar">
        <div className="document-view-context">
          <span className="document-source" title={props.selectedFile?.relativePath ?? props.selectedWorkspace.displayName}>本地 · {props.selectedWorkspace.displayName}{props.selectedFile?.relativePath.includes("/") ? ` / ${props.selectedFile.relativePath.slice(0, props.selectedFile.relativePath.lastIndexOf("/"))}` : ""}</span>
          {view !== "preview" ? <button onClick={() => setView("preview")} type="button"><BackIcon/>返回预览</button> : null}
          <strong title={props.selectedFile?.relativePath}>{selectedFileName}</strong>
          {view === "preview" && props.selectedFile?.baselineVersionId ? <button className="diff-trigger" onClick={() => setView("diff")} type="button"><DiffIcon/>查看差异</button> : null}
        </div>
        <div className="document-toolbar-actions">
          {statusChips.length ? <div aria-label="当前文件状态" className="file-status-summary">
            {statusChips.map((chip) => <StatusChip key={chip.label} label={chip.label} tone={chip.tone}/>)}
          </div> : null}
          {view === "preview" && previewViewAvailable ? <button
            aria-label={previewView === "rendered" ? "切换到源码" : "切换到预览"}
            className="toolbar-action preview-view-toggle"
            onClick={() => setPreviewView((current) => current === "rendered" ? "source" : "rendered")}
            title={previewView === "rendered" ? "查看源码" : "查看预览"}
            type="button">
            {previewView === "rendered" ? <CodeIcon/> : <EyeIcon/>}
          </button> : null}
          <button className="toolbar-action" disabled={!props.selectedFile} onClick={props.onOpenAi} type="button"><AiIcon/>AI 协作</button>
          <div className="toolbar-more" ref={moreRef}>
            <button aria-expanded={moreOpen} aria-label="更多文件操作" className="toolbar-action" disabled={!props.selectedFile} onClick={() => setMoreOpen((open) => !open)} title="更多文件操作" type="button"><MoreIcon/></button>
            {moreOpen ? <div className="toolbar-more-menu" role="menu">
              <button onClick={() => { setMoreOpen(false); void openSelectedFile("default"); }} role="menuitem" type="button"><ExternalOpenIcon/>系统打开</button>
              {platformUi ? <button onClick={() => { setMoreOpen(false); void openSelectedFile("finder"); }} role="menuitem" type="button"><RevealIcon/>在{platformUi.fileManagerName}中显示</button> : null}
              <button disabled={!props.selectedFile?.checksumValue} onClick={() => { setMoreOpen(false); setConfirmationDialogOpen(true); }} role="menuitem" type="button"><ConfirmIcon/>确认内容</button>
              <button onClick={() => { setMoreOpen(false); setPanel("file"); }} role="menuitem" type="button"><InfoIcon/>文件详情</button>
            </div> : null}
          </div>
        </div>
      </div>
      <div className="document-canvas">{props.selectedFile ? view === "preview"
        ? <Suspense fallback={<div className="preview-message" role="status">正在加载预览</div>}><FilePreviewPane workspaceId={props.selectedWorkspace.workspaceId} workspaceKind={props.selectedWorkspace.workspaceKind} localRoot={props.selectedWorkspace.localRoot} hostPlatform={props.hostPlatform} file={props.selectedFile} view={previewView} onViewAvailabilityChange={setPreviewViewAvailable}/></Suspense>
        : <FileDiffPane workspaceId={props.selectedWorkspace.workspaceId} localFileId={props.selectedFile.localFileId} leftSource="baseline" rightSource="local"/>
        : null}</div>
    </section>
    {confirmationDialogOpen && props.selectedFile?.checksumAlgorithm && props.selectedFile.checksumValue ? <FileConfirmationDialog
      baselineVersionId={props.selectedFile.baselineVersionId}
      checksumAlgorithm={props.selectedFile.checksumAlgorithm}
      checksumValue={props.selectedFile.checksumValue}
      localFileId={props.selectedFile.localFileId}
      onCancel={() => setConfirmationDialogOpen(false)}
      onConfirmed={props.onConfirmationChanged}
      onMessage={props.onMessage}
      relativePath={props.selectedFile.relativePath}
      workspaceId={props.selectedWorkspace.workspaceId}/>: null}
    {panel ? <><button aria-label="关闭侧边面板" className="workspace-drawer-backdrop" onClick={() => setPanel(null)} type="button"/><aside aria-label="文件详情" className="workspace-drawer">
      <header><div><h2>文件详情</h2></div><button aria-label="关闭侧边面板" onClick={() => setPanel(null)} type="button"><CloseIcon/></button></header>
      <div className="workspace-drawer-body">{props.selectedFile ? <>
        <dl className="file-detail-list"><Row label="路径" value={props.selectedFile.relativePath}/><Row label="存在状态" value={presenceLabel(props.selectedFile.presenceState)}/><Row label="状态修订" value={String(props.selectedFile.stateRevision ?? "—")}/></dl>
        {props.deliverableBinding ? <p className="file-deliverable-binding">该文件通过交付物《{props.deliverableBinding.deliverableTitle}》第 {props.deliverableBinding.versionNo} 版关联到当前项目{props.deliverableBinding.accepted ? "（已验收）" : "（候选版本）"}。</p> : null}
      </> : <p className="drawer-empty">未选择文件。</p>}</div>
    </aside></> : null}
  </div>;
}


export function fileNameFromRelativePath(relativePath: string): string {
  return relativePath.split("/").filter(Boolean).at(-1) ?? relativePath;
}

function Row({ label, value }: { label: string; value: string }) { return <div><dt>{label}</dt><dd>{value}</dd></div>; }

export function WorkspaceFileSearch(props: { files: WorkspaceFileItem[]; query: string; loadState: LoadState; error: unknown; onRetry: () => void; onFile: (id: string) => void }) {
  const [limit, setLimit] = useState(100);
  useEffect(() => setLimit(100), [props.query]);
  const matches = useMemo(() => props.files.filter((file) => file.fileType !== "directory" && file.relativePath.toLocaleLowerCase().includes(props.query.trim().toLocaleLowerCase())), [props.files, props.query]);
  if (props.loadState === "loading") return <p role="status">正在读取文件…</p>;
  if (props.loadState === "error") return <LoadFailure error={props.error} what="文件搜索" onRetry={props.onRetry}/>;
  return <div className="explorer-search-results" aria-label="文件搜索结果">
    <small>{matches.length ? `${matches.length} 个匹配文件` : "已索引文件中没有匹配项"}</small>
    {matches.slice(0, limit).map((file) => <button key={file.localFileId} onClick={() => props.onFile(file.localFileId)} title={file.relativePath} type="button"><FileIcon/><span><strong>{fileNameFromRelativePath(file.relativePath)}</strong><small>{file.relativePath}</small></span></button>)}
    {matches.length > limit ? <button onClick={() => setLimit((value) => value + 100)} type="button">显示更多</button> : null}
  </div>;
}
