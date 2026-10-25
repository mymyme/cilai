import { conversationFileArtifacts } from "./ai-artifacts";
import { displayModelId } from "./model-label";
import { ImageArtifactLightbox } from "./artifact-preview";
import { TaskArtifactList, type TaskArtifactPreview } from "./task-artifacts";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { randomUuid } from "./uuid";
import { useModal } from "./use-modal";
import type { AttentionCaseItem, AttentionCaseListResult, AttentionCaseTransitionResult, BusinessTaskContributionItem, BusinessTaskGetResult, BusinessTaskItem, DeliverableItem, DeliverableListResult, MyWorkGetResult, MyWorkItem, ProjectPhaseItem, ProjectProgressGetResult, WorkspaceSummary } from "./generated/protocol";
import { rpc, sha256 } from "./app-server-rpc";
import { Select } from "./select";
import type { MessageSink } from "./app-message";
import { deadlineDay, listMoment } from "./datetime";
import { LoadFailure } from "./load-failure";
import { SearchField } from "./search-field";
import { emptyAiQuerySnapshot, loadAiRuntimeQuery, type AiRuntimeQuerySnapshot } from "./ai-runtime";

export interface RecentConversation { conversationId: string; title: string; updatedAt: number; projectName: string | null; running: boolean }

interface MyWorkPageProps {
  workspaces: WorkspaceSummary[];
  eventSequence: number;
  onMessage: MessageSink;
  onOpenSource: (item: MyWorkItem) => void;
  onOpenRun: (runId: string) => void;
  onOpenConversation: (conversationId: string) => void;
  onStartConversation?: () => void;
  onStartTask?: (projectId: string | null) => void;
  recentConversations?: RecentConversation[];
  recentFailure?: { error: unknown; retry: () => void };
}

const groups = [
  { id: "todo", label: "待处理", hint: "需要你继续处理的任务" },
  { id: "decision", label: "待决策", hint: "等待你的选择" },
  { id: "in_progress", label: "进行中", hint: "正在推进的任务" },
  { id: "review", label: "待验收", hint: "已有产物更新" },
] as const;

const priorityLabel: Record<string, string> = {
  low: "低",
  medium: "中",
  high: "高",
  critical: "紧急",
};

const decisionActionOptions = [
  { value: "approved", label: "通过" },
  { value: "change_requested", label: "请求修改" },
  { value: "rejected", label: "拒绝" },
  { value: "delegated", label: "转交" },
] as const;

export function MyWorkPage(props: MyWorkPageProps) {
  const [items, setItems] = useState<MyWorkItem[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);
  // 读不出来和这里是空的必须分开：空状态是一句关于事实的陈述。
  const [loadError, setLoadError] = useState<unknown>(null);
  const [query, setQuery] = useState("");
  const [projectId, setProjectId] = useState("all");
  const [priority, setPriority] = useState("all");
  const [activeDecision, setActiveDecision] = useState<MyWorkItem | null>(null);
  const [activeDetail, setActiveDetail] = useState<MyWorkItem | null>(null);
  const [openingItemId, setOpeningItemId] = useState<string | null>(null);
  const [retryNonce, setRetryNonce] = useState(0);
  const loadGeneration = useRef(0);
  const conversationOpenGeneration = useRef(0);

  useEffect(() => () => { conversationOpenGeneration.current += 1; }, []);

  const openItemConversation = async (item: MyWorkItem) => {
    if (openingItemId !== null) return;
    const generation = ++conversationOpenGeneration.current;
    setOpeningItemId(item.workItemId);
    try {
      const conversationId = await loadWorkItemConversationId(item);
      if (generation === conversationOpenGeneration.current) props.onOpenConversation(conversationId);
    } catch (error) {
      if (generation === conversationOpenGeneration.current) props.onMessage(String(error), "error");
    } finally {
      if (generation === conversationOpenGeneration.current) setOpeningItemId(null);
    }
  };

  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    setLoading(true);
    setLoadError(null);
    try {
      const result = await rpc<MyWorkGetResult>("myWork/get", { projectId: null, focusGroup: null, limit: 200 });
      if (generation !== loadGeneration.current) return;
      setItems(result.items);
      setActiveDetail((current) => current ? result.items.find((item) => item.workItemId === current.workItemId) ?? null : null);
      setTotal(result.total);
    } catch (error) {
      if (generation === loadGeneration.current) setLoadError(error);
    } finally {
      if (generation === loadGeneration.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
    return () => { loadGeneration.current += 1; };
  }, [load, props.eventSequence, retryNonce]);

  const projects = useMemo(() => Array.from(new Map(items.map((item) => [item.projectId, item.projectName])).entries()), [items]);
  const filtered = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase("zh-CN");
    return items.filter((item) => {
      if (projectId !== "all" && item.projectId !== projectId) return false;
      if (priority !== "all" && item.priority !== priority) return false;
      if (!normalized) return true;
      return [item.title, item.projectName, item.phaseName, item.deliverableTitle, item.responsibilityLabel]
        .filter(Boolean)
        .join("\n")
        .toLocaleLowerCase("zh-CN")
        .includes(normalized);
    });
  }, [items, priority, projectId, query]);

  const activeDecisionWorkspaceName = activeDecision
    ? props.workspaces.find((workspace) => workspace.workspaceId === activeDecision.primaryWorkspaceId)?.displayName ?? null
    : null;

  return <section className="my-work-page">
    <header className="my-work-hero">
      <div><h1>工作台</h1><p className="my-work-summary"><strong>{total}</strong> 个当前待办<span aria-hidden="true">·</span>{projects.length} 个相关项目</p></div>
      {props.onStartConversation ? <button className="my-work-start" onClick={props.onStartConversation} type="button">开始对话</button> : null}
    </header>

    <div className="my-work-toolbar">
      <SearchField ariaLabel="搜索个人工作项" className="my-work-search" onChange={setQuery} placeholder="搜索任务、项目、阶段或交付物" value={query}/>
      <label><span>项目</span><Select ariaLabel="按项目筛选" options={[{ value: "all", label: "全部项目" }, ...projects.map(([id, name]) => ({ value: id, label: name }))]} value={projectId} onChange={setProjectId}/></label>
      <label><span>优先级</span><Select ariaLabel="按优先级筛选" options={[{ value: "all", label: "全部优先级" }, { value: "critical", label: "紧急" }, { value: "high", label: "高" }, { value: "medium", label: "中" }, { value: "low", label: "低" }]} value={priority} onChange={setPriority}/></label>
      {props.onStartTask ? <button className="my-work-create-trigger" onClick={() => props.onStartTask?.(projectId === "all" ? null : projectId)} type="button">新建任务</button> : null}
    </div>

    {loading ? <div className="my-work-loading">正在加载任务…</div>
      : loadError ? <LoadFailure error={loadError} what="工作项" onRetry={() => setRetryNonce((value) => value + 1)}/> : <div className="my-work-board">
      {groups.map((group) => {
        const groupItems = filtered.filter((item) => item.focusGroup === group.id);
        return <section className="my-work-column" data-group={group.id} key={group.id}>
          <header><div><span><GroupIcon group={group.id}/></span><div><h3>{group.label}</h3></div></div><strong>{groupItems.length}</strong></header>
          <div className="my-work-list">{groupItems.map((item) => <WorkItemCard item={item} key={item.workItemId} disabled={openingItemId !== null} onDetail={() => group.id === "decision" && item.attentionCaseId ? setActiveDecision(item) : setActiveDetail(item)} onOpen={() => props.onOpenSource(item)} onOpenConversation={() => void openItemConversation(item)}/>)}{groupItems.length === 0 ? <div className="my-work-empty"><EmptyIcon/><span>{filtered.length ? "当前筛选下没有这类事项" : "暂无需要处理的事项"}</span></div> : null}</div>
        </section>;
      })}
    </div>}

    {props.recentConversations?.length ? <section className="my-work-recent" aria-label="继续对话"><h2>继续对话</h2><div>{props.recentConversations.map((conversation) => <button key={conversation.conversationId} onClick={() => props.onOpenConversation(conversation.conversationId)} type="button"><span><strong>{conversation.title}</strong><small>{conversation.projectName ?? "未分类"} · {conversation.running ? "运行中" : listMoment(conversation.updatedAt)}</small></span><svg aria-hidden="true" viewBox="0 0 20 20"><path d="M4 10h12m-5-5 5 5-5 5"/></svg></button>)}</div></section> : null}
    {props.recentFailure ? <LoadFailure error={props.recentFailure.error} what="最近对话" onRetry={props.recentFailure.retry}/> : null}

    {activeDetail ? <TaskDetailDrawer
      item={activeDetail}
      onClose={() => setActiveDetail(null)}
      onMessage={props.onMessage}
      onOpenConversation={(conversationId) => { props.onOpenConversation(conversationId); setActiveDetail(null); }}
      onOpenRun={(runId) => { props.onOpenRun(runId); setActiveDetail(null); }}
    /> : null}

    {activeDecision ? <DecisionDrawer
      item={activeDecision}
      onClose={() => setActiveDecision(null)}
      onMessage={props.onMessage}
      onOpenRun={(runId) => { props.onOpenRun(runId); setActiveDecision(null); }}
      onOpenSource={() => { props.onOpenSource(activeDecision); setActiveDecision(null); }}
      onResolved={() => { setActiveDecision(null); void load(); }}
      workspaceName={activeDecisionWorkspaceName}
    /> : null}
  </section>;
}

export function TaskDetailDrawer(props: { item: MyWorkItem; onClose: () => void; onMessage: MessageSink; onOpenConversation: (conversationId: string) => void; onOpenRun: (runId: string) => void }) {
  const [snapshot, setSnapshot] = useState<AiRuntimeQuerySnapshot>(() => emptyAiQuerySnapshot("work"));
  const [facts, setFacts] = useState<WorkDetailFacts>(emptyWorkDetailFacts);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<unknown>(null);
  const [preview, setPreview] = useState<TaskArtifactPreview | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const drawerRef = useModal<HTMLElement>({ onClose: props.onClose, active: preview === null });

  const loadDetail = useCallback(async () => {
    const [nextSnapshot, nextFacts] = await loadWorkDetailSnapshot(props.item);
    setSnapshot(nextSnapshot);
    setFacts(nextFacts);
  }, [props.item]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    void loadWorkDetailSnapshot(props.item)
      .then(([nextSnapshot, nextFacts]) => { if (!cancelled) { setSnapshot(nextSnapshot); setFacts(nextFacts); } })
      .catch((error) => {
        if (!cancelled) {
          setLoadError(error);
        }
      })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [props.item, props.onMessage]);

  const directRunIds = useMemo(() => new Set([props.item.blockingRunId, facts.attention?.agentRunId].filter((value): value is string => Boolean(value))), [facts.attention?.agentRunId, props.item.blockingRunId]);
  const archivedConversationIds = useMemo(() => new Set(snapshot.conversations.filter((item) => item.archived).map((item) => item.conversationId)), [snapshot.conversations]);
  const supersededRunIds = useMemo(() => new Set(snapshot.runs.filter((run) => run.superseded).map((run) => run.runId)), [snapshot.runs]);
  const visibleContributions = useMemo(() => facts.contributions.filter((item) => !archivedConversationIds.has(item.conversationId) && !supersededRunIds.has(item.runId)), [archivedConversationIds, facts.contributions, supersededRunIds]);
  const contributionRunIds = useMemo(() => new Set(visibleContributions.map((item) => item.runId)), [visibleContributions]);
  const contributionConversationIds = useMemo(() => new Set(visibleContributions.map((item) => item.conversationId)), [visibleContributions]);
  const directConversationIds = useMemo(() => new Set(snapshot.runs
    .filter((run) => directRunIds.has(run.runId) && !archivedConversationIds.has(run.conversationId))
    .map((run) => run.conversationId)), [archivedConversationIds, directRunIds, snapshot.runs]);
  const conversations = useMemo(() => snapshot.conversations
    .filter((conversation) => !conversation.archived && (facts.task
      ? contributionConversationIds.has(conversation.conversationId) || directConversationIds.has(conversation.conversationId)
      : conversationMatchesWorkItem(conversation, props.item)
        || conversation.context.projectId === props.item.projectId && directConversationIds.has(conversation.conversationId)))
    .sort((left, right) => right.updatedAt - left.updatedAt), [contributionConversationIds, directConversationIds, facts.task, props.item, snapshot.conversations]);
  const conversationIds = useMemo(() => new Set(conversations.map((conversation) => conversation.conversationId)), [conversations]);
  const runs = useMemo(() => snapshot.runs
    .filter((run) => !run.superseded && !archivedConversationIds.has(run.conversationId) && (facts.task
      ? contributionRunIds.has(run.runId) || directRunIds.has(run.runId)
      : conversationIds.has(run.conversationId) || directRunIds.has(run.runId)))
    .sort((left, right) => right.startedAt - left.startedAt), [archivedConversationIds, conversationIds, contributionRunIds, directRunIds, facts.task, snapshot.runs]);
  const progressEntries = useMemo(() => {
    if (!facts.task) return runs.map((run) => ({ contribution: null, run }));
    const entries: Array<{ contribution: BusinessTaskContributionItem | null; run: AiRuntimeQuerySnapshot["runs"][number] | null }> = visibleContributions.map((contribution) => ({
      contribution,
      run: runs.find((run) => run.runId === contribution.runId) ?? null,
    }));
    for (const run of runs) {
      if (directRunIds.has(run.runId) && !contributionRunIds.has(run.runId)) entries.push({ contribution: null, run });
    }
    return entries;
  }, [contributionRunIds, directRunIds, facts.task, runs, visibleContributions]);
  const taskEntries = useMemo(() => {
    const latestRunByConversation = new Map<string, string>();
    for (const contribution of visibleContributions) {
      if (!latestRunByConversation.has(contribution.conversationId)) {
        latestRunByConversation.set(contribution.conversationId, contribution.runId);
      }
    }
    const entries = progressEntries.filter((entry) => entry.contribution);
    return {
      current: entries.filter(({ contribution }) => contribution?.conversationStatus !== "completed"
        && latestRunByConversation.get(contribution!.conversationId) === contribution!.runId),
      history: entries.filter(({ contribution }) => contribution?.conversationStatus === "completed"
        || latestRunByConversation.get(contribution!.conversationId) !== contribution!.runId),
    };
  }, [progressEntries, visibleContributions]);
  const artifacts = useMemo(() => conversationFileArtifacts([...runs].reverse()).map(({ artifact, runId }) => ({ artifact, run: runs.find((run) => run.runId === runId)! })), [runs]);
  const textOutcomes = useMemo(() => progressEntries.filter((entry) => entry.run?.outputPreview?.trim() && !entry.run.artifacts.length), [progressEntries]);
  const deliverableVersions = useMemo(() => facts.deliverable
    ? [...facts.deliverable.versions].sort((left, right) => right.versionNo - left.versionNo)
    : [], [facts.deliverable]);
  const outcomeCount = artifacts.length + textOutcomes.length + deliverableVersions.length;
  const responsibility = detailResponsibility(
    facts.task?.assigneeType ?? props.item.responsibilityType,
    facts.task?.assigneeLabel ?? props.item.responsibilityLabel,
  );

  return <><button aria-label="关闭任务详情" className="workspace-drawer-backdrop" onClick={props.onClose} type="button"/><aside aria-label="任务详情" className="workspace-drawer my-work-detail-drawer" ref={drawerRef}>
    <header><div><p className="eyebrow">{[props.item.projectName, props.item.phaseName, groupLabel(props.item.focusGroup)].filter(Boolean).join(" · ")}</p><h2>{props.item.title}</h2></div><button aria-label="关闭侧边面板" onClick={props.onClose} type="button"><CloseIcon/></button></header>
    <div className="workspace-drawer-body">
      {loading ? <p className="drawer-empty">正在读取关联对话与产物…</p> : loadError ? <LoadFailure error={loadError} what="任务关联内容" onRetry={() => {
        setLoading(true);
        setLoadError(null);
        void loadDetail().catch(setLoadError).finally(() => setLoading(false));
      }}/> : <>
        {taskEntries.current.length ? <section className="work-detail-next"><strong>{props.item.focusGroup === "decision" ? "需要你决策" : props.item.focusGroup === "review" ? "查看成果并验收" : "继续推进"}</strong><p>{taskEntries.current[0].contribution?.outcomeSummary || taskEntries.current[0].contribution?.reason}</p><button type="button" onClick={() => props.onOpenConversation(taskEntries.current[0].contribution!.conversationId)}>继续对话</button></section> : null}
        <dl className="work-detail-overview">
          <div><dt>{facts.task ? "当前对话" : "对话"}</dt><dd>{facts.task ? taskEntries.current.length : conversations.length}</dd></div>
          <div><dt>{facts.task ? "历史记录" : "推进"}</dt><dd>{facts.task ? taskEntries.history.length : progressEntries.length}</dd></div>
          <div><dt>{facts.task ? "执行轮次" : "产出"}</dt><dd>{facts.task ? facts.task.attemptNo : outcomeCount}</dd></div>
          {facts.task ? <div><dt>产物</dt><dd>{artifacts.length}</dd></div> : null}
          {props.item.dueAt ? <div><dt>截止</dt><dd>{deadlineDay(props.item.dueAt)}</dd></div> : null}
          <div><dt>更新</dt><dd>{listMoment(props.item.updatedAt)}</dd></div>
        </dl>

        {facts.task ? <>
          <section className="work-detail-task-goal">
            <span>任务目标</span>
            <p>{facts.task.objective}</p>
          </section>
          <section className="work-detail-section work-detail-progress">
            <header><div><h3>当前对话与产出</h3></div><strong>{taskEntries.current.length}</strong></header>
            {taskEntries.current.length ? <TaskConversationEntries aggregate conversations={conversations} entries={taskEntries.current} onOpenConversation={props.onOpenConversation} onPreview={setPreview} runs={runs}/>
              : <p className="work-detail-empty">当前没有待处理对话。</p>}
          </section>
          {taskEntries.history.length ? <details className="work-detail-history" onToggle={(event) => setHistoryOpen(event.currentTarget.open)}>
            <summary><span>历史记录</span><small>{taskEntries.history.length}</small></summary>
            {historyOpen ? <TaskConversationEntries conversations={conversations} entries={taskEntries.history} onOpenConversation={props.onOpenConversation} onPreview={setPreview} runs={runs}/> : null}
          </details> : null}
        </> : null}

        {facts.phase && props.item.sourceType === "project_phase" ? <section className="work-detail-section work-detail-contract">
          <header><div><h3>阶段验收依据</h3></div></header>
          <div className="work-detail-objective"><span>阶段目标</span><p>{facts.phase.goal}</p></div>
          <div className="work-detail-contract-grid"><div className="criteria"><span>验收标准</span><p>{facts.phase.acceptanceCriteria}</p></div><div><span>要求交付物</span><p>{facts.phase.requiredDeliverableTitle ?? "尚未绑定"}</p></div></div>
        </section> : null}

        {facts.attention && props.item.sourceType === "project_blocker" ? <section className="work-detail-section work-detail-blocked"><h3>当前阻塞</h3><p>{facts.attention.detail}</p>{facts.attention.agentRunId ? <button onClick={() => props.onOpenRun(facts.attention!.agentRunId!)} type="button">打开对话</button> : null}</section>
          : props.item.blockingRunId ? <section className="work-detail-section work-detail-blocked"><h3>当前阻塞</h3><p>{props.item.blockingReason}</p><button onClick={() => props.onOpenRun(props.item.blockingRunId!)} type="button">打开对话</button></section> : null}

        {!facts.task ? <><section className="work-detail-section work-detail-progress">
          <header><div><h3>对话进展</h3></div><strong>{progressEntries.length}</strong></header>
          {progressEntries.length ? <div className="work-detail-progress-list">{progressEntries.map(({ contribution, run }) => {
            const conversationId = contribution?.conversationId ?? run?.conversationId;
            const conversation = conversations.find((item) => item.conversationId === conversationId);
            const request = run ? snapshot.messages.find((message) => message.messageId === run.inputMessageId) : null;
            const occurredAt = run?.completedAt ?? run?.startedAt ?? contribution?.createdAt;
            const result = run?.outputPreview?.trim() || contribution?.reason;
            return <article data-status={run?.status ?? "missing"} key={contribution?.runId ?? run?.runId}>
              <header>
                <div><strong>{conversation?.title ?? "对话记录不可用"}</strong><span>{[occurredAt ? listMoment(occurredAt) : null, run?.modelLabel ?? (run?.model ? displayModelId(run.model) : null), contribution?.stage ? groupLabel(contribution.stage) : null].filter(Boolean).join(" · ")}</span></div>
                <i>{run ? runStatusLabel(run.status) : "运行记录不可用"}</i>
              </header>
              {request?.content ? <p><span>要求</span>{compactWorkText(request.content, 160)}</p> : null}
              {result ? <p><span>结果</span>{compactWorkText(result, 180)}</p> : null}
              {run?.artifacts.length ? <div className="work-detail-progress-files">{run.artifacts.map((artifact) => <span key={artifact.artifactId}>{artifact.label}</span>)}</div> : null}
              {run || conversation ? <footer><button onClick={() => conversation ? props.onOpenConversation(conversation.conversationId) : props.onOpenRun(run!.runId)} type="button">打开对话</button></footer> : null}
            </article>;
          })}</div> : <p className="work-detail-empty">尚无对话进展。</p>}
        </section>

        <section className="work-detail-section work-detail-outcomes">
          <header><div><h3>当前产出</h3></div><strong>{outcomeCount}</strong></header>
          {facts.deliverable ? <article className="work-detail-deliverable">
            <header><div><span>目标交付物</span><strong>{facts.deliverable.title}</strong></div><i data-status={facts.deliverable.status}>{deliverableStatusLabel(facts.deliverable.status)}</i></header>
            <p><span>验收标准</span>{facts.deliverable.acceptanceCriteria}</p>
            {deliverableVersions.length ? <div className="work-detail-versions">{deliverableVersions.map((version) => <section className="work-detail-version" key={version.versionId}>
              <header><strong>V{version.versionNo}</strong><i data-status={version.status}>{deliverableVersionStatusLabel(version.status)}</i></header>
              <span>{version.summary}</span>
              {version.bindings.length ? <ul>{version.bindings.map((binding) => <li key={binding.localFileId}>{binding.relativePath}</li>)}</ul> : <small>该版本没有关联文件</small>}
            </section>)}</div> : <small>尚未提交可验收版本</small>}
          </article> : null}
          {artifacts.length ? <><div className="work-detail-subhead"><h4>候选文件</h4><span>{artifacts.length}</span></div><div className="work-detail-artifacts">{artifacts.map(({ artifact, run }) => {
            const sourceConversation = conversations.find((conversation) => conversation.conversationId === run.conversationId);
            return <article key={`${run.runId}-${artifact.artifactId}`}><ArtifactIcon/><div><strong>{artifact.label}</strong><span>{artifact.path ?? artifact.artifactType}</span><small>{[sourceConversation?.title, run.model, runStatusLabel(run.status), listMoment(run.completedAt ?? run.startedAt)].filter(Boolean).join(" · ")}</small></div><button onClick={() => sourceConversation ? props.onOpenConversation(sourceConversation.conversationId) : props.onOpenRun(run.runId)} type="button">打开对话</button></article>;
          })}</div></> : null}
          {textOutcomes.length ? <><div className="work-detail-subhead"><h4>文本结果</h4><span>{textOutcomes.length}</span></div><div className="work-detail-text-outcomes">{textOutcomes.map(({ contribution, run }) => {
            if (!run?.outputPreview) return null;
            const sourceConversation = conversations.find((conversation) => conversation.conversationId === (contribution?.conversationId ?? run.conversationId));
            return <article key={run.runId}><div><strong>{sourceConversation?.title ?? "对话结果"}</strong><p>{compactWorkText(run.outputPreview, 220)}</p><small>{[runStatusLabel(run.status), listMoment(run.completedAt ?? run.startedAt)].join(" · ")}</small></div><button onClick={() => sourceConversation ? props.onOpenConversation(sourceConversation.conversationId) : props.onOpenRun(run.runId)} type="button">打开对话</button></article>;
          })}</div></> : null}
          {!facts.deliverable && !outcomeCount ? <p className="work-detail-empty">尚无可核查产出。</p> : null}
        </section></> : null}

        {facts.task ? <details className="work-detail-definition" open>
          <summary><span>任务定义</span><small>第 {facts.task.attemptNo} 轮</small></summary>
          <div className="work-detail-definition-body">
            <div className="work-detail-contract-grid">
              <div><span>预期产出</span><p>{facts.task.expectedOutput}</p></div>
              <div className="criteria"><span>完成标准</span><p>{facts.task.completionCriteria}</p></div>
            </div>
            {facts.task.inputContext ? <div className="work-detail-objective"><span>输入</span><p>{facts.task.inputContext}</p></div> : null}
            <dl className="work-detail-definition-meta"><div><dt>{responsibility.label}</dt><dd>{responsibility.value}</dd></div><div><dt>优先级</dt><dd>{priorityLabel[props.item.priority] ?? props.item.priority}</dd></div></dl>
          </div>
        </details> : null}
      </>}
    </div>
  </aside>{preview ? <ImageArtifactLightbox activeIndex={0} artifacts={[preview.artifact]} onChange={() => {}} onClose={() => setPreview(null)} previews={{ [preview.artifact.artifactId]: preview.url }}/> : null}</>;
}

type TaskConversationEntry = {
  contribution: BusinessTaskContributionItem | null;
  run: AiRuntimeQuerySnapshot["runs"][number] | null;
};

function TaskConversationEntries(props: {
  conversations: AiRuntimeQuerySnapshot["conversations"];
  entries: TaskConversationEntry[];
  aggregate?: boolean;
  runs: AiRuntimeQuerySnapshot["runs"];
  onPreview: (preview: TaskArtifactPreview) => void;
  onOpenConversation: (conversationId: string) => void;
}) {
  return <div className="work-detail-progress-list">{props.entries.map(({ contribution, run }) => {
    if (!contribution) return null;
    const conversation = props.conversations.find((item) => item.conversationId === contribution.conversationId);
    const relatedRuns = props.aggregate ? props.runs.filter((item) => item.conversationId === contribution.conversationId).slice().reverse() : run ? [run] : [];
    const artifacts = conversationFileArtifacts(relatedRuns);
    const missingRun = props.aggregate
      ? props.entries.some((entry) => entry.contribution?.conversationId === contribution.conversationId && !entry.run)
      : !run;
    return <article className="work-detail-task-conversation" data-status={contribution.conversationStatus} key={contribution.runId}>
      <button aria-label={`打开对话：${conversation?.title ?? "对话记录不可用"}`} disabled={!conversation} onClick={() => { if (conversation) props.onOpenConversation(conversation.conversationId); }} type="button">
        <strong>{conversation?.title ?? "对话记录不可用"}</strong>
        <span>{artifacts.length ? `${artifacts.length} 份产物` : missingRun ? "运行记录不可用" : "暂无产物"}</span>
      </button>
      {artifacts.length ? <TaskArtifactList entries={artifacts} onPreview={props.onPreview}/> : null}
    </article>;
  })}</div>;
}

interface WorkDetailFacts {
  task: BusinessTaskItem | null;
  contributions: BusinessTaskContributionItem[];
  deliverable: DeliverableItem | null;
  phase: ProjectPhaseItem | null;
  attention: AttentionCaseItem | null;
}

const emptyWorkDetailFacts: WorkDetailFacts = { task: null, contributions: [], deliverable: null, phase: null, attention: null };

async function loadWorkItemConversationId(item: MyWorkItem): Promise<string> {
  const [snapshot, taskResult, attentionResult] = await Promise.all([
    loadAiRuntimeQuery({ scope: "project", projectId: item.projectId }),
    !item.blockingRunId && !item.attentionCaseId && item.taskId
      ? rpc<BusinessTaskGetResult>("businessTask/get", { businessTaskId: item.taskId }) : Promise.resolve(null),
    !item.blockingRunId && item.attentionCaseId
      ? rpc<AttentionCaseListResult>("attentionCase/list", { projectId: item.projectId }) : Promise.resolve(null),
  ]);
  const conversations = snapshot.conversations.filter((conversation) => !conversation.archived && conversation.context.projectId === item.projectId);
  if (item.blockingRunId || item.attentionCaseId) {
    const runId = item.blockingRunId ?? attentionResult?.cases.find((entry) => entry.attentionCaseId === item.attentionCaseId)?.agentRunId;
    const run = snapshot.runs.find((entry) => entry.runId === runId && !entry.superseded);
    const conversation = conversations.find((entry) => entry.conversationId === run?.conversationId);
    if (conversation) return conversation.conversationId;
  } else if (item.taskId) {
    if (!taskResult?.task) throw new Error("未找到该工作项对应的正式任务。");
    const latestByConversation = new Map<string, BusinessTaskContributionItem>();
    for (const contribution of [...taskResult.contributions].sort((left, right) => right.createdAt - left.createdAt)) {
      if (!latestByConversation.has(contribution.conversationId)) latestByConversation.set(contribution.conversationId, contribution);
    }
    const contribution = [...latestByConversation.values()].find((entry) => entry.conversationStatus === "active"
      && entry.stage === item.focusGroup
      && conversations.some((conversation) => conversation.conversationId === entry.conversationId)
      && snapshot.runs.some((run) => run.runId === entry.runId && run.conversationId === entry.conversationId && !run.superseded));
    if (contribution) return contribution.conversationId;
  } else {
    const conversation = conversations.filter((entry) => conversationMatchesWorkItem(entry, item))
      .sort((left, right) => right.updatedAt - left.updatedAt)[0];
    if (conversation) return conversation.conversationId;
  }
  throw new Error("未找到该事项当前可打开的关联对话。");
}

async function loadWorkDetailSnapshot(item: MyWorkItem): Promise<[AiRuntimeQuerySnapshot, WorkDetailFacts]> {
  const facts = await loadWorkDetailFacts(item);
  const runIds = facts.task ? [...new Set([
    ...facts.contributions.map((contribution) => contribution.runId),
    item.blockingRunId, facts.attention?.agentRunId,
  ].filter((id): id is string => Boolean(id)))] : undefined;
  const snapshot = await loadAiRuntimeQuery({ scope: "work", projectId: item.projectId, runIds });
  return [snapshot, facts];
}

async function loadWorkDetailFacts(item: MyWorkItem): Promise<WorkDetailFacts> {
  const [taskResult, deliverableResult, progressResult, attentionResult] = await Promise.all([
    item.taskId ? rpc<BusinessTaskGetResult>("businessTask/get", { businessTaskId: item.taskId }) : Promise.resolve(null),
    item.deliverableId ? rpc<DeliverableListResult>("deliverable/list", { projectId: item.projectId }) : Promise.resolve(null),
    item.phaseId ? rpc<ProjectProgressGetResult>("projectProgress/get", { projectId: item.projectId }) : Promise.resolve(null),
    item.attentionCaseId ? rpc<AttentionCaseListResult>("attentionCase/list", { projectId: item.projectId }) : Promise.resolve(null),
  ]);
  const task = taskResult?.task ?? null;
  const contributions = taskResult?.contributions ?? [];
  const deliverable = deliverableResult?.deliverables.find((entry) => entry.deliverableId === item.deliverableId) ?? null;
  const phase = progressResult?.plans.flatMap((plan) => plan.phases).find((entry) => entry.phaseId === item.phaseId) ?? null;
  const attention = attentionResult?.cases.find((entry) => entry.attentionCaseId === item.attentionCaseId) ?? null;
  if (item.taskId && !task) throw new Error("未找到该工作项对应的正式任务合同。");
  if (item.deliverableId && !deliverable) throw new Error("未找到该工作项对应的正式交付物。");
  if (item.phaseId && !phase) throw new Error("未找到该工作项对应的项目阶段。");
  if (item.attentionCaseId && !attention) throw new Error("未找到该工作项对应的阻塞或决策记录。");
  return { task, contributions, deliverable, phase, attention };
}

function conversationMatchesWorkItem(conversation: AiRuntimeQuerySnapshot["conversations"][number], item: MyWorkItem) {
  if (conversation.context.projectId !== item.projectId) return false;
  if (item.taskId) return conversation.context.taskId === item.taskId || conversation.workClassification?.taskId === item.taskId;
  if (item.deliverableId) return conversation.context.deliverableId === item.deliverableId;
  if (item.phaseId) return conversation.context.phaseId === item.phaseId;
  return false;
}

function compactWorkText(value: string, limit: number) {
  let content = value;
  try {
    const parsed = JSON.parse(value) as { reply?: unknown };
    if (typeof parsed.reply === "string") content = parsed.reply;
  } catch {
    // 普通回复不是 JSON，直接生成纯文本摘要。
  }
  const normalized = content
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/^[-*_]{3,}\s*$/gm, "")
    .replace(/[*_`>|]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  return normalized.length > limit ? `${normalized.slice(0, limit).trimEnd()}…` : normalized;
}

function detailResponsibility(type: string, value: string) {
  if (["agent", "ai", "model"].includes(type)) return { label: "协作方式", value: "AI 对话协作" };
  if (type === "collaboration") return { label: "协作方式", value };
  return { label: "负责人", value };
}

function deliverableStatusLabel(status: string) {
  return status === "accepted" ? "已验收" : status === "revision_needed" ? "需修改" : status === "candidate" ? "待验收" : "已定义";
}

function deliverableVersionStatusLabel(status: string) {
  return status === "accepted" ? "已验收" : status === "rejected" ? "已退回" : "候选版本";
}

function groupLabel(group: string) {
  return groups.find((item) => item.id === group)?.label ?? group;
}

function runStatusLabel(status: string) {
  if (status === "completed") return "运行完成";
  if (["queued", "running", "paused", "terminating"].includes(status)) return "运行中";
  if (["waiting_permission", "permission_approved"].includes(status)) return "等待授权";
  if (status === "terminated") return "运行已终止";
  if (status === "interrupted") return "运行已中断";
  return "运行失败";
}

function DecisionDrawer(props: { item: MyWorkItem; workspaceName: string | null; onClose: () => void; onMessage: MessageSink; onOpenSource: () => void; onOpenRun: (runId: string) => void; onResolved: () => void }) {
  const [decision, setDecision] = useState<AttentionCaseItem | null>(null);
  const [detailLoading, setDetailLoading] = useState(true);
  const [actor, setActor] = useState("");
  const [resolution, setResolution] = useState("");
  const [decisionAction, setDecisionAction] = useState("approved");
  const [delegatedTo, setDelegatedTo] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setDetailLoading(true);
    void rpc<AttentionCaseListResult>("attentionCase/list", { projectId: props.item.projectId })
      .then((result) => { if (!cancelled) setDecision(result.cases.find((entry) => entry.attentionCaseId === props.item.attentionCaseId) ?? null); })
      .catch((error) => { if (!cancelled) props.onMessage(String(error), "error"); })
      .finally(() => { if (!cancelled) setDetailLoading(false); });
    return () => { cancelled = true; };
  }, [props.item.attentionCaseId, props.item.projectId, props.onMessage]);

  const drawerRef = useModal<HTMLElement>({ onClose: props.onClose });

  const submit = async () => {
    if (!decision || !actor.trim() || !resolution.trim() || decisionAction === "delegated" && !delegatedTo.trim() || busy) return;
    setBusy(true);
    try {
      const clientOperationId = `desktop-decision-resolve-${randomUuid()}`;
      const targetStatus = decisionAction === "delegated" ? "open" : "resolved";
      const requestFingerprint = await sha256([decision.attentionCaseId, String(decision.stateRevision), targetStatus, actor.trim(), resolution.trim(), decisionAction, delegatedTo.trim()].join("\n"));
      await rpc<AttentionCaseTransitionResult>("attentionCase/transition", {
        attentionCaseId: decision.attentionCaseId,
        targetStatus,
        expectedRevision: decision.stateRevision,
        actorLabel: actor.trim(),
        resolutionSummary: resolution.trim(),
        decisionAction,
        delegatedTo: decisionAction === "delegated" ? delegatedTo.trim() : null,
        clientOperationId,
        requestFingerprint,
      });
      props.onMessage(decisionAction === "delegated" ? "决策已转交，新的责任人和审计信息已回写项目事实。" : "决策已提交，结论和审计信息已回写项目事实。");
      props.onResolved();
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  return <><button aria-label="关闭决策处理" className="workspace-drawer-backdrop" onClick={props.onClose} type="button"/><aside aria-label="处理决策" className="workspace-drawer" ref={drawerRef}>
    <header><div><p className="eyebrow">{props.item.projectName}{props.workspaceName ? ` · ${props.workspaceName}` : ""}</p><h2>{props.item.title}</h2></div><button aria-label="关闭侧边面板" onClick={props.onClose} type="button"><CloseIcon/></button></header>
    <div className="workspace-drawer-body">
      {detailLoading ? <p className="drawer-empty">正在加载决策详情…</p> : !decision ? <p className="drawer-empty">未找到该决策的详情，可能已被处理或删除。</p> : <>
        <p className="decision-drawer-question">{decision.detail}</p>
        <dl className="decision-drawer-detail">
          <div><dt>可选方案</dt><dd>{decision.decisionOptions}</dd></div>
          <div><dt>影响范围</dt><dd>{decision.decisionImpact}</dd></div>
          <div><dt>证据</dt><dd>{decision.decisionEvidence}</dd></div>
          <div className="recommended"><dt>建议</dt><dd>{decision.decisionRecommendation}</dd></div>
          <div><dt>不决定的后果</dt><dd>{decision.decisionConsequence}</dd></div>
        </dl>
        {decision.status === "resolved" ? <blockquote className="decision-drawer-resolved"><strong>{decision.decisionAction === "delegated" ? `已转交给 ${decision.decisionDelegatedTo}` : "已处理"}</strong>{decision.decisionActionSummary ?? decision.resolutionSummary}</blockquote> : <section className="decision-resolve-form">
          <label>操作人<input onChange={(event) => setActor(event.target.value)} placeholder="记录谁做出决策" value={actor}/></label>
          <label>决策动作<Select options={decisionActionOptions} value={decisionAction} onChange={setDecisionAction}/></label>
          {decisionAction === "delegated" ? <label>转交给<input onChange={(event) => setDelegatedTo(event.target.value)} placeholder="新的决策负责人" value={delegatedTo}/></label> : null}
          <label>决策说明<textarea onChange={(event) => setResolution(event.target.value)} placeholder="说明选择、理由与后续行动" rows={3} value={resolution}/></label>
          <footer>
            <button onClick={props.onOpenSource} type="button">查看项目上下文</button>
            {decision.agentRunId ? <button onClick={() => props.onOpenRun(decision.agentRunId!)} type="button">打开对话</button> : null}
            <button className="primary" disabled={!actor.trim() || !resolution.trim() || decisionAction === "delegated" && !delegatedTo.trim() || busy} onClick={() => void submit()} type="button">{busy ? "正在提交…" : decisionAction === "delegated" ? "确认转交" : "提交决策"}</button>
          </footer>
        </section>}
      </>}
    </div>
  </aside></>;
}

function WorkItemCard(props: { item: MyWorkItem; disabled: boolean; onDetail: () => void; onOpen: () => void; onOpenConversation: () => void }) {
  const due = props.item.dueAt === null ? null : deadlineDay(props.item.dueAt);
  const overdue = props.item.dueAt !== null && props.item.dueAt < Date.now();
  const blocked = Boolean(props.item.blockingRunId);
  const action = blocked ? "查看阻塞" : props.item.focusGroup === "decision" ? "确认决策" : props.item.focusGroup === "review" ? "查看成果" : "继续处理";
  return <article className="my-work-card">
    <button className="my-work-project" onClick={props.onOpen} type="button"><ProjectIcon/><span>{props.item.projectName}</span></button>
    <h4><button className="my-work-title" onClick={props.onDetail} title={props.item.title} type="button">{props.item.title}</button></h4>
    <time className="my-work-updated">{listMoment(props.item.updatedAt)}</time>
    {blocked ? <p className="my-work-blocked">执行受阻，需要查看原因</p> : null}
    <footer>
      <div className="my-work-signals">
        {props.item.priority === "critical" ? <span className="my-work-urgent">紧急</span> : null}
        {due ? <span className={overdue ? "my-work-overdue" : "my-work-due"}>{overdue ? "已逾期 · " : "截止 "}{due}</span> : null}
      </div>
      <button className="my-work-primary" disabled={props.disabled} onClick={props.onOpenConversation} type="button">{action}</button>
    </footer>
  </article>;
}

function GroupIcon({ group }: { group: string }) {
  if (group === "todo") return <svg aria-hidden="true" viewBox="0 0 20 20"><circle cx="10" cy="10" r="7"/><path d="M10 6v4l3 2"/></svg>;
  if (group === "in_progress") return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M10 3a7 7 0 1 1-5 2"/><path d="M3 3v5h5"/></svg>;
  if (group === "review") return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M4 3h12v14H4zM7 7h6M7 10h6"/><path d="m7 14 1.5 1.5L12 12"/></svg>;
  return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M10 2.5 17 6v5c0 3.5-2.5 5.5-7 7-4.5-1.5-7-3.5-7-7V6z"/><path d="M10 6v5M10 14h.01"/></svg>;
}
function ProjectIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M3 5h6l1.5 2H17v9H3z"/></svg>; }
function ArtifactIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M4 2.5h8l4 4v11H4zM12 2.5v4h4M7 10h6M7 13h4"/></svg>; }
function EmptyIcon() { return <svg aria-hidden="true" viewBox="0 0 32 32"><path d="M7 8h18v17H7zM11 13h10M11 18h7"/></svg>; }
function CloseIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="m5 5 10 10M15 5 5 15"/></svg>; }
