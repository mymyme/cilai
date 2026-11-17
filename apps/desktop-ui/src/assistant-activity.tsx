import { useCallback, useEffect, useLayoutEffect, useRef, useState, type Ref } from "react";
import type { WorkSupplement } from "./assistant-work";
import { CloseIcon, ConversationIconButton, ConversationInfoRow, FileIcon } from "./conversation-view";
import { conversationMessageMoment } from "./datetime";
import { StatusChip } from "./file-status";
import { MoreIcon } from "./icons";
import { LoadFailure } from "./load-failure";
import { PersonalRecordSection, type PersonalRecordKind } from "./personal-records";
import { RichMenu, RichMenuItem } from "./rich-menu";
import { WorkbenchFilterGroup } from "./workbench-filter-group";

export interface AssistantTrackingItem {
  id: string;
  runId: string | null;
  title: string;
  status: string;
  updatedAt: number;
  projectName?: string | null;
  waiting?: { question: string; impact: string } | null;
  operationId?: string | null;
  messageId?: string | null;
  supplementMessageId?: string | null;
  requestMessageId?: string | null;
  artifactRunId?: string;
  artifacts?: { artifactId: string; label: string }[];
}

export type AssistantTrackingCursor = [updatedAt: number, id: string];
export type AssistantTrackingHistory = {
  tracking: AssistantTrackingItem[];
  next: AssistantTrackingCursor | null;
};
export type AssistantTrackingHistoryView = "completed" | "history";

const emptyHistory: AssistantTrackingHistory & { loading: boolean; error: string | null; reload: boolean } = {
  tracking: [], next: null, loading: false, error: null, reload: false,
};

const filters = [
  ["current", "当前"],
  ["attention", "待我处理"],
  ["active", "进行中"],
  ["done", "已完成"],
  ["all", "全部"],
] as const;
type Filter = typeof filters[number][0];
const tabs = [["overview", "总览"], ["followup", "跟进"], ["todo", "待办"], ["memo", "备忘录"]] as const;
export type AssistantActivityTab = typeof tabs[number][0];

const attention = new Set(["waiting_user", "waiting_permission", "failed", "delivery_error", "unavailable", "interrupted", "paused"]);
const failed = new Set(["failed", "unavailable", "interrupted"]);
const active = new Set(["pending", "queued", "running", "terminating", "permission_approved"]);
const completed = new Set(["completed", "result_available"]);

function group(item: AssistantTrackingItem) {
  if (attention.has(item.status)) return "attention";
  if (active.has(item.status)) return "active";
  if (completed.has(item.status)) return "done";
  return "all";
}

export function AssistantActivity({ items, open, pageActive, tab, onTabChange, recordsSessionId, recordsUpdateKey, onOpenRecords,
  closeRef, onClose, now, labels, onLoadHistory, onSupplement, onLocate, onOpenArtifact, onStop, onUnfollow }: {
  items: AssistantTrackingItem[];
  open: boolean;
  pageActive: boolean;
  tab: AssistantActivityTab;
  onTabChange: (tab: AssistantActivityTab) => void;
  recordsSessionId: string | null;
  recordsUpdateKey: string;
  onOpenRecords: (kind: PersonalRecordKind, id?: string) => void;
  closeRef: Ref<HTMLButtonElement>;
  onClose: () => void;
  now: number;
  labels: Record<string, string>;
  onLoadHistory: (view: AssistantTrackingHistoryView, before: AssistantTrackingCursor | null) => Promise<AssistantTrackingHistory>;
  onSupplement: (target: WorkSupplement) => void;
  onLocate: (messageId: string) => void;
  onOpenArtifact: (runId: string, artifactId: string) => void;
  onStop: (runId: string) => void;
  onUnfollow: (operationId: string) => Promise<boolean>;
}) {
  const [filter, setFilter] = useState<Filter>("current");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [history, setHistory] = useState(emptyHistory);
  const historyRef = useRef(history);
  historyRef.current = history;
  const historyRequest = useRef(0);
  const scroll = useRef<HTMLDivElement>(null);
  const historyView = filter === "done" ? "completed" : "history";
  const historyEnabled = pageActive && open && tab === "followup" && (filter === "done" || filter === "all" && historyOpen);
  const historyUpdateKey = JSON.stringify([recordsUpdateKey, items.map(item => [item.id, item.runId, item.status, item.updatedAt])]);
  useLayoutEffect(() => {
    if (scroll.current) scroll.current.scrollTop = 0;
  }, [tab, open]);
  const loadHistory = useCallback(async (view: AssistantTrackingHistoryView, before: AssistantTrackingCursor | null = null, preserveCount = 0) => {
    const request = ++historyRequest.current;
    const rows = new Map((before ? historyRef.current.tracking : []).map(item => [item.id, item]));
    setHistory(current => ({ ...(before || preserveCount ? current : emptyHistory), loading: true, error: null, reload: false }));
    try {
      let cursor = before;
      do {
        const page = await onLoadHistory(view, cursor);
        if (request !== historyRequest.current) return;
        for (const item of page.tracking) rows.set(item.id, item);
        cursor = page.next;
      } while (cursor && rows.size < preserveCount);
      setHistory({ tracking: [...rows.values()], next: cursor, loading: false, error: null, reload: false });
    } catch (error) {
      if (request === historyRequest.current) setHistory(current => ({ ...current, loading: false, error: String(error), reload: preserveCount > 0 }));
    }
  }, [onLoadHistory]);

  useEffect(() => {
    if (!historyEnabled) return;
    void loadHistory(historyView, null, historyRef.current.tracking.length);
    return () => { historyRequest.current++; };
  }, [historyEnabled, historyView, historyUpdateKey, loadHistory]);

  async function unfollow(item: AssistantTrackingItem) {
    if (!item.operationId || !await onUnfollow(item.operationId)) return;
    historyRequest.current++;
    setHistory(current => ({ ...current, tracking: current.tracking.filter(row => row.id !== item.id), loading: false }));
    setExpanded(current => current === item.id ? null : current);
  }

  function changeFilter(next: Filter) {
    if (next === filter) return;
    historyRequest.current++;
    setFilter(next);
    setExpanded(null);
    setHistoryOpen(false);
    setHistory(emptyHistory);
  }

  function toggleHistory() {
    setHistoryOpen(!historyOpen);
    if (historyOpen) {
      historyRequest.current++;
      setHistory(emptyHistory);
    }
  }

  if (!open) return null;
  const attentionCount = items.filter(item => attention.has(item.status)).length;
  const options = filters.map(([key, label]) => [key, key === "attention" && attentionCount > 0 ? `${label} ${attentionCount}` : label] as const);
  const ordered = [...items]
    .sort((a, b) => Number(group(b) === "attention") - Number(group(a) === "attention") || b.updatedAt - a.updatedAt);
  const visible = ordered.filter(item => filter === "all" || filter === "current" || group(item) === filter);
  const overview = tab === "overview";
  const followup = tab === "followup";
  const currentPreview = overview ? ordered.filter(item => attention.has(item.status) || active.has(item.status)).slice(0, 3) : [];
  const currentIds = new Set(items.map(item => item.id));
  const historyRows = history.tracking.filter(item => !currentIds.has(item.id));
  const renderList = (rows: AssistantTrackingItem[]) => <ul className="pa-work-list">{rows.map(item => {
      const stopRun = active.has(item.status) || item.status === "waiting_permission" || item.status === "paused" ? item.runId : null;
      const open = expanded === item.id;
      const messageId = item.messageId ?? item.supplementMessageId ?? item.requestMessageId;
      const locateLabel = item.messageId ? "定位消息" : item.supplementMessageId ? "定位补充消息" : "定位原委托";
      const tone = failed.has(item.status) ? "danger" : attention.has(item.status) ? "attention" : completed.has(item.status) ? "success" : "neutral";
      const row = <>
        <span className="pa-work-name">
          <strong title={item.title}>{item.title}</strong>
          <span className="pa-work-meta">
            {item.projectName ? <span title={item.projectName}>{item.projectName}</span> : null}
            <time dateTime={new Date(item.updatedAt).toISOString()}>{conversationMessageMoment(item.updatedAt, now)}</time>
          </span>
        </span>
        <StatusChip label={labels[item.status] ?? item.status} tone={tone}/>
      </>;
      return <li key={item.id} data-expanded={open}>
        <ConversationInfoRow className="pa-work-row" aria-expanded={open} onClick={() => setExpanded(open ? null : item.id)}>{row}</ConversationInfoRow>
        {open ? <div className="pa-work-detail">
          {!messageId ? <p className="pa-work-note ai-conversation-info-empty">没有可定位的助理消息</p> : null}
          {item.waiting ? <div className="pa-work-question">
            <strong>{item.waiting.question}</strong><p>{item.waiting.impact}</p>
          </div> : null}
          {item.artifactRunId && item.artifacts?.length ? <ul className="pa-work-files">{item.artifacts.map(file =>
            <li key={file.artifactId}>
              <ConversationInfoRow className="ai-conversation-info-item" title={file.label} onClick={() => onOpenArtifact(item.artifactRunId!, file.artifactId)}>
                <FileIcon/><strong>{file.label}</strong>
              </ConversationInfoRow>
            </li>)}</ul> : null}
          <div className="pa-work-actions">
            {item.waiting && item.runId ? <button type="button" className="secondary-button" onClick={() => onSupplement({ id: item.id, runId: item.runId!, title: item.title })}>补充信息</button> : null}
            {messageId ? <button type="button" className="secondary-button" onClick={() => onLocate(messageId)}>
              {locateLabel}
            </button> : null}
            {stopRun || item.operationId ? <div className="pa-work-more">
              <RichMenu ariaLabel="跟进操作" align="end" width={152} keyboardNavigation
                trigger={({ open, toggle, triggerRef }) => <ConversationIconButton ref={triggerRef} onClick={toggle}
                  aria-label="更多操作" aria-haspopup="menu" aria-expanded={open}><MoreIcon/></ConversationIconButton>}>
                {stopRun ? <RichMenuItem label="停止执行" onSelect={() => onStop(stopRun)}/> : null}
                {item.operationId ? <RichMenuItem label="停止跟进" onSelect={() => void unfollow(item)}/> : null}
              </RichMenu>
            </div> : null}
          </div>
        </div> : null}
      </li>;
    })}</ul>;

  return <aside id="personal-assistant-activity" className="pa-side ai-conversation-info-panel" aria-label="个人事项">
    <header>
      <h2>个人事项</h2>
      <ConversationIconButton ref={closeRef} onClick={onClose} aria-label="关闭个人事项"><CloseIcon/></ConversationIconButton>
    </header>
    <div className="pa-followup-controls">
      <WorkbenchFilterGroup compact className="pa-activity-tabs" label="个人事项分类" options={tabs} value={tab} onChange={onTabChange}/>
    </div>
    {followup ? <div className="pa-followup-controls pa-work-filters">
      <WorkbenchFilterGroup compact label="工作状态" options={options} value={filter} onChange={changeFilter}/>
    </div> : null}
    <div className="pa-work-scroll" ref={scroll}>
      {overview ? <section className="pa-activity-section" aria-label="工作跟进">
        <div className="pa-activity-section-header"><h3>工作跟进</h3></div>
        {currentPreview.length ? renderList(currentPreview)
          : <p className="pa-followup-empty ai-conversation-info-empty" role="status">暂无跟进</p>}
      </section> : null}
      {recordsSessionId && (overview || tab === "todo") ? <PersonalRecordSection key={`${recordsSessionId}:todo`}
        kind="todo" runtimeSessionId={recordsSessionId} updateKey={recordsUpdateKey} limit={overview ? 3 : undefined} now={now} onOpen={onOpenRecords}/> : null}
      {recordsSessionId && (overview || tab === "memo") ? <PersonalRecordSection key={`${recordsSessionId}:memo`}
        kind="memo" runtimeSessionId={recordsSessionId} updateKey={recordsUpdateKey} limit={overview ? 2 : undefined} now={now} onOpen={onOpenRecords}/> : null}
      {followup && filter !== "done" ? (visible.length ? renderList(visible)
        : <p className="pa-followup-empty ai-conversation-info-empty" role="status">暂无{filter === "all" ? "当前" : ""}事项</p>) : null}
      {followup && filter === "all" ? <div className="pa-work-history-toggle">
        <button type="button" className="secondary-button" aria-expanded={historyOpen} aria-controls="pa-tracking-history" onClick={toggleHistory}>历史事项</button>
      </div> : null}
      {followup && (filter === "done" || filter === "all" && historyOpen) ? <div id="pa-tracking-history" aria-busy={history.loading}>
        {historyRows.length ? renderList(historyRows) : !history.loading && !history.error && !history.next
          ? <p className="pa-followup-empty ai-conversation-info-empty" role="status">{filter === "done" ? "暂无已完成事项" : "暂无历史事项"}</p> : null}
        {history.error ? <div className="pa-work-page-actions"><LoadFailure what="历史事项" error={history.error} onRetry={() => void loadHistory(historyView, history.reload ? null : history.next, history.reload ? history.tracking.length : 0)}/></div> : null}
        {history.loading ? <p className="pa-followup-empty ai-conversation-info-empty" role="status">正在读取历史事项…</p> : null}
        {history.next && !history.error && !history.loading ? <div className="pa-work-page-actions">
          <button type="button" className="secondary-button" onClick={() => void loadHistory(historyView, history.next)}>查看更早</button>
        </div> : null}
      </div> : null}
    </div>
  </aside>;
}
