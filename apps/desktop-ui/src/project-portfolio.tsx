import { MoreIcon } from "./icons";
import { useEffect, useMemo, useState } from "react";
import type { ProjectPortfolioItem, ProjectPortfolioListResult, ProjectPortfolioPhaseItem, ProjectSummary, WorkspaceSummary } from "./generated/protocol";
import { rpc } from "./app-server-rpc";
import { auditMoment, relativeDay } from "./datetime";
import { ProjectCreatePanel } from "./project-create";
import { ProjectDeleteDialog } from "./project-delete";
import { Select } from "./select";
import type { MessageSink } from "./app-message";
import { LoadFailure } from "./load-failure";
import { SearchField } from "./search-field";

interface PortfolioItem {
  project: ProjectSummary;
  workspace: WorkspaceSummary | null;
  workspaces: WorkspaceSummary[];
  workspaceIds: string[];
  activePhases: ProjectPortfolioPhaseItem[];
  nextPhaseName: string | null;
  taskTotal: number;
  taskCompleted: number;
  acceptedDeliverables: number;
  deliverableCount: number;
  blockers: number;
  decisions: number;
  incidents: number;
}

export function ProjectPortfolioPage(props: { createIntent: number; deleteProjectId: string | null; onAddWorkspace: () => Promise<WorkspaceSummary | null | undefined>; workspaces: WorkspaceSummary[]; eventSequence: number; onMessage: MessageSink; onEditProject: (projectId: string) => void; onOpenProgress: (workspaceId: string | null, projectId: string, phaseId: string | null) => void }) {
  const [items, setItems] = useState<PortfolioItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [loaded, setLoaded] = useState(false);
  // 读不出来和这里是空的必须分开：空状态是一句关于事实的陈述。
  const [loadError, setLoadError] = useState<unknown>(null);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<"all" | "attention" | "healthy">("all");
  const [sort, setSort] = useState<"attention" | "updated" | "name">("attention");
  const [visibleLimit, setVisibleLimit] = useState(24);
  const [creating, setCreating] = useState(false);
  const [reloadToken, setReloadToken] = useState(0);
  const [pendingDelete, setPendingDelete] = useState<PortfolioItem | null>(null);
  // Bumped by the file page's 新建项目 entry so the same panel opens wherever the user started.
  useEffect(() => { if (props.createIntent > 0) setCreating(true); }, [props.createIntent]);
  // 对话页的"移除项目"把删除请求转到这里：真实的数量和输入项目名确认都在这一页，
  // 在别处另造一个删除路径只会让同一个动作有两套护栏。
  useEffect(() => {
    if (props.deleteProjectId === null) return;
    const target = items.find((item) => item.project.projectId === props.deleteProjectId);
    if (target) setPendingDelete(target);
  }, [props.deleteProjectId, items]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    void loadPortfolio(props.workspaces).then((result) => { if (!cancelled) { setItems(result); setLoaded(true); } }).catch((error) => { if (!cancelled) { setLoadError(error); props.onMessage(String(error), "error"); } }).finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [props.eventSequence, props.workspaces, reloadToken]);

  const matched = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase("zh-CN");
    const filtered = items.filter((item) => {
      if (filter === "attention" && item.blockers + item.decisions + item.incidents === 0) return false;
      if (filter === "healthy" && item.blockers + item.decisions + item.incidents > 0) return false;
      return !normalized || `${item.project.displayName}\n${item.project.goal}\n${item.workspaces.map((workspace) => workspace.displayName).join("\n")}`.toLocaleLowerCase("zh-CN").includes(normalized);
    });
    return filtered.sort((left, right) => sort === "name"
      ? left.project.displayName.localeCompare(right.project.displayName, "zh-CN")
      : sort === "updated"
        ? right.project.updatedAt - left.project.updatedAt
        : right.blockers + right.decisions + right.incidents - (left.blockers + left.decisions + left.incidents) || right.project.updatedAt - left.project.updatedAt);
  }, [filter, items, query, sort]);
  const visible = matched.slice(0, visibleLimit);

  return <section aria-busy={loading} className="project-portfolio-page">
    <header><div><h1>项目</h1><p className="portfolio-summary"><strong>{items.length}</strong> 个项目<span aria-hidden="true">·</span><strong data-attention={items.reduce((sum, item) => sum + item.blockers + item.decisions + item.incidents, 0) > 0 ? "true" : undefined}>{items.reduce((sum, item) => sum + item.blockers + item.decisions + item.incidents, 0)}</strong> 项需要介入</p></div></header>
    <div className="portfolio-controls"><SearchField ariaLabel="搜索项目" className="portfolio-search" onChange={(value) => { setQuery(value); setVisibleLimit(24); }} placeholder="搜索项目、目标或工作区" value={query}/><div className="portfolio-control-group"><Select ariaLabel="项目排序" options={[{ value: "attention", label: "介入优先" }, { value: "updated", label: "最近更新" }, { value: "name", label: "项目名称" }]} value={sort} onChange={(value) => setSort(value as typeof sort)}/><nav>{(["all", "attention", "healthy"] as const).map((value) => <button className={filter === value ? "active" : ""} key={value} onClick={() => { setFilter(value); setVisibleLimit(24); }} type="button">{value === "all" ? "全部项目" : value === "attention" ? "需要介入" : "推进正常"}</button>)}</nav></div><button className="portfolio-create-trigger" onClick={() => setCreating((value) => !value)} type="button">{creating ? "收起" : "新建项目"}</button></div>
    {pendingDelete ? <ProjectDeleteDialog
      attentionTotal={pendingDelete.blockers + pendingDelete.decisions + pendingDelete.incidents}
      deliverableTotal={pendingDelete.deliverableCount}
      displayName={pendingDelete.project.displayName}
      onCancel={() => setPendingDelete(null)}
      onDeleted={() => { setPendingDelete(null); setReloadToken((value) => value + 1); }}
      onMessage={props.onMessage}
      projectId={pendingDelete.project.projectId}
      taskTotal={pendingDelete.taskTotal}/> : null}
    {creating ? <ProjectCreatePanel
      onCancel={() => setCreating(false)}
      onCreated={(project, workspaceId) => {
        setCreating(false);
        setReloadToken((value) => value + 1);
        props.onOpenProgress(workspaceId, project.projectId, null);
      }}
      onMessage={props.onMessage}
      existingProjects={items.map((item) => ({ projectId: item.project.projectId, displayName: item.project.displayName }))}
      onAddWorkspace={props.onAddWorkspace}
      workspaces={props.workspaces}/> : null}
    {loading && !loaded ? <div className="portfolio-empty">正在读取各项目推进事实…</div>
      : loadError ? <LoadFailure error={loadError} what="项目" onRetry={() => setReloadToken((value) => value + 1)}/>
        : visible.length ? <>
          <div className="portfolio-table-shell">
            <table className="portfolio-table">
              <thead><tr><th scope="col">项目</th><th scope="col">当前阶段</th><th className="numeric" scope="col">已完成 / 总任务</th><th className="numeric" scope="col">已验收 / 总交付</th><th className="numeric" scope="col">待介入</th><th scope="col">工作区</th><th scope="col">项目更新</th><th className="actions" scope="col">操作</th></tr></thead>
              <tbody>{visible.map((item) => <ProjectRow item={item} key={item.project.projectId} onDelete={setPendingDelete} onEdit={props.onEditProject} onProgress={props.onOpenProgress}/>)}</tbody>
            </table>
          </div>
          {visible.length < matched.length ? <button className="portfolio-load-more" onClick={() => setVisibleLimit((value) => value + 24)} type="button">加载更多 · 剩余 {matched.length - visible.length}</button> : null}
        </> : <div className="portfolio-empty"><ProjectIcon/><strong>没有匹配的项目</strong><span>当前没有可访问的项目；项目不会根据文件夹名称自动推断。</span>{creating ? null : <button className="primary" onClick={() => setCreating(true)} type="button">新建项目</button>}</div>}
  </section>;
}

async function loadPortfolio(workspaces: WorkspaceSummary[]): Promise<PortfolioItem[]> {
  const workspaceById = new Map(workspaces.map((workspace) => [workspace.workspaceId, workspace]));
  const result = await rpc<ProjectPortfolioListResult>("projectPortfolio/list", { workspaceIds: workspaces.map((workspace) => workspace.workspaceId) });
  return result.items.map((item) => portfolioItem(item, workspaceById));
}

function portfolioItem(item: ProjectPortfolioItem, workspaceById: Map<string, WorkspaceSummary>): PortfolioItem {
  const workspace = item.workspaceId ? workspaceById.get(item.workspaceId) ?? null : null;
  if (item.workspaceId && !workspace) throw new Error(`项目总览引用了未知工作区：${item.workspaceId}`);
  const workspaces = item.workspaceIds.map((workspaceId) => {
    const linked = workspaceById.get(workspaceId);
    if (!linked) throw new Error(`项目总览引用了未知工作区：${workspaceId}`);
    return linked;
  });
  return {
    project: { projectId: item.projectId, displayName: item.displayName, goal: item.goal, status: item.status, primaryWorkspaceId: item.workspaceId, stateRevision: item.stateRevision, updatedAt: item.updatedAt },
    workspace,
    workspaces,
    workspaceIds: item.workspaceIds,
    activePhases: item.activePhases,
    nextPhaseName: item.nextPhaseName,
    taskTotal: item.taskTotal,
    taskCompleted: item.taskCompleted,
    acceptedDeliverables: item.deliverableAccepted,
    deliverableCount: item.deliverableTotal,
    blockers: item.blockerCount,
    decisions: item.decisionCount,
    incidents: item.incidentCount,
  };
}

function ProjectRow({ item, onProgress, onDelete, onEdit }: { item: PortfolioItem; onProgress: (workspaceId: string | null, projectId: string, phaseId: string | null) => void; onEdit: (projectId: string) => void; onDelete: (item: PortfolioItem) => void }) {
  const attention = item.blockers + item.decisions + item.incidents;
  const phase = item.activePhases[0] ?? null;
  const phaseLabel = phase
    ? `${phase.name} · ${phase.status === "review" ? "待阶段验收" : phase.status === "paused" ? "已暂停" : "进行中"}`
    : item.nextPhaseName ?? "尚无活动阶段";
  const workspaceLabel = `${item.workspaces.length} 个`;
  const workspaceNames = item.workspaces.length
    ? item.workspaces.map((workspace) => workspace.displayName).join("、")
    : "暂无工作区";
  return <tr className="portfolio-row">
    <td className="portfolio-project-cell">
      <button
        className="portfolio-project-open"
        onClick={() => onProgress(item.workspace?.workspaceId ?? null, item.project.projectId, phase?.phaseId ?? null)}
        title={`进入 ${item.project.displayName}`}
        type="button">
        <strong>{item.project.displayName}</strong>
      </button>
    </td>
    <td className="portfolio-row-phase" title={phaseLabel}>{phaseLabel}</td>
    <td className="portfolio-row-metric">{item.taskCompleted}/{item.taskTotal}</td>
    <td className="portfolio-row-metric">{item.acceptedDeliverables}/{item.deliverableCount}</td>
    <td className="portfolio-row-metric" data-alert={attention > 0 ? "true" : undefined}>{attention}</td>
    <td className="portfolio-row-workspace" title={workspaceNames}>{workspaceLabel}</td>
    <td><time dateTime={new Date(item.project.updatedAt).toISOString()} title={auditMoment(item.project.updatedAt)}>{relativeDay(item.project.updatedAt)}</time></td>
    <td><span className="portfolio-row-actions">
      <button aria-label={`编辑项目 ${item.project.displayName}`} onClick={() => onEdit(item.project.projectId)} title="编辑项目" type="button"><EditIcon/></button>
      <details className="inline-more"><summary aria-label={`更多项目操作 ${item.project.displayName}`}><MoreIcon/></summary><button aria-label={`删除项目 ${item.project.displayName}`} className="danger" onClick={() => onDelete(item)} title="删除项目" type="button"><TrashIcon/>删除</button></details>
    </span></td>
  </tr>;
}

function EditIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M13.2 3.6a1.7 1.7 0 0 1 2.4 2.4L7.4 14.2 4 15l.8-3.4Z"/></svg>; }
function TrashIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M4 6h12M8 6V4.5h4V6M5.5 6l.8 9.5h7.4l.8-9.5M8.5 9v4M11.5 9v4"/></svg>; }
function ProjectIcon() { return <svg aria-hidden="true" viewBox="0 0 48 48"><path d="M7 12h14l4 5h16v22H7z"/><path d="M14 26h20M14 32h14"/></svg>; }
