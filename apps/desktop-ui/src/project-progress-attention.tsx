import { useState } from "react";
import type {
AttentionCaseItem
} from "./generated/protocol";
import { AgentIcon,AttentionIcon,BlockerIcon,DecisionIcon } from './project-progress-icons';
import { Select } from "./select";


export function AttentionPanel(props: {
  attentionCases: AttentionCaseItem[];
  busy: boolean;
  detail: string;
  decisionOptions: string;
  decisionImpact: string;
  decisionEvidence: string;
  decisionRecommendation: string;
  decisionConsequence: string;
  mode: "blocker" | "decision" | null;
  operatorReady: boolean;
  owner: string;
  phaseName: (phaseId: string | null) => string;
  priority: string;
  title: string;
  onCancel: () => void;
  onCreate: () => void;
  onDetail: (value: string) => void;
  onDecisionOptions: (value: string) => void;
  onDecisionImpact: (value: string) => void;
  onDecisionEvidence: (value: string) => void;
  onDecisionRecommendation: (value: string) => void;
  onDecisionConsequence: (value: string) => void;
  onMode: (mode: "blocker" | "decision") => void;
  onOwner: (value: string) => void;
  onPriority: (value: string) => void;
  onTitle: (value: string) => void;
  onTransition: (
    item: AttentionCaseItem,
    status: "open" | "acknowledged" | "resolved",
    decisionAction?: string,
    delegatedTo?: string,
  ) => void;
}) {
  const openCases = props.attentionCases.filter(
    (item) => item.status !== "resolved" && item.caseType !== "agent_run_incident",
  );
  const resolvedCases = props.attentionCases.filter(
    (item) => item.status === "resolved",
  );
  return (
    <section className="attention-panel">
      <header>
        <div>
          <h3>项目关注事项</h3>
          <p>阻塞、待决策和智能体异常都是项目事实，并会投影到个人工作台。</p>
        </div>
        <div>
          <button onClick={() => props.onMode("blocker")} type="button">
            <BlockerIcon />
            记录阻塞
          </button>
          <button onClick={() => props.onMode("decision")} type="button">
            <DecisionIcon />
            发起决策
          </button>
        </div>
      </header>
      {props.mode ? (
        <form
          className="attention-composer"
          onSubmit={(event) => {
            event.preventDefault();
            props.onCreate();
          }}
        >
          <div>
            <strong>
              {props.mode === "blocker" ? "记录项目阻塞" : "发起结构化决策请求"}
            </strong>
            <button onClick={props.onCancel} type="button">
              取消
            </button>
          </div>
          <label>
            标题
            <input
              autoFocus
              onChange={(event) => props.onTitle(event.target.value)}
              placeholder={
                props.mode === "blocker"
                  ? "什么正在阻碍项目推进？"
                  : "需要做出什么决策？"
              }
              value={props.title}
            />
          </label>
          <label>
            {props.mode === "blocker" ? "阻塞原因" : "决策问题"}
            <textarea
              onChange={(event) => props.onDetail(event.target.value)}
              rows={2}
              value={props.detail}
            />
          </label>
          {props.mode === "decision" ? (
            <div className="decision-card-fields">
              <label>
                可选方案
                <textarea
                  onChange={(event) =>
                    props.onDecisionOptions(event.target.value)
                  }
                  rows={3}
                  placeholder="列出至少两个可执行方案"
                  value={props.decisionOptions}
                />
              </label>
              <label>
                影响范围
                <textarea
                  onChange={(event) =>
                    props.onDecisionImpact(event.target.value)
                  }
                  rows={3}
                  placeholder="对阶段、任务、交付物和时间的影响"
                  value={props.decisionImpact}
                />
              </label>
              <label>
                证据
                <textarea
                  onChange={(event) =>
                    props.onDecisionEvidence(event.target.value)
                  }
                  rows={3}
                  placeholder="支持判断的成果版本、数据或风险事实"
                  value={props.decisionEvidence}
                />
              </label>
              <label>
                建议与理由
                <textarea
                  onChange={(event) =>
                    props.onDecisionRecommendation(event.target.value)
                  }
                  rows={3}
                  value={props.decisionRecommendation}
                />
              </label>
              <label>
                不决定的后果
                <textarea
                  onChange={(event) =>
                    props.onDecisionConsequence(event.target.value)
                  }
                  rows={3}
                  value={props.decisionConsequence}
                />
              </label>
            </div>
          ) : null}
          <div>
            <label>
              责任人
              <input
                onChange={(event) => props.onOwner(event.target.value)}
                placeholder="明确到个人"
                value={props.owner}
              />
            </label>
            <label>
              优先级
              <Select
                options={[
                  { value: "medium", label: "中" },
                  { value: "high", label: "高" },
                  { value: "critical", label: "紧急" },
                  { value: "low", label: "低" },
                ]}
                value={props.priority}
                onChange={props.onPriority}
              />
            </label>
          </div>
          <button
            className="primary"
            disabled={
              props.busy ||
              !props.operatorReady ||
              !props.title.trim() ||
              !props.detail.trim() ||
              !props.owner.trim() ||
              (props.mode === "decision" &&
                [
                  props.decisionOptions,
                  props.decisionImpact,
                  props.decisionEvidence,
                  props.decisionRecommendation,
                  props.decisionConsequence,
                ].some((value) => !value.trim()))
            }
            type="submit"
          >
            {props.mode === "blocker" ? "确认记录" : "确认发起"}
          </button>
        </form>
      ) : null}
      <div className="attention-list">
        {openCases.map((item) => (
          <AttentionCard
            item={item}
            key={item.attentionCaseId}
            busy={props.busy}
            operatorReady={props.operatorReady}
            phaseName={props.phaseName(item.phaseId)}
            onTransition={props.onTransition}
          />
        ))}
        {openCases.length === 0 ? (
          <div className="attention-empty">
            <AttentionIcon />
            <span>当前项目没有未解决的关注事项</span>
          </div>
        ) : null}
      </div>
      {resolvedCases.length ? (
        <details className="attention-history">
          <summary>已解决 {resolvedCases.length}</summary>
          {resolvedCases.map((item) => (
            <AttentionCard
              item={item}
              key={item.attentionCaseId}
              busy={props.busy}
              operatorReady={props.operatorReady}
              phaseName={props.phaseName(item.phaseId)}
              onTransition={props.onTransition}
            />
          ))}
        </details>
      ) : null}
    </section>
  );
}

export function AttentionCard(props: {
  item: AttentionCaseItem;
  busy: boolean;
  operatorReady: boolean;
  phaseName: string;
  onTransition: (
    item: AttentionCaseItem,
    status: "open" | "acknowledged" | "resolved",
    decisionAction?: string,
    delegatedTo?: string,
  ) => void;
}) {
  const typeLabel =
    props.item.caseType === "project_blocker"
      ? "项目阻塞"
      : props.item.caseType === "decision_request"
        ? "待决策"
        : "智能体异常";
  const [decisionAction, setDecisionAction] = useState("approved");
  const [delegatedTo, setDelegatedTo] = useState("");
  const decisionReady =
    props.item.caseType !== "decision_request" ||
    Boolean(
      decisionAction && (decisionAction !== "delegated" || delegatedTo.trim()),
    );
  const submit = () =>
    props.onTransition(
      props.item,
      decisionAction === "delegated" ? "open" : "resolved",
      props.item.caseType === "decision_request" ? decisionAction : undefined,
      decisionAction === "delegated" ? delegatedTo.trim() : undefined,
    );
  return (
    <article
      className="attention-card"
      data-status={props.item.status}
      data-type={props.item.caseType}
    >
      <header>
        <span>
          {props.item.caseType === "project_blocker" ? (
            <BlockerIcon />
          ) : props.item.caseType === "decision_request" ? (
            <DecisionIcon />
          ) : (
            <AgentIcon />
          )}
          {typeLabel}
        </span>
        <small>
          {props.phaseName} ·{" "}
          {props.item.status === "acknowledged"
            ? "处置中"
            : props.item.status === "resolved"
              ? "已解决"
              : "待处理"}
        </small>
      </header>
      <strong>{props.item.title}</strong>
      <p>{props.item.detail}</p>
      {props.item.caseType === "decision_request" ? (
        <>
          <dl className="decision-card-detail">
            <div>
              <dt>可选方案</dt>
              <dd>{props.item.decisionOptions}</dd>
            </div>
            <div>
              <dt>影响</dt>
              <dd>{props.item.decisionImpact}</dd>
            </div>
            <div>
              <dt>证据</dt>
              <dd>{props.item.decisionEvidence}</dd>
            </div>
            <div>
              <dt>建议</dt>
              <dd>{props.item.decisionRecommendation}</dd>
            </div>
            <div>
              <dt>不决定的后果</dt>
              <dd>{props.item.decisionConsequence}</dd>
            </div>
          </dl>
          {props.item.status !== "resolved" ? (
            <div className="attention-decision-action">
              <Select
                ariaLabel="决策动作"
                options={[
                  { value: "approved", label: "通过" },
                  { value: "change_requested", label: "请求修改" },
                  { value: "rejected", label: "拒绝" },
                  { value: "delegated", label: "转交" },
                ]}
                value={decisionAction}
                onChange={setDecisionAction}
              />
              {decisionAction === "delegated" ? (
                <input
                  aria-label="转交给"
                  onChange={(event) => setDelegatedTo(event.target.value)}
                  placeholder="新的决策负责人"
                  value={delegatedTo}
                />
              ) : null}
            </div>
          ) : null}
        </>
      ) : null}
      {props.item.agentLabel ? (
        <p className="attention-agent">
          {props.item.agentLabel} · 运行 {props.item.agentRunId}
        </p>
      ) : null}
      <footer>
        <span>
          {props.item.responsibilityLabel} ·{" "}
          {props.item.priority === "critical"
            ? "紧急"
            : props.item.priority === "high"
              ? "高优先"
              : props.item.priority === "low"
                ? "低优先"
                : "中优先"}
        </span>
        <div>
          {props.item.caseType === "agent_run_incident" &&
          props.item.status === "open" ? (
            <button
              disabled={props.busy || !props.operatorReady}
              onClick={() => props.onTransition(props.item, "acknowledged")}
              type="button"
            >
              已知晓
            </button>
          ) : null}
          {props.item.status !== "resolved" ? (
            <button
              className="primary"
              disabled={props.busy || !props.operatorReady || !decisionReady}
              onClick={submit}
              type="button"
            >
              {props.item.caseType === "decision_request"
                ? decisionAction === "delegated"
                  ? "确认转交"
                  : "提交决策"
                : "标记已解决"}
            </button>
          ) : null}
        </div>
      </footer>
      {props.item.decisionActionSummary ? (
        <blockquote>
          {props.item.decisionAction === "delegated"
            ? `已转交给 ${props.item.decisionDelegatedTo}：`
            : "决策结论："}
          {props.item.decisionActionSummary}
        </blockquote>
      ) : props.item.resolutionSummary ? (
        <blockquote>{props.item.resolutionSummary}</blockquote>
      ) : null}
    </article>
  );
}
