import { useState } from "react";
import type {
BusinessTaskItem,
DeliverableItem,
ProjectPhaseItem
} from "./generated/protocol";
import { AiIcon,PlusIcon } from './project-progress-icons';
import { nextTaskStatus,PhaseDraft,phaseStatusLabel,taskStatusLabel } from './project-progress-model';
import { Select } from "./select";


export function PlanComposer(props: {
  busy: boolean;
  deliverables: DeliverableItem[];
  drafts: PhaseDraft[];
  summary: string;
  onSummary: (value: string) => void;
  onDraft: (index: number, patch: Partial<PhaseDraft>) => void;
  onAdd: () => void;
  onRemove: (index: number) => void;
  onCancel: () => void;
  onSubmit: () => void;
}) {
  const valid =
    props.summary.trim() &&
    props.drafts.length &&
    props.drafts.every(
      (phase) =>
        phase.name.trim() &&
        phase.goal.trim() &&
        phase.acceptanceCriteria.trim(),
    );
  return (
    <section className="plan-composer">
      <header>
        <div>
          <h2>创建阶段计划草案</h2>
          <p>通过前置关系表达串行或并行路线；无前置阶段的节点可以同时启动。</p>
        </div>
        <button onClick={props.onCancel} type="button">
          取消
        </button>
      </header>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          props.onSubmit();
        }}
      >
        <label>
          计划说明
          <input
            autoFocus
            value={props.summary}
            onChange={(event) => props.onSummary(event.target.value)}
            placeholder="例如：完成桌面端第一轮可验收交付"
          />
        </label>
        <div className="phase-draft-list">
          {props.drafts.map((phase, index) => (
            <article key={index}>
              <header>
                <span>{index + 1}</span>
                <strong>阶段 {index + 1}</strong>
                {props.drafts.length > 1 ? (
                  <button
                    aria-label={`删除阶段 ${index + 1}`}
                    onClick={() => props.onRemove(index)}
                    type="button"
                  >
                    删除
                  </button>
                ) : null}
              </header>
              <label>
                阶段名称
                <input
                  value={phase.name}
                  onChange={(event) =>
                    props.onDraft(index, { name: event.target.value })
                  }
                  placeholder="例如：需求定义"
                />
              </label>
              <label>
                阶段目标
                <textarea
                  value={phase.goal}
                  onChange={(event) =>
                    props.onDraft(index, { goal: event.target.value })
                  }
                  rows={2}
                  placeholder="这一阶段要形成什么变化？"
                />
              </label>
              <label>
                完成标准
                <textarea
                  value={phase.acceptanceCriteria}
                  onChange={(event) =>
                    props.onDraft(index, {
                      acceptanceCriteria: event.target.value,
                    })
                  }
                  rows={2}
                  placeholder="满足什么条件才能完成阶段？"
                />
              </label>
              <label>
                关联交付物（可选）
                <Select
                  placeholder="暂不关联"
                  options={props.deliverables.map((deliverable) => ({
                    value: deliverable.deliverableId,
                    label: deliverable.title,
                  }))}
                  value={phase.requiredDeliverableId}
                  onChange={(value) =>
                    props.onDraft(index, { requiredDeliverableId: value })
                  }
                />
              </label>
              <details className="phase-dependencies">
                <summary>前置阶段</summary>
                {index === 0 ? (
                  <p>首个阶段没有前置条件。</p>
                ) : (
                  props.drafts.slice(0, index).map((candidate, predecessor) => (
                    <label key={predecessor}>
                      <input
                        checked={phase.predecessorIndexes.includes(predecessor)}
                        onChange={(event) =>
                          props.onDraft(index, {
                            predecessorIndexes: event.target.checked
                              ? [...phase.predecessorIndexes, predecessor].sort(
                                  (left, right) => left - right,
                                )
                              : phase.predecessorIndexes.filter(
                                  (value) => value !== predecessor,
                                ),
                          })
                        }
                        type="checkbox"
                      />
                      <span>
                        {candidate.name.trim() || `阶段 ${predecessor + 1}`}
                      </span>
                    </label>
                  ))
                )}
              </details>
            </article>
          ))}
        </div>
        <div className="plan-composer-actions">
          <button onClick={props.onAdd} type="button">
            <PlusIcon />
            添加阶段
          </button>
          <button
            className="primary"
            disabled={props.busy || !valid}
            type="submit"
          >
            {props.busy ? "正在保存…" : "保存草案"}
          </button>
        </div>
      </form>
    </section>
  );
}

export function TaskCard(props: {
  task: BusinessTaskItem;
  availablePhases: ProjectPhaseItem[];
  busy: boolean;
  operatorReady: boolean;
  onTransition: (task: BusinessTaskItem, target: string) => void;
  onMove: (
    task: BusinessTaskItem,
    input: {
      targetPhaseId: string;
      dependencyImpact: string;
      deliverableImpact: string;
      acceptanceImpact: string;
    },
  ) => Promise<void>;
  onOpenAi: () => void;
}) {
  const target = nextTaskStatus[props.task.status];
  const [showMove, setShowMove] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [targetPhaseId, setTargetPhaseId] = useState("");
  const [dependencyImpact, setDependencyImpact] = useState("");
  const [deliverableImpact, setDeliverableImpact] = useState("");
  const [acceptanceImpact, setAcceptanceImpact] = useState("");
  const movable =
    ["candidate", "todo", "rework"].includes(props.task.status) &&
    props.availablePhases.length > 0;
  const moveReady = Boolean(
    props.operatorReady &&
    targetPhaseId &&
    dependencyImpact.trim() &&
    deliverableImpact.trim() &&
    acceptanceImpact.trim(),
  );
  const submitMove = async () => {
    if (!moveReady) return;
    await props.onMove(props.task, {
      targetPhaseId,
      dependencyImpact,
      deliverableImpact,
      acceptanceImpact,
    });
    setShowMove(false);
    setTargetPhaseId("");
    setDependencyImpact("");
    setDeliverableImpact("");
    setAcceptanceImpact("");
  };
  return (
    <article
      className="business-task-card"
      data-expanded={expanded}
      data-status={props.task.status}
    >
      <button
        aria-expanded={expanded}
        className="business-task-toggle"
        onClick={() => setExpanded((value) => !value)}
        title={expanded ? "收起任务合同" : "展开任务合同"}
        type="button"
      >
        {expanded ? "收起" : "详情"}
      </button>
      <header>
        <div>
          <span>{taskStatusLabel[props.task.status] ?? props.task.status}</span>
          <i>
            {props.task.priority === "critical"
              ? "紧急"
              : props.task.priority === "high"
                ? "高优先"
                : props.task.priority === "low"
                  ? "低优先"
                  : "中优先"}
          </i>
        </div>
        <strong>{props.task.title}</strong>
        <p>{props.task.objective}</p>
      </header>
      {expanded ? (
        <dl>
          <div>
            <dt>预期输出</dt>
            <dd>{props.task.expectedOutput}</dd>
          </div>
          <div>
            <dt>完成标准</dt>
            <dd>{props.task.completionCriteria}</dd>
          </div>
          <div>
            <dt>执行尝试</dt>
            <dd>第 {props.task.attemptNo} 次</dd>
          </div>
        </dl>
      ) : null}
      {showMove ? (
        <section className="task-move-form">
          <header>
            <strong>移动到其他阶段</strong>
            <button onClick={() => setShowMove(false)} type="button">
              取消
            </button>
          </header>
          <label>
            目标阶段
            <Select
              autoFocus
              options={[
                { value: "", label: "请选择" },
                ...props.availablePhases.map((phase) => ({
                  value: phase.phaseId,
                  label: `${phase.name} · ${phaseStatusLabel[phase.status] ?? phase.status}`,
                })),
              ]}
              value={targetPhaseId}
              onChange={setTargetPhaseId}
            />
          </label>
          <label>
            任务依赖影响
            <textarea
              onChange={(event) => setDependencyImpact(event.target.value)}
              placeholder="前置输入、后续任务或阻塞会如何变化？"
              rows={2}
              value={dependencyImpact}
            />
          </label>
          <label>
            交付物影响
            <textarea
              onChange={(event) => setDeliverableImpact(event.target.value)}
              placeholder="任务输出将归属或影响哪个交付物？"
              rows={2}
              value={deliverableImpact}
            />
          </label>
          <label>
            阶段验收影响
            <textarea
              onChange={(event) => setAcceptanceImpact(event.target.value)}
              placeholder="原阶段与目标阶段的验收口径如何调整？"
              rows={2}
              value={acceptanceImpact}
            />
          </label>
          <button
            className="primary"
            disabled={props.busy || !moveReady}
            onClick={() => void submitMove()}
            type="button"
          >
            确认移动并记录影响
          </button>
        </section>
      ) : null}
      <footer>
        <span>
          {props.task.assigneeLabel} ·{" "}
          {props.task.assigneeType === "agent"
            ? "智能体"
            : props.task.assigneeType === "collaboration"
              ? "协作"
              : "个人"}
        </span>
        <div>
          <button onClick={props.onOpenAi} type="button">
            <AiIcon />
            AI 执行
          </button>
          {movable ? (
            <button
              onClick={() => setShowMove((value) => !value)}
              type="button"
            >
              移动阶段
            </button>
          ) : null}
          {props.task.status === "review" ? (
            <button
              disabled={props.busy || !props.operatorReady}
              onClick={() => props.onTransition(props.task, "rework")}
              type="button"
            >
              请求返工
            </button>
          ) : null}
          {target ? (
            <button
              disabled={
                props.busy || !props.operatorReady || !props.operatorReady
              }
              onClick={() => props.onTransition(props.task, target)}
              type="button"
            >
              {target === "todo"
                ? "纳入待办"
                : target === "in_progress"
                  ? "开始执行"
                  : target === "review"
                    ? "提交验收"
                    : "确认完成"}
            </button>
          ) : null}
        </div>
      </footer>
    </article>
  );
}
