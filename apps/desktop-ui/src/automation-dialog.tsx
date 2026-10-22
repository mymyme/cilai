import { automationNextPreview } from "./automation-preview";
import { type ReactNode, useEffect, useState } from "react";
import type { AiProjectOption } from "./ai-workbench";
import { useModal } from "./use-modal";
import type { AiRuntimeSnapshot, AutomationScheduleKind } from "./ai-runtime";
import { Select } from "./select";
import "./automation-dialog.css";

export interface AutomationDraft {
  automationRuleId: string | null;
  name: string;
  actionPrompt: string;
  scheduleKind: AutomationScheduleKind;
  intervalMinutes: number;
  timeOfDayMinutes: number;
  weekday: number;
  weekdays: number[];
  notifyOnEveryRun: boolean;
  agentId: string;
  modelId: string | null;
  projectId: string | null;
  executionMode: "new" | "existing";
  conversationId: string | null;
  enabled: boolean;
}

const intervalOptions = [
  { value: "15", label: "每 15 分钟" },
  { value: "60", label: "每小时" },
  { value: "240", label: "每 4 小时" },
];

const weekdayOptions = [
  { value: "1", label: "星期一" },
  { value: "2", label: "星期二" },
  { value: "3", label: "星期三" },
  { value: "4", label: "星期四" },
  { value: "5", label: "星期五" },
  { value: "6", label: "星期六" },
  { value: "7", label: "星期日" },
];

const scheduleOptions = [
  { value: "interval", label: "按间隔" },
  { value: "daily", label: "每天" },
  { value: "weekly", label: "每周" },
];

/**
 * The rule is a written instruction the model carries out, so the instruction is the dialog's
 * subject and everything else is settings underneath it. Schedules are expressed the way a person
 * states them — every day at a time, or a weekday at a time — rather than as an elapsed interval.
 */
export function AutomationDialog(props: {
  draft: AutomationDraft;
  agents: AiRuntimeSnapshot["agents"];
  models: AiRuntimeSnapshot["models"];
  projects: AiProjectOption[];
  conversations: AiRuntimeSnapshot["conversations"];
  busy: boolean;
  onDraft: (draft: AutomationDraft) => void;
  onCancel: () => void;
  onSave: () => void;
  onOpenModels?: () => void;
}) {
  const { draft, busy, onDraft, onCancel } = props;

  const modalRef = useModal<HTMLDivElement>({ onClose: onCancel, busy });

  const project = props.projects.find((item) => item.projectId === draft.projectId);
  const conversations = props.conversations.filter((item) => !item.archived && item.context.projectId === draft.projectId);
  const titleCounts = new Map<string, number>();
  for (const conversation of conversations) titleCounts.set(conversation.title, (titleCounts.get(conversation.title) ?? 0) + 1);
  const executionUnavailable = !draft.modelId;
  const [now, setNow] = useState(() => new Date());
  useEffect(() => { const timer = window.setInterval(() => setNow(new Date()), 30_000); return () => window.clearInterval(timer); }, []);
  const next = automationNextPreview(draft, now);
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const incomplete = executionUnavailable || !draft.name.trim() || !draft.actionPrompt.trim() || !props.agents.some((agent) => agent.agentId === draft.agentId) || !project?.primaryWorkspaceRoot || (draft.executionMode === "existing" && !conversations.some((item) => item.conversationId === draft.conversationId)) || (draft.modelId !== null && !props.models.some((model) => model.modelId === draft.modelId)) || (draft.scheduleKind === "weekly" && draft.weekdays.length === 0);

  return <div
    aria-label={draft.automationRuleId ? "编辑自动化" : "新建自动化"}
    aria-modal="true"
    className="automation-dialog-backdrop"
    ref={modalRef}
    onMouseDown={(event) => { if (event.target === event.currentTarget && !busy) onCancel(); }}
    role="dialog">
    <section className="automation-dialog">
      <header>
        <div className="automation-heading"><span>{draft.automationRuleId ? "编辑自动化" : "新建自动化"}</span><h3>{draft.name.trim() || "未命名自动化"}</h3></div>
        <div className="automation-header-actions">
          <label className="automation-enabled-control"><span>启用</span><button
            aria-label="是否启用"
            aria-checked={draft.enabled}
            className="automation-enabled-switch"
            disabled={busy}
            role="switch"
            type="button"
            onClick={() => onDraft({ ...draft, enabled: !draft.enabled })}><span aria-hidden="true"/></button></label>
          <button aria-label="关闭" className="automation-close" disabled={busy} onClick={onCancel} type="button"><CloseIcon/></button>
        </div>
      </header>

      <div className="automation-body">
        <label className="automation-field automation-name-field">
          <span>自动化名称</span>
          <input
            aria-label="自动化名称"
            autoFocus
            disabled={busy}
            placeholder="一句话说清这条自动化是干什么的"
            value={draft.name}
            onChange={(event) => onDraft({ ...draft, name: event.target.value })}/>
        </label>

        <label className="automation-field">
          <span>指令</span>
          <textarea
            aria-label="交给智能体执行的指令"
            className="automation-prompt"
            disabled={busy}
            placeholder="用自然语言写清楚要智能体做什么，包括依据哪些文件、产出什么、放到哪里。"
            rows={4}
            value={draft.actionPrompt}
            onChange={(event) => onDraft({ ...draft, actionPrompt: event.target.value })}/>
        </label>

      <Group icon={<InfoIcon/>} title="详情">
        <Row label="项目"><Select ariaLabel="执行项目" disabled={busy} value={draft.projectId ?? ""}
          options={[{ value: "", label: "选择项目", disabled: true }, ...props.projects.map((item) => ({ value: item.projectId, label: item.displayName, disabled: !item.primaryWorkspaceRoot }))]}
          onChange={(projectId) => onDraft({ ...draft, projectId, conversationId: null })}/></Row>
        <Row label="执行位置"><span className="automation-static">{project?.primaryWorkspaceRoot ? `本机 · ${project.primaryWorkspaceName ?? project.displayName}` : "需连接本地文件夹"}</span></Row>
        <Row label="停止条件"><span className="automation-static">停用规则或退出工作台</span></Row>
        <Row label="对话方式"><Select ariaLabel="对话方式" disabled={busy || !project?.primaryWorkspaceRoot} value={draft.executionMode}
          options={[{ value: "new", label: "新对话" }, { value: "existing", label: "已有对话" }]}
          onChange={(value) => onDraft({ ...draft, executionMode: value as AutomationDraft["executionMode"], conversationId: null })}/></Row>
        {draft.executionMode === "existing" ? <Row label="对话"><Select ariaLabel="选择对话" disabled={busy || !project?.primaryWorkspaceRoot || conversations.length === 0} value={draft.conversationId ?? ""} placeholder={conversations.length ? "选择对话" : "暂无可用对话"}
          options={conversations.map((item) => ({ value: item.conversationId, label: titleCounts.get(item.title) === 1 ? item.title : `${item.title} · ${new Date(item.updatedAt).toLocaleString("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" })}` }))}
          onChange={(value) => onDraft({ ...draft, conversationId: value })}/></Row> : null}
        <Row label="模型"><Select
          onOpen={props.onOpenModels}
          ariaLabel="模型"
          placeholder="请选择模型"
          disabled={busy}
          options={props.models.map((model) => ({ value: model.modelId, label: model.label }))}
          value={draft.modelId ?? ""}
          onChange={(value) => onDraft({ ...draft, modelId: value })}/></Row>
        {executionUnavailable ? <p role="status">请选择可用的模型。</p> : null}
      </Group>

      <Group icon={<ScheduleIcon/>} title="频率">
        <Row label="重复"><Select
          ariaLabel="重复方式"
          disabled={busy}
          options={scheduleOptions}
          value={draft.scheduleKind}
          onChange={(value) => onDraft({ ...draft, scheduleKind: value as AutomationScheduleKind })}/></Row>
        {draft.scheduleKind === "interval" ? <Row label="间隔"><Select
          ariaLabel="执行间隔"
          disabled={busy}
          options={intervalOptions}
          value={String(draft.intervalMinutes)}
          onChange={(value) => onDraft({ ...draft, intervalMinutes: Number(value) })}/></Row> : null}
        {draft.scheduleKind === "weekly" ? <>
          <Row label="每隔"><span className="automation-static">1 周</span></Row>
          <Row label="开启"><div aria-label="每周执行日期" className="automation-weekdays" role="group">
            {weekdayOptions.map((option) => {
              const weekday = Number(option.value);
              const selected = draft.weekdays.includes(weekday);
              return <button aria-label={option.label} aria-pressed={selected} className={selected ? "selected" : ""} disabled={busy} key={option.value} onClick={() => {
                const weekdays = selected ? draft.weekdays.filter((item) => item !== weekday) : [...draft.weekdays, weekday].sort();
                onDraft({ ...draft, weekdays, weekday: weekdays[0] ?? draft.weekday });
              }} title={option.label} type="button">{option.label.slice(-1)}</button>;
            })}
          </div></Row>
        </> : null}
        {draft.scheduleKind === "interval" ? null : <Row label="时间"><input
          aria-label="执行时间"
          disabled={busy}
          type="time"
          value={minutesToTime(draft.timeOfDayMinutes)}
          onChange={(event) => onDraft({ ...draft, timeOfDayMinutes: timeToMinutes(event.target.value) })}/></Row>}
        <Row label="时区"><span className="automation-static">{timezone} · 本机时间</span></Row>
        <Row label="下次触发"><span className="automation-static">{!draft.enabled ? "停用，不排期" : next ? `${next.toLocaleString("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit", hour12: false })}（预计）` : "请补全执行日期"}</span></Row>
        <Row label="通知"><Select
          ariaLabel="通知方式"
          disabled={busy}
          options={[{ value: "false", label: "仅需要处理时" }, { value: "true", label: "所有运行" }]}
          value={String(draft.notifyOnEveryRun)}
          onChange={(value) => onDraft({ ...draft, notifyOnEveryRun: value === "true" })}/></Row>
      </Group>
      </div>

      <footer>
        <button disabled={busy} onClick={onCancel} type="button">取消</button>
        <button className="primary" disabled={busy || incomplete} onClick={props.onSave} type="button">{busy ? "保存中…" : "保存"}</button>
      </footer>
    </section>
  </div>;
}

function Group({ title, icon, children }: { title: string; icon: ReactNode; children: ReactNode }) {
  return <section className="automation-group"><h4><span>{title}</span>{icon}</h4><dl>{children}</dl></section>;
}

function CloseIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18"/></svg>; }
function InfoIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.5"/></svg>; }
function ScheduleIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9 7 7M17 17l2.1 2.1M19.1 4.9 17 7M7 17l-2.1 2.1"/></svg>; }

function Row({ label, children }: { label: string; children: ReactNode }) {
  return <div><dt>{label}</dt><dd>{children}</dd></div>;
}

/** `<input type="time">` speaks HH:MM; the rule stores minutes past midnight. */
export function minutesToTime(value: number): string {
  const total = Math.max(0, Math.min(1439, Math.trunc(value)));
  return `${String(Math.floor(total / 60)).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
}

export function timeToMinutes(value: string): number {
  const [hour, minute] = value.split(":").map(Number);
  if (!Number.isFinite(hour) || !Number.isFinite(minute)) return 0;
  return Math.max(0, Math.min(1439, hour * 60 + minute));
}
