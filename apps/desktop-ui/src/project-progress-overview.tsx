import { useState } from "react";
import { conversationFileArtifacts } from "./ai-artifacts";
import {
type AiConversation
} from "./ai-runtime";
import type {
AttentionCaseItem,
BusinessTaskItem,
DeliverableItem,
ProjectPhaseItem
} from "./generated/protocol";
import { projectTaskGroups,taskPriorityLabel,taskStatusLabel } from './project-progress-model';
import { SearchField } from "./search-field";
import { Select } from "./select";
import { TaskArtifactList,type TaskArtifactPreview } from "./task-artifacts";


export function ProjectOverview(props: {
  artifacts: ReturnType<typeof conversationFileArtifacts>;
  onPreview: (preview: TaskArtifactPreview) => void;
  attentionCases: AttentionCaseItem[];
  conversations: AiConversation[];
  deliverables: DeliverableItem[];
  hasPlan: boolean;
  tasks: BusinessTaskItem[];
  onOpenConversation: (conversationId: string) => void;
  onOpenTasks: (status?: string) => void;
  onOpenTask: (task: BusinessTaskItem) => void;
  onSetPhases: () => void;
}) {
  const openCases = props.attentionCases
    .filter((item) => item.status !== "resolved" && item.caseType !== "agent_run_incident")
    .sort((left, right) => right.updatedAt - left.updatedAt);
  const recentTasks = [...props.tasks]
    .sort((left, right) => right.updatedAt - left.updatedAt)
    .slice(0, 5);
  return (
    <main className="project-overview">
      <section className="project-status-overview">
        <header>
          <div>
            <h2>任务状态</h2>
          </div>
          <button onClick={() => props.onOpenTasks()} type="button">
            查看全部
          </button>
        </header>
        <div className="project-status-grid">
          {projectTaskGroups.map((group) => {
            const count = props.tasks.filter((task) =>
              group.statuses.includes(task.status as never),
            ).length;
            return (
              <button
                data-status={group.key}
                key={group.key}
                onClick={() => props.onOpenTasks(group.key)}
                type="button"
              >
                <span>{group.label}</span>
                <strong>{count}</strong>
              </button>
            );
          })}
        </div>
      </section>

      {openCases.length ? (
        <section className="project-overview-attention">
          <header>
            <h2>需介入事项</h2>
            <span>{openCases.length}</span>
          </header>
          <div>
            {openCases.slice(0, 5).map((item) => (
              <article key={item.attentionCaseId}>
                <span>
                  {item.caseType === "decision_request"
                    ? "决策"
                    : item.caseType === "project_blocker"
                      ? "阻塞"
                      : "异常"}
                </span>
                <strong>{item.title}</strong>
                <small>{item.responsibilityLabel}</small>
              </article>
            ))}
          </div>
        </section>
      ) : null}

      <section className="project-overview-streams">
        <article>
          <header>
            <h2>最近任务</h2>
            <span>{props.tasks.length}</span>
          </header>
          {recentTasks.length ? (
            <div>
              {recentTasks.map((task) => (
                <button
                  key={task.businessTaskId}
                  onClick={() => props.onOpenTask(task)}
                  type="button"
                >
                  <span>{taskStatusLabel[task.status] ?? task.status}</span>
                  <strong>{task.title}</strong>
                  <small>{formatBriefTime(task.updatedAt)}</small>
                </button>
              ))}
            </div>
          ) : (
            <p>暂无任务</p>
          )}
        </article>
        <article>
          <header>
            <h2>最近对话</h2>
            <span>{props.conversations.length}</span>
          </header>
          {props.conversations.length ? (
            <div>
              {props.conversations.slice(0, 5).map((conversation) => (
                <button
                  key={conversation.conversationId}
                  onClick={() =>
                    props.onOpenConversation(conversation.conversationId)
                  }
                  type="button"
                >
                  <strong>{conversation.title}</strong>
                  <small>{formatBriefTime(conversation.updatedAt)}</small>
                </button>
              ))}
            </div>
          ) : (
            <p>暂无项目对话</p>
          )}
        </article>
        <article>
          <header>
            <h2>最近产物</h2>
            <span>{props.artifacts.length}</span>
          </header>
          {props.artifacts.length ? (
            <TaskArtifactList entries={props.artifacts} onPreview={props.onPreview}/>

          ) : (
            <p>暂无登记产物</p>
          )}
        </article>
      </section>

      {props.hasPlan || props.deliverables.length ? (
        <section className="project-overview-structure">
          <h2>管理结构</h2>
          <div>
            {props.hasPlan ? <span>已设置阶段</span> : null}
            {props.deliverables.length ? (
              <span>正式交付物 {props.deliverables.length}</span>
            ) : null}
          </div>
        </section>
      ) : (
        <button
          className="project-overview-phase-action"
          onClick={props.onSetPhases}
          type="button"
        >
          需要分段管理时设置阶段
        </button>
      )}
    </main>
  );
}

export function ProjectTaskLedger(props: {
  phases: ProjectPhaseItem[];
  tasks: BusinessTaskItem[];
  status: string;
  onStatus: (status: string) => void;
  onOpenTask: (task: BusinessTaskItem) => void;
}) {
  const [query, setQuery] = useState("");
  const [owner, setOwner] = useState("all");
  const [phase, setPhase] = useState("all");
  const [priority, setPriority] = useState("all");
  const [board, setBoard] = useState(false);
  const phaseNames = new Map(props.phases.map((phase) => [phase.phaseId, phase.name]));
  const owners = [...new Set(props.tasks.map((task) => task.assigneeLabel))];
  const normalized = query.trim().toLocaleLowerCase();
  const filtered = props.tasks.filter((task) =>
    (!normalized || [task.title, task.objective, task.assigneeLabel].join(" ").toLocaleLowerCase().includes(normalized)) &&
    (owner === "all" || task.assigneeLabel === owner) &&
    (phase === "all" || (task.phaseId ?? "none") === phase) &&
    (priority === "all" || task.priority === priority));
  return <section className="project-task-ledger">
    <div className="project-task-filters">
      <SearchField ariaLabel="搜索项目任务" placeholder="搜索任务" value={query} onChange={setQuery}/>
      <Select ariaLabel="任务负责人" value={owner} onChange={setOwner} options={[{ value: "all", label: "全部负责人" }, ...owners.map((value) => ({ value, label: value }))]}/>
      <Select ariaLabel="任务阶段" value={phase} onChange={setPhase} options={[{ value: "all", label: "全部阶段" }, { value: "none", label: "未绑定阶段" }, ...props.phases.map((item) => ({ value: item.phaseId, label: item.name }))]}/>
      <Select ariaLabel="任务优先级" value={priority} onChange={setPriority} options={[{ value: "all", label: "全部优先级" }, ...Object.entries(taskPriorityLabel).map(([value, label]) => ({ value, label }))]}/>
      <button aria-pressed={!board} onClick={() => setBoard(false)} type="button">列表</button><button aria-pressed={board} onClick={() => setBoard(true)} type="button">看板</button>
    </div>
    <nav className="project-task-status-filter" aria-label="任务状态筛选">
      <button aria-pressed={props.status === "all"} onClick={() => props.onStatus("all")} type="button">全部 {filtered.length}</button>
      {projectTaskGroups.map((group) => <button aria-pressed={props.status === group.key} key={group.key} onClick={() => props.onStatus(group.key)} type="button">{group.label} {filtered.filter((task) => group.statuses.includes(task.status as never)).length}</button>)}
    </nav>
    {!filtered.some((task) => projectTaskGroups.some((group) => (props.status === "all" || props.status === group.key) && group.statuses.includes(task.status as never))) ? <p className="project-task-empty" role="status">没有匹配的任务</p> : null}
    <div className="project-task-columns" data-view={board ? "board" : "list"}>
      {projectTaskGroups.filter((group) => props.status === "all" || props.status === group.key).map((group) => {
        const tasks = filtered.filter((task) => group.statuses.includes(task.status as never));
        return <section data-status={group.key} data-empty={!tasks.length} key={group.key}>
          <header><h3>{group.label}</h3><span>{tasks.length}</span></header>
          <div>{tasks.map((task) => <article key={task.businessTaskId}>
            <button className="project-task-open" onClick={() => props.onOpenTask(task)} type="button">{task.title}</button>
            <span>{taskStatusLabel[task.status]}</span>
            <small>{task.phaseId ? phaseNames.get(task.phaseId) ?? "阶段不可用" : "未绑定阶段"}</small>
            <span>{task.assigneeLabel}</span><small>{taskPriorityLabel[task.priority] ?? task.priority}</small>
            <time>{formatBriefTime(task.updatedAt)}</time>
          </article>)}{!tasks.length ? <p>暂无任务</p> : null}</div>
        </section>;
      })}
    </div>
  </section>;
}

export function formatBriefTime(value: number) {
  if (!value) return "";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}
