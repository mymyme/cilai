import { useModelCatalog, ModelCatalogNotice } from "./use-model-catalog";
import { reuseInstructions, type WorkReuseIntent } from "./work-reuse";
import { useEffect, useMemo, useRef, useState } from "react";
import type { WorkspaceSummary } from "./generated/protocol";
import type { AiProjectOption } from "./ai-workbench";
import { aiErrorMessage, deleteAiAutomationRule, aiRuntimeChangedEvent, emptyAiQuerySnapshot, loadAiRuntimeQuery, runAiAutomationRule, saveAiAutomationRule, type AiAutomationRule, type AiRuntimeQuerySnapshot } from "./ai-runtime";
import { AutomationDialog, minutesToTime, type AutomationDraft } from "./automation-dialog";
import type { MessageSink } from "./app-message";
import { auditMoment } from "./datetime";
import { SearchField } from "./search-field";
import { ConfirmDialog } from "./confirm-dialog";
import "./automations.css";
import { AutomationTriggerHistory } from "./automation-trigger-history";

type AutomationFilter = "all" | "enabled" | "paused" | "attention";

const filters: Array<{ value: AutomationFilter; label: string }> = [
  { value: "all", label: "全部" },
  { value: "enabled", label: "已启用" },
  { value: "paused", label: "已暂停" },
  { value: "attention", label: "需处理" },
];

const suggestions: Array<{ name: string; description: string; actionPrompt: string; scheduleKind: AutomationDraft["scheduleKind"]; intervalMinutes: number; timeOfDayMinutes: number; weekday: number }> = [
  {
    name: "每日工作摘要",
    description: "每天 09:00 梳理当前工作区的近期变更、待办与风险",
    actionPrompt: "梳理当前工作区最近一天的文件变更、未完成事项和需要关注的风险，输出一份简明工作摘要。只引用可核实的本地事实，不修改项目状态。",
    scheduleKind: "daily",
    intervalMinutes: 60,
    timeOfDayMinutes: 9 * 60,
    weekday: 1,
  },
  {
    name: "每周进展回顾",
    description: "每周五 16:00 汇总本周成果、阻塞与下周重点",
    actionPrompt: "基于当前工作区本周的真实记录，汇总已完成成果、尚未解决的阻塞和下周建议重点，标明证据来源，输出候选周报。",
    scheduleKind: "weekly",
    intervalMinutes: 60,
    timeOfDayMinutes: 16 * 60,
    weekday: 5,
  },
  {
    name: "工作区变更检查",
    description: "每 4 小时检查新增材料和需要人工确认的变化",
    actionPrompt: "检查当前工作区最近的新增或变更材料，识别可能影响现有结论、交付物或待办的内容，输出需要人工确认的候选清单。",
    scheduleKind: "interval",
    intervalMinutes: 240,
    timeOfDayMinutes: 9 * 60,
    weekday: 1,
  },
];

export function AutomationsPage(props: { openIntent?: {ruleId:string;token:string}|null; onOpenIntentConsumed?: (token:string)=>void; reuseIntent?: WorkReuseIntent | null; onReuseConsumed?: () => void; workspace: WorkspaceSummary | null; project: { projectId: string; displayName: string } | null; projects?: AiProjectOption[]; onMessage: MessageSink; onOpenRun: (runId: string) => void }) {
  const [snapshot, setSnapshot] = useState<AiRuntimeQuerySnapshot>(() => emptyAiQuerySnapshot("automations"));
  const [snapshotLoaded, setSnapshotLoaded] = useState(false);
  const [snapshotError, setSnapshotError] = useState<string | null>(null);
  const catalog = useModelCatalog(snapshotLoaded);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const automationSurface = useRef<HTMLElement>(null);
  const consumedOpen = useRef<string|null>(null);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<AutomationFilter>("all");
  const [draft, setDraft] = useState<AutomationDraft | null>(null);
  const [busy, setBusy] = useState(false);
  const [triggerRevision,setTriggerRevision]=useState(0);
  const [pendingDelete, setPendingDelete] = useState<AiAutomationRule | null>(null);
  const deleting = useRef(false);
  const refreshGeneration = useRef(0);
  const consumedReuse = useRef<string | null>(null);
  useEffect(() => {
    const intent = props.reuseIntent;
    if (!intent || consumedReuse.current === intent.token) return;
    consumedReuse.current = intent.token;
    setDraft({ automationRuleId: null, name: intent.title, actionPrompt: reuseInstructions(intent), scheduleKind: "daily", intervalMinutes: 60, timeOfDayMinutes: 9 * 60, weekday: 1, weekdays: [1], notifyOnEveryRun: false, agentId: intent.agentId, modelId: intent.modelId, projectId: intent.projectId, executionMode: "existing", conversationId: intent.conversationId, enabled: false });
    props.onReuseConsumed?.();
  }, [props.reuseIntent]);

  const hasWorkspace = Boolean(props.workspace?.localRoot || props.projects?.some((project) => project.primaryWorkspaceRoot));

  const refresh = async () => {
    const generation = ++refreshGeneration.current;
    try {
      const result = await loadAiRuntimeQuery({ scope: "automations" });
      if (generation !== refreshGeneration.current) return;
      setSnapshot(result);
      setSnapshotLoaded(true);
      setSnapshotError(null);
      setSelectedId((current) => current && result.automationRules.some((item) => item.automationRuleId === current) ? current : null);
    } catch (error) {
      if (generation === refreshGeneration.current) setSnapshotError(aiErrorMessage(error));
    }
  };

  useEffect(() => {
    void refresh();
    const changed = () => { void refresh(); };
    window.addEventListener(aiRuntimeChangedEvent, changed);
    return () => {
      refreshGeneration.current += 1;
      window.removeEventListener(aiRuntimeChangedEvent, changed);
    };
  }, [props.workspace?.workspaceId]);

  useEffect(() => {
    const intent = props.openIntent;
    if (!intent || !snapshotLoaded || consumedOpen.current === intent.token) return;
    consumedOpen.current = intent.token;
    if (snapshot.automationRules.some(rule => rule.automationRuleId === intent.ruleId)) {
      setQuery(""); setFilter("all"); setSelectedId(intent.ruleId);
    } else props.onMessage("自动化已不可用。", "error");
    props.onOpenIntentConsumed?.(intent.token);
  }, [props.openIntent, snapshotLoaded, snapshot.automationRules, props.onMessage, props.onOpenIntentConsumed]);
  useEffect(() => {
    const row = [...(automationSurface.current?.querySelectorAll<HTMLElement>("[data-automation-id]") ?? [])].find(item => item.dataset.automationId === selectedId);
    row?.scrollIntoView?.({ block: "center" });
  }, [selectedId]);

  const visibleRules = useMemo(
    () => filterAutomationRules(snapshot.automationRules, query, filter),
    [filter, query, snapshot.automationRules],
  );

  const create = (preset?: typeof suggestions[number]) => {
    if (!snapshot.agents[0]) {
      props.onMessage("请先选择可用的模型。", "error");
      return;
    }
    setDraft({
      automationRuleId: null,
      name: preset?.name ?? "",
      actionPrompt: preset?.actionPrompt ?? "",
      scheduleKind: preset?.scheduleKind ?? "daily",
      intervalMinutes: preset?.intervalMinutes ?? 60,
      timeOfDayMinutes: preset?.timeOfDayMinutes ?? 9 * 60,
      weekday: preset?.weekday ?? 1,
      weekdays: [preset?.weekday ?? 1],
      notifyOnEveryRun: false,
      agentId: snapshot.agents[0].agentId,
      modelId: catalog.models[0]?.modelId ?? null,
      projectId: props.project?.projectId ?? null,
      executionMode: "new",
      conversationId: null,
      enabled: true,
    });
  };

  const edit = (rule: AiAutomationRule) => setDraft(toDraft(rule));

  const save = async () => {
    if (!draft) return;
    const project = props.projects?.find((item) => item.projectId === draft.projectId);
    if (!project?.primaryWorkspaceRoot || !project.primaryWorkspaceId || !project.primaryWorkspaceName) {
      props.onMessage("请先选择已连接文件夹的项目。", "error");
      return;
    }
    const target = draft.executionMode === "existing" ? snapshot.conversations.find((item) => item.conversationId === draft.conversationId && item.context.projectId === project.projectId && !item.archived) : null;
    if (draft.executionMode === "existing" && !target) {
      props.onMessage("请选择该项目下的已有对话。", "error");
      return;
    }
    setBusy(true);
    try {
      const saved = await saveAiAutomationRule({
        automationRuleId: draft.automationRuleId,
        name: draft.name,
        actionPrompt: draft.actionPrompt,
        scheduleKind: draft.scheduleKind,
        intervalMinutes: draft.intervalMinutes,
        timeOfDayMinutes: draft.timeOfDayMinutes,
        weekday: draft.weekday,
        weekdays: draft.weekdays,
        notifyOnEveryRun: draft.notifyOnEveryRun,
        agentId: draft.agentId,
        modelId: draft.modelId,
        enabled: draft.enabled,
        conversationId: target?.conversationId ?? null,
        conditionSummary: scheduleSummary(draft),
        workspaceId: target?.context.workspaceId ?? project.primaryWorkspaceId,
        workspaceName: target?.context.workspaceName ?? project.primaryWorkspaceName,
        workspaceRoot: target?.context.workspaceRoot ?? project.primaryWorkspaceRoot,
        projectId: project.projectId,
        projectName: project.displayName,
      });
      await refresh();
      setSelectedId(saved.automationRuleId);
      setDraft(null);
      props.onMessage("自动化已保存；只有启用的规则会进入本机调度。");
    } catch (error) {
      props.onMessage(aiErrorMessage(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const run = async (rule: AiAutomationRule) => {
    setBusy(true);
    try {
      const result = await runAiAutomationRule(rule.automationRuleId);
      await refresh();
      props.onMessage("自动化运行已进入队列。");
      props.onOpenRun(result.runId);
    } catch (error) {
      props.onMessage(aiErrorMessage(error), "error");
    } finally {
      setBusy(false);
      setTriggerRevision(value=>value+1);
    }
  };

  const confirmDelete = async () => {
    if (!pendingDelete || deleting.current) return;
    deleting.current = true;
    setBusy(true);
    try {
      await deleteAiAutomationRule(pendingDelete.automationRuleId);
      refreshGeneration.current += 1;
      setSnapshot((current) => ({ ...current, automationRules: current.automationRules.filter((rule) => rule.automationRuleId !== pendingDelete.automationRuleId) }));
      setSelectedId((current) => current === pendingDelete.automationRuleId ? null : current);
      setPendingDelete(null);
      props.onMessage("自动化已删除。");
    } catch (error) {
      props.onMessage(`删除失败：${aiErrorMessage(error)}`, "error");
    } finally {
      deleting.current = false;
      setBusy(false);
    }
  };

  const enabledCount = snapshot.automationRules.filter((item) => item.enabled).length;
  const attentionCount = snapshot.automationRules.filter((item) => Boolean(item.lastError)).length;

  return <section className="automations-page automation-schedule-page" ref={automationSurface}>
    <header className="automation-page-header">
      <div>
        <h1>自动化</h1>
        <p>让智能体按计划执行只读任务，结果保持为候选内容，等待你确认。</p>
      </div>
      <button className="automation-create" disabled={busy || !hasWorkspace} onClick={() => create()} title={hasWorkspace ? undefined : "请先连接项目文件夹"} type="button"><PlusIcon/>新建自动化</button>
    </header>

    <main className="automation-schedule-content">
      <SearchField ariaLabel="搜索已安排任务" className="automation-search" clearable clearLabel="清除搜索" onChange={setQuery} placeholder="搜索已安排任务" value={query}/>

      <div className="automation-list-toolbar">
        <nav aria-label="自动化状态筛选">
          {filters.map((item) => <button aria-pressed={filter === item.value} className={filter === item.value ? "active" : ""} key={item.value} onClick={() => setFilter(item.value)} type="button">{item.label}</button>)}
        </nav>
        <span>{snapshot.automationRules.length} 条任务 · {enabledCount} 条已启用{attentionCount ? ` · ${attentionCount} 条需处理` : ""}</span>
      </div>

      <section aria-label="已安排任务列表" className="automation-rule-list">
        {visibleRules.map((rule) => {
          const selected = selectedId === rule.automationRuleId;
          const localBlocked = !rule.modelId;
          const runs = snapshot.runs
            .filter((item) => item.automationRuleId === rule.automationRuleId)
            .sort((left, right) => right.startedAt - left.startedAt);
          return <article className={`automation-rule${selected ? " selected" : ""}`} data-status={automationStatus(rule)} data-automation-id={rule.automationRuleId} key={rule.automationRuleId}>
            <div className="automation-rule-row">
              <button aria-expanded={selected} className="automation-rule-open" onClick={() => setSelectedId(selected ? null : rule.automationRuleId)} type="button">
                <span className="automation-rule-state"><StatusIcon status={automationStatus(rule)}/></span>
                <span className="automation-rule-copy">
                  <strong>{rule.name}</strong>
                  <small>{scheduleLabel(rule)} · {localBlocked ? "未选择模型" : nextRunSummary(rule)}</small>
                </span>
                <span className="automation-rule-agent">{rule.modelId ? catalog.models.find((model) => model.modelId === rule.modelId)?.label ?? rule.modelId : snapshot.agents.find((agent) => agent.agentId === rule.agentId)?.name ?? rule.agentId}</span>
                <span className="automation-rule-status"><i/>{automationStatusLabel(rule)}</span>
              </button>
              <button aria-label={`删除${rule.name}`} className="automation-rule-delete" disabled={busy} onClick={() => setPendingDelete(rule)} title="删除" type="button"><DeleteIcon/></button>
              <button aria-label={`编辑${rule.name}`} className="automation-rule-edit" disabled={busy} onClick={() => edit(rule)} title="编辑" type="button"><EditIcon/></button>
              <button aria-expanded={selected} aria-label={`${selected ? "收起" : "展开"}${rule.name}`} className="automation-rule-toggle" onClick={() => setSelectedId(selected ? null : rule.automationRuleId)} title={selected ? "收起" : "展开"} type="button"><ChevronIcon/></button>
            </div>

            {selected ? <section className="automation-detail">
              <div className="automation-detail-actions">
                <button className="primary" disabled={busy || localBlocked || !hasWorkspace} onClick={() => void run(rule)} type="button"><PlayIcon/>立即运行</button>
              </div>
              <blockquote className="automation-detail-prompt">{rule.actionPrompt}</blockquote>
              <dl className="automation-detail-facts">
                <div><dt>项目</dt><dd>{rule.projectName ?? "请选择项目"}</dd></div>
                <div><dt>对话</dt><dd>{rule.conversationId ? snapshot.conversations.find((item) => item.conversationId === rule.conversationId)?.title ?? "对话不可用" : "新对话"}</dd></div>
                <div><dt>下次运行</dt><dd>{localBlocked ? "未选择模型" : rule.nextRunAt ? auditMoment(rule.nextRunAt) : "未排期"}</dd></div>
              </dl>
              <AutomationTriggerHistory ruleId={rule.automationRuleId} refreshToken={`${snapshot.stateRevision}:${triggerRevision}`} availableRunIds={runs.map(item=>item.runId)} onOpenRun={props.onOpenRun} onReconciled={refresh}/>
              <section className="automation-audit">
                <header><h3>运行历史</h3><span>{runs.length ? `${runs.length} 条记录` : "暂无记录"}</span></header>
                {runs.map((item) => <button key={item.runId} onClick={() => props.onOpenRun(item.runId)} type="button"><RunsIcon/><span><strong>{runStatusLabel(item.status)} · 规则 v{item.automationRuleVersion}</strong><small>{auditMoment(item.startedAt)}</small>{item.errorReason ? <em>{item.errorReason}</em> : null}</span><ArrowIcon/></button>)}
                {!runs.length ? <p>{rule.lastError ?? "该自动化还没有运行记录。"}</p> : null}
              </section>
            </section> : null}
          </article>;
        })}
        {!visibleRules.length && snapshotLoaded && !snapshotError ? <div className="automation-list-empty">
          <SearchEmptyIcon/>
          <strong>{snapshot.automationRules.length ? "没有符合条件的任务" : "还没有已安排任务"}</strong>
          <span>{snapshot.automationRules.length ? "调整搜索词或状态筛选后再试。" : hasWorkspace ? "从下方建议开始，或新建一条自己的自动化。" : "连接项目文件夹后可新建自动化。"}</span>
        </div> : null}
      </section>

      {!query && filter === "all" && hasWorkspace ? <section className="automation-suggestions">
        <header><h2>建议自动化</h2><p>选择后仍可在保存前修改指令、智能体与运行时间。</p></header>
        <div>{suggestions.map((item) => <button disabled={busy} key={item.name} onClick={() => create(item)} type="button"><SuggestionIcon kind={item.scheduleKind}/><span><strong>{item.name}</strong><small>{item.description}</small></span><PlusIcon/></button>)}</div>
      </section> : null}
    </main>

    {pendingDelete ? <ConfirmDialog
      title={`删除“${pendingDelete.name}”？`}
      description="删除后不再自动执行，已有对话和运行记录保留。已入队或正在执行的一轮将继续完成。"
      confirmLabel="删除"
      danger
      busy={busy}
      onCancel={() => setPendingDelete(null)}
      onConfirm={() => void confirmDelete()}/> : null}

    <ModelCatalogNotice catalog={catalog}/>
    {snapshotError ? <div role="alert">{snapshotError}<button type="button" onClick={() => void refresh()}>重试</button></div> : null}
    {draft ? <AutomationDialog
      agents={snapshot.agents}
      busy={busy}
      draft={draft}
      models={catalog.models}
      onOpenModels={catalog.refreshOnOpen}
      projects={props.projects ?? []}
      conversations={snapshot.conversations}
      onCancel={() => setDraft(null)}
      onDraft={setDraft}
      onSave={() => void save()}/> : null}
  </section>;
}

export function filterAutomationRules(rules: AiAutomationRule[], query: string, filter: AutomationFilter): AiAutomationRule[] {
  const normalizedQuery = query.trim().toLocaleLowerCase("zh-CN");
  return rules.filter((rule) => {
    const matchesStatus = filter === "all"
      || (filter === "enabled" && rule.enabled && !rule.lastError)
      || (filter === "paused" && !rule.enabled)
      || (filter === "attention" && Boolean(rule.lastError));
    if (!matchesStatus) return false;
    if (!normalizedQuery) return true;
    const searchable = [rule.name, rule.actionPrompt, rule.workspaceName, scheduleLabel(rule)].join("\n").toLocaleLowerCase("zh-CN");
    return searchable.includes(normalizedQuery);
  });
}

export function nextRunDistance(nextRunAt: number | null, now: number = Date.now()): string {
  if (!nextRunAt) return "未排期";
  const delta = nextRunAt - now;
  if (delta <= 0) return "等待调度";
  const minutes = Math.ceil(delta / 60_000);
  if (minutes < 60) return `${minutes} 分钟后`;
  const hours = Math.ceil(delta / 3_600_000);
  if (hours < 24) return `${hours} 小时后`;
  const days = Math.ceil(delta / 86_400_000);
  return `${days} 天后`;
}

function nextRunSummary(rule: AiAutomationRule): string {
  if (!rule.enabled) return "已暂停，不会自动运行";
  if (rule.lastError) return "上次运行需要处理";
  return `下次运行 ${nextRunDistance(rule.nextRunAt)}`;
}

function automationStatus(rule: AiAutomationRule): "enabled" | "paused" | "attention" {
  if (rule.lastError) return "attention";
  return rule.enabled ? "enabled" : "paused";
}

function automationStatusLabel(rule: AiAutomationRule): string {
  const status = automationStatus(rule);
  if (status === "attention") return "需处理";
  return status === "enabled" ? "已启用" : "已暂停";
}

function runStatusLabel(status: string): string {
  if (status === "completed") return "运行已完成";
  if (status === "failed") return "运行失败";
  if (status === "interrupted") return "运行已中断";
  if (status === "terminated") return "运行已终止";
  if (status === "queued") return "等待运行";
  return "正在运行";
}

function toDraft(rule: AiAutomationRule): AutomationDraft {
  return {
    automationRuleId: rule.automationRuleId,
    name: rule.name,
    actionPrompt: rule.actionPrompt,
    scheduleKind: rule.scheduleKind,
    intervalMinutes: rule.intervalMinutes,
    timeOfDayMinutes: rule.timeOfDayMinutes,
    weekday: rule.weekday,
    weekdays: rule.weekdays?.length ? rule.weekdays : [rule.weekday],
    notifyOnEveryRun: rule.notifyOnEveryRun,
    agentId: rule.agentId,
    modelId: rule.modelId ?? null,
    projectId: rule.projectId,
    executionMode: rule.conversationId ? "existing" : "new",
    conversationId: rule.conversationId ?? null,
    enabled: rule.enabled,
  };
}

const weekdayNames = ["", "周一", "周二", "周三", "周四", "周五", "周六", "周日"];

export function scheduleLabel(rule: { scheduleKind: string; intervalMinutes: number; timeOfDayMinutes: number; weekday: number; weekdays?: number[] }): string {
  if (rule.scheduleKind === "daily") return `每天 ${minutesToTime(rule.timeOfDayMinutes)}`;
  if (rule.scheduleKind === "weekly") {
    const weekdays = rule.weekdays?.length ? rule.weekdays : [rule.weekday];
    return `每${weekdays.map((weekday) => weekdayNames[weekday] ?? "周一").join("、")} ${minutesToTime(rule.timeOfDayMinutes)}`;
  }
  const minutes = rule.intervalMinutes;
  return minutes < 60 ? `每 ${minutes} 分钟` : `每 ${minutes / 60} 小时`;
}

function scheduleSummary(draft: AutomationDraft): string {
  return `${scheduleLabel(draft)}，在当前工作区内执行`;
}

function DeleteIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13M10 10v7M14 10v7"/></svg>; }
function PlusIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>; }
function EditIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m14.5 5.5 4 4M5 19l3.8-.8L19 8a2.1 2.1 0 0 0-3-3L5.8 15.2 5 19Z"/></svg>; }
function PlayIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m9 7 8 5-8 5V7Z"/></svg>; }
function ChevronIcon() { return <svg aria-hidden="true" className="automation-chevron" viewBox="0 0 24 24"><path d="m9 7 5 5-5 5"/></svg>; }
function RunsIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M5 12h3l2-5 4 10 2-5h3"/><path d="M4 4h16v16H4z"/></svg>; }
function ArrowIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M4 10h11M11 6l4 4-4 4"/></svg>; }
function SearchEmptyIcon() { return <svg aria-hidden="true" viewBox="0 0 32 32"><circle cx="14" cy="14" r="8"/><path d="m20 20 7 7M10 14h8"/></svg>; }
function StatusIcon({ status }: { status: "enabled" | "paused" | "attention" }) {
  if (status === "attention") return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M12 4 3.8 19h16.4L12 4Z"/><path d="M12 9v4M12 16h.01"/></svg>;
  if (status === "paused") return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8"/><path d="M10 9v6M14 9v6"/></svg>;
  return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8"/><path d="m10 8.5 5 3.5-5 3.5v-7Z"/></svg>;
}
function SuggestionIcon({ kind }: { kind: AutomationDraft["scheduleKind"] }) {
  if (kind === "weekly") return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M6 4v3M18 4v3M4 9h16M5 6h14v14H5z"/><path d="M8 13h3M8 16h6"/></svg>;
  if (kind === "interval") return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M12 5a7 7 0 1 1-6.2 3.8"/><path d="M5 4v5h5M12 8v4l3 2"/></svg>;
  return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M7 18h10M8 18v-6a4 4 0 0 1 8 0v6M6 18h12M10 21h4"/></svg>;
}
