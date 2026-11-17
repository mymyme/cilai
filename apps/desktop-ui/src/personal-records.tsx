import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { randomUuid } from "./uuid";
import { invoke, listen } from "./host";
import { useModal } from "./use-modal";
import { createPortal } from "./page-portal";
import { SearchField } from "./search-field";
import { Select } from "./select";
import { CloseIcon } from "./icons";
import { ConfirmDialog } from "./confirm-dialog";
import { auditMoment, clockMoment, deadlineDay } from "./datetime";
import { aiRuntimeChangedEvent } from "./ai-runtime";
import { ConversationInfoRow } from "./conversation-view";
import { StatusChip } from "./file-status";
import { LoadFailure } from "./load-failure";
import { WorkbenchFilterGroup } from "./workbench-filter-group";
import "./personal-records.css";

export type PersonalRecordKind = "todo" | "memo";
export type PersonalReminderIntent = { runtimeSessionId: string; todoId: string; token: string };
type Filter = "current" | "history";
type Repeat = "once" | "daily" | "weekly";
type Scope = { kind: "project" | "conversation"; id: string };
type ScopeOption = Scope & { label: string };
type Item = {
  id: string;
  title: string;
  revision: number;
  updatedAt: number;
  sourceMessageId: string | null;
};
type Todo = Item & {
  details: string;
  state: "open" | "completed" | "canceled";
  dueAt: number | null;
  reminder: {
    input: { localTime: string; timeZone: string; repeat: Repeat };
    nextAt: number | null;
    lastAt: number | null;
    completed: boolean;
    snoozedAt: number | null;
  } | null;
};
type Memo = Item & { body: string; url: string | null; scope: Scope | null; scopeLabel: string | null; archived: boolean };
type Notice = {
  id: string;
  todoId: string;
  title: string;
  originalAt: number;
  scheduledAt: number;
  recordedAt: number;
  missed: boolean;
  count: number;
  desktop: "pending" | "claimed" | "submitted" | "failed" | "unavailable" | "disabled" | "skipped";
};
type Page<T> = { items: T[]; nextOffset: number | null };
type RecordItem = Todo | Memo;
type Mutation = { action: string; id?: string; revision?: number; item?: object; archived?: boolean; minutes?: number };
type Editor = { record: RecordItem | null; title: string; content: string; due: string; time: string; zone: string; repeat: Repeat; url: string; scope: string };

const recurrence: Record<Repeat, string> = { once: "单次", daily: "每天", weekly: "每周" };
const desktopStatus: Record<Notice["desktop"], string> = {
  pending: "系统通知未确认", claimed: "系统通知待核对", submitted: "已提交系统通知",
  failed: "系统通知失败", unavailable: "系统通知不可用", disabled: "系统通知已关闭", skipped: "",
};
const recordsChangedEvent = "cilai-personal-records-changed";

function useRecordsRefresh(runtimeSessionId: string, kind: PersonalRecordKind) {
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    const changed = () => setRefresh(value => value + 1);
    const recordsChanged = (event: Event) => {
      const detail = (event as CustomEvent<{ runtimeSessionId: string; kind: PersonalRecordKind }>).detail;
      if (detail?.runtimeSessionId === runtimeSessionId && detail.kind === kind) changed();
    };
    window.addEventListener(aiRuntimeChangedEvent, changed);
    window.addEventListener(recordsChangedEvent, recordsChanged);
    let active = true;
    const stop = kind === "todo" ? listen<{ runtimeSessionId: string }>("personal-reminders-changed", payload => {
      if (active && payload.runtimeSessionId === runtimeSessionId) changed();
    }) : null;
    void stop?.catch(() => {});
    return () => {
      active = false;
      window.removeEventListener(aiRuntimeChangedEvent, changed);
      window.removeEventListener(recordsChangedEvent, recordsChanged);
      void stop?.then(dispose => dispose(), () => {});
    };
  }, [runtimeSessionId, kind]);
  return [refresh, () => setRefresh(value => value + 1)] as const;
}

async function listRecords(runtimeSessionId: string, kind: PersonalRecordKind, filter: Filter, query: string, offset: number) {
  const page = await invoke<Page<RecordItem>>("ai_pa_records", { runtimeSessionId, kind, request: { action: "list", query, filter, offset } });
  if (!Array.isArray(page?.items)) throw new Error("个人记录服务返回了无效列表。");
  return page;
}

function recordError(error: unknown) {
  const message = String(error).replace(/^Error: /, "");
  const code = message.match(/\b(?:pa_[a-z_]+|ai_profile_changed)\b/)?.[0] ?? message;
  const labels: Record<string, string> = {
    pa_record_conflict: "记录已更新。请保留当前修改，重新打开最新记录后再保存。",
    pa_record_missing: "这条记录已不可用。",
    pa_record_input_invalid: "请检查填写内容。",
    pa_reminder_time_invalid: "该提醒时间无效，请重新选择。",
    pa_reminder_in_past: "提醒时间应晚于现在。",
    pa_time_zone_invalid: "请输入有效时区，如 Asia/Shanghai。",
    pa_due_time_invalid: "截止时间无效。",
    pa_snooze_invalid: "本次提醒已处理或尚未触发。",
    pa_snooze_overlaps_next: "稍后提醒时间应早于下一次重复提醒。",
    pa_todo_closed: "这项待办已结束。",
    pa_occurrence_completed: "本次待办已完成。",
    pa_memo_content_required: "请填写正文或链接。",
    pa_memo_url_invalid: "请输入有效的 HTTP 或 HTTPS 链接。",
    pa_scope_unavailable: "关联的工作已不可用，请重新选择。",
    ai_profile_changed: "账号或工作台会话已变化，请重新打开。",
  };
  return labels[code] ?? message;
}

function scopeValue(scope: Scope | null) {
  return scope ? `${scope.kind}:${scope.id}` : "";
}

function localInput(at: number | null | undefined) {
  if (at == null) return "";
  const date = new Date(at);
  const pad = (part: number) => String(part).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function editorFor(record: RecordItem | null): Editor {
  const todo = record && "state" in record ? record : null;
  const memo = record && "body" in record ? record : null;
  return {
    record, title: record?.title ?? "", content: todo?.details ?? memo?.body ?? "",
    due: localInput(todo?.dueAt), time: todo?.reminder?.input.localTime ?? "",
    zone: todo?.reminder?.input.timeZone ?? Intl.DateTimeFormat().resolvedOptions().timeZone,
    repeat: todo?.reminder?.input.repeat ?? "once", url: memo?.url ?? "",
    scope: scopeValue(memo?.scope ?? null),
  };
}

function group(todo: Todo, now: number) {
  if (todo.state !== "open") return todo.state === "completed" ? "已完成" : "已取消";
  const at = todo.dueAt ?? todo.reminder?.snoozedAt ?? todo.reminder?.nextAt;
  if (at == null) return todo.reminder?.lastAt ? "已提醒" : "未安排";
  if (at < now) return "逾期";
  return new Date(at).toDateString() === new Date(now).toDateString() ? "今天" : "接下来";
}

function todoMeta(todo: Todo, format = auditMoment) {
  const reminder = todo.reminder;
  const next = reminder?.snoozedAt ?? reminder?.nextAt;
  return [
    todo.dueAt == null ? null : `截止 ${format(todo.dueAt)}`,
    next == null ? (reminder?.lastAt ? "已提醒" : null) : `提醒 ${format(next)}`,
    reminder && reminder.input.repeat !== "once" ? recurrence[reminder.input.repeat] : null,
    reminder?.completed ? "本次已完成" : null,
  ].filter(Boolean).join(" · ");
}

export function PersonalRecordSection({ kind, runtimeSessionId, updateKey, limit, now, onOpen }: {
  kind: PersonalRecordKind;
  runtimeSessionId: string;
  updateKey: string;
  limit?: number;
  now: number;
  onOpen: (kind: PersonalRecordKind, id?: string) => void;
}) {
  const title = kind === "todo" ? "待办" : "备忘录";
  const [refresh, reload] = useRecordsRefresh(runtimeSessionId, kind);
  const [result, setResult] = useState<{ items: RecordItem[] } | { error: string } | null>(null);
  useEffect(() => {
    let active = true;
    void listRecords(runtimeSessionId, kind, "current", "", 0).then(page => {
      if (active) setResult({ items: page.items });
    }, failure => {
      if (active) setResult({ error: recordError(failure) });
    });
    return () => { active = false; };
  }, [runtimeSessionId, kind, refresh, updateKey]);

  const items = result && "items" in result ? result.items.slice(0, limit) : [];
  const formatTime = (time: number) => {
    const day = new Date(time).toDateString() === new Date(now).toDateString() ? "今天" : deadlineDay(time, now);
    return `${day} ${clockMoment(time)}`;
  };
  return <section className="pa-activity-section" aria-label={title}>
    <div className="pa-activity-section-header">
      <h3>{title}</h3>
      {limit === undefined ? <button type="button" className="secondary-button" onClick={() => onOpen(kind)}>管理{title}</button> : null}
    </div>
    {!result ? <p className="pa-followup-empty ai-conversation-info-empty" role="status">正在读取{title}…</p>
      : "error" in result ? <div className="pa-work-page-actions"><LoadFailure what={title} error={result.error} onRetry={reload}/></div>
      : !items.length ? <p className="pa-followup-empty ai-conversation-info-empty" role="status">暂无{title}</p>
      : <ul className="pa-work-list">{items.map(item => {
        const todo = "state" in item ? item : null;
        const memo = "body" in item ? item : null;
        const excerpt = memo && (memo.body.trim().replace(/\s+/g, " ") || memo.url);
        const meta = todo ? todoMeta(todo, formatTime) || "未安排" : memo?.scopeLabel || `更新于 ${formatTime(item.updatedAt)}`;
        return <li key={item.id}>
          <ConversationInfoRow className="pa-work-row" onClick={() => onOpen(kind, item.id)}>
            <span className="pa-work-name">
              <strong title={item.title}>{item.title}</strong>
              {excerpt ? <span className="pa-record-excerpt">{excerpt}</span> : null}
              <span className="pa-record-preview-meta">{meta}</span>
            </span>
            {todo && group(todo, Date.now()) === "逾期" ? <StatusChip label="逾期" tone="attention"/> : null}
          </ConversationInfoRow>
        </li>;
      })}</ul>}
  </section>;
}

export function PersonalRecordsDialog(props: {
  kind: PersonalRecordKind;
  runtimeSessionId: string;
  initialRecordId?: string;
  updateKey?: string;
  scopes: ScopeOption[];
  onClose: () => void;
  onLocateSource: (id: string) => void;
}) {
  const { kind, runtimeSessionId, initialRecordId, updateKey } = props;
  const heading = useId();
  const selectId = useId();
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const [view, setView] = useState<Filter | "notices">("current");
  const notices = view === "notices";
  const [page, setPage] = useState<Page<RecordItem | Notice> | null>(null);
  const [refresh, reload] = useRecordsRefresh(runtimeSessionId, kind);
  const [editor, setEditor] = useState<Editor | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [confirm, setConfirm] = useState<{ label: string; description: string; request: Mutation } | null>(null);
  const operation = useRef<{ payload: string; id: string } | null>(null);
  const pending = useRef(false);
  const lifecycle = useRef(0);
  const pageRequest = useRef(0);
  const modal = useModal<HTMLDivElement>({ onClose: props.onClose, busy, active: !confirm });
  const title = kind === "todo" ? "待办" : "备忘录";
  const record = editor?.record;

  useEffect(() => {
    lifecycle.current += 1;
    return () => { lifecycle.current += 1; pageRequest.current += 1; };
  }, [runtimeSessionId]);
  useEffect(() => {
    const timer = setTimeout(() => setSearch(query), 180);
    return () => clearTimeout(timer);
  }, [query]);

  async function load(offset: number) {
    const generation = ++pageRequest.current;
    setLoading(true);
    try {
      const value = notices
        ? await invoke<Page<Notice>>("ai_pa_reminder_history", { runtimeSessionId, offset })
        : await listRecords(runtimeSessionId, kind, view, search, offset);
      if (generation !== pageRequest.current) return;
      setPage(previous => {
        const items = offset ? [...(previous?.items ?? []), ...value.items] : value.items;
        return { items: [...new Map(items.map(item => [item.id, item])).values()], nextOffset: value.nextOffset };
      });
    } catch (failure) {
      if (generation === pageRequest.current) setError(recordError(failure));
    } finally {
      if (generation === pageRequest.current) setLoading(false);
    }
  }
  useEffect(() => {
    setPage(null);
    void load(0);
    return () => { pageRequest.current += 1; };
  }, [kind, runtimeSessionId, search, view, refresh, updateKey]);

  async function open(id: string) {
    const generation = lifecycle.current;
    setError("");
    try {
      const value = await invoke<RecordItem>("ai_pa_records", { runtimeSessionId, kind, request: { action: "get", id } });
      if (generation === lifecycle.current) setEditor(editorFor(value));
    } catch (failure) {
      if (generation === lifecycle.current) setError(recordError(failure));
    }
  }
  useEffect(() => {
    if (initialRecordId) void open(initialRecordId);
  }, [initialRecordId, kind, runtimeSessionId]);

  async function change(request: Mutation) {
    if (pending.current) return;
    pending.current = true;
    setError("");
    setBusy(true);
    const generation = lifecycle.current;
    const payload = JSON.stringify(request);
    if (operation.current?.payload !== payload) operation.current = { payload, id: randomUuid() };
    try {
      await invoke("ai_pa_records", { runtimeSessionId, kind, request, operationId: operation.current.id });
      if (generation !== lifecycle.current) return;
      operation.current = null;
      setConfirm(null);
      setEditor(null);
      window.dispatchEvent(new CustomEvent(recordsChangedEvent, { detail: { runtimeSessionId, kind } }));
    } catch (failure) {
      if (generation === lifecycle.current) setError(recordError(failure));
    } finally {
      pending.current = false;
      if (generation === lifecycle.current) setBusy(false);
    }
  }

  function edit<K extends keyof Editor>(key: K, value: Editor[K]) {
    setEditor(current => current && { ...current, [key]: value });
  }

  function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!editor) return;
    const memoScope = props.scopes.find(scope => scopeValue(scope) === editor.scope);
    const oldScope = editor.record && "scope" in editor.record ? editor.record.scope : null;
    const scope = memoScope ? { kind: memoScope.kind, id: memoScope.id } : (editor.scope ? oldScope : null);
    let dueAt = null;
    if (kind === "todo" && editor.due) {
      const date = new Date(editor.due);
      if (!Number.isFinite(date.getTime()) || localInput(date.getTime()) !== editor.due) {
        setError("该截止时间无效，请重新选择。");
        return;
      }
      dueAt = date.toISOString();
    }
    const item = kind === "todo"
      ? { title: editor.title, details: editor.content, dueAt, reminder: editor.time ? {
        localTime: editor.time, timeZone: editor.zone.trim(), repeat: editor.repeat,
      } : null }
      : { title: editor.title, body: editor.content, url: editor.url.trim() || null, scope };
    void change({ action: "save", id: record?.id, revision: record?.revision, item });
  }

  const todo = record && "state" in record ? record : null;
  const memo = record && "body" in record ? record : null;
  const closed = Boolean(todo && todo.state !== "open");
  const cancelTodoLabel = todo?.reminder && todo.reminder.input.repeat !== "once" ? "停止重复" : "取消待办";
  const filterName = kind === "todo" ? "历史" : "已归档";
  const scopeOptions = [{ value: "", label: "无" }, ...props.scopes.map(scope => ({
    value: scopeValue(scope), label: `${scope.kind === "project" ? "项目" : "工作"} · ${scope.label}`,
  }))];
  if (memo?.scope && !scopeOptions.some(option => option.value === scopeValue(memo.scope))) {
    scopeOptions.push({ value: scopeValue(memo.scope), label: memo.scopeLabel ?? "原关联" });
  }

  const groupNames = ["逾期", "今天", "接下来", "已提醒", "未安排", "已完成", "已取消"];
  const now = Date.now();
  const rows = page?.items ?? [];
  const groups = kind === "todo" && !notices
    ? groupNames.map(name => ({ name, items: rows.filter((item): item is Todo => "state" in item && group(item, now) === name) }))
      .filter(entry => entry.items.length > 0)
    : [];
  const row = (item: RecordItem) => <ConversationInfoRow className="pa-record-row" key={item.id} onClick={() => void open(item.id)}>
    <strong>{item.title}</strong>
    <small>{"state" in item ? todoMeta(item) : [item.scopeLabel, auditMoment(item.updatedAt)].filter(Boolean).join(" · ")}</small>
  </ConversationInfoRow>;
  const action = (name: string, more: Partial<Mutation> = {}) => ({ action: name, id: record?.id, revision: record?.revision, ...more });
  const back = () => { setError(""); setEditor(null); };

  return createPortal(<>
    <div className="confirm-backdrop" ref={modal} role="dialog" aria-modal="true" aria-labelledby={heading} inert={confirm ? true : undefined}
      onMouseDown={event => { if (event.target === event.currentTarget && !busy && !confirm) props.onClose(); }}>
      <section className="confirm-dialog pa-record-dialog">
        <header className="pa-record-header">
          <h2 id={heading}>{editor ? (record ? (kind === "todo" ? "待办详情" : "备忘录详情") : `新建${title}`) : title}</h2>
          <button type="button" className="pa-record-icon" aria-label="关闭" title="关闭" autoFocus onClick={props.onClose} disabled={busy}><CloseIcon/></button>
        </header>
        {error && !confirm ? <p className="workbench-operation-error" role="alert">{error}</p> : null}
        {editor ? <form className="pa-record-form" onSubmit={save}>
          <div className="pa-record-form-body">
            <label>标题<input autoFocus required maxLength={160} value={editor.title} disabled={busy || closed} onChange={event => edit("title", event.currentTarget.value)}/></label>
            <label>{kind === "todo" ? "说明" : "正文"}<textarea rows={kind === "todo" ? 3 : 6} maxLength={kind === "todo" ? 3000 : 6000} value={editor.content} disabled={busy || closed} onChange={event => edit("content", event.currentTarget.value)}/></label>
            {kind === "todo" ? <>
              <div className="pa-record-fields">
                <label>截止时间<input type="datetime-local" value={editor.due} disabled={busy || closed} onChange={event => edit("due", event.currentTarget.value)}/></label>
                <label>提醒时间<input type="datetime-local" value={editor.time} disabled={busy || closed} onChange={event => edit("time", event.currentTarget.value)}/></label>
              </div>
              {editor.time ? <div className="pa-record-fields">
                <label>时区<input required maxLength={80} value={editor.zone} disabled={busy || closed} onChange={event => edit("zone", event.currentTarget.value)}/></label>
                <div className="pa-record-field"><label htmlFor={selectId}>重复</label><Select id={selectId} ariaLabel="重复" value={editor.repeat} disabled={busy || closed}
                  options={Object.entries(recurrence).map(([value, label]) => ({ value, label }))}
                  onChange={value => edit("repeat", value as Repeat)}/></div>
              </div> : null}
              {todo ? <p className="pa-record-meta">{todo.state !== "open" ? group(todo, now) : todoMeta(todo)}</p> : null}
            </> : <>
              <label>链接<input type="url" maxLength={2048} value={editor.url} disabled={busy} onChange={event => edit("url", event.currentTarget.value)}/></label>
              <div className="pa-record-field"><label htmlFor={selectId}>关联工作</label><Select id={selectId} ariaLabel="关联工作" value={editor.scope} options={scopeOptions} disabled={busy} onChange={value => edit("scope", value)}/></div>
              {memo?.url ? <button className="pa-record-text-action" type="button" disabled={busy} onClick={() =>
                void invoke("ai_pa_open_memo_link", { runtimeSessionId, id: memo.id }).catch(failure => setError(recordError(failure)))}>打开已保存链接</button> : null}
            </>}
            {record?.sourceMessageId ? <button type="button" className="pa-record-text-action" disabled={busy} onClick={() => props.onLocateSource(record.sourceMessageId!)}>定位原消息</button> : null}
          </div>
          <footer className="confirm-actions pa-record-footer">
            <button type="button" disabled={busy} onClick={back}>返回</button>
            {todo && !closed ? <>
              <button type="button" disabled={busy} onClick={() => {
                setError("");
                setConfirm({
                  label: cancelTodoLabel,
                  description: "此事项会移入历史，后续提醒将停止。",
                  request: action("cancel"),
                });
              }}>{cancelTodoLabel}</button>
              {todo.reminder?.lastAt != null && !todo.reminder.completed ? <button type="button" disabled={busy} onClick={() => void change(action("snooze", { minutes: 10 }))}>稍后 10 分钟</button> : null}
              <button type="button" disabled={busy || todo.reminder?.completed} onClick={() => void change(action("complete"))}>{todo.reminder && todo.reminder.input.repeat !== "once" ? "完成本次" : "完成"}</button>
            </> : null}
            {memo ? <>
              <button type="button" disabled={busy} onClick={() => {
                setError("");
                setConfirm({ label: "删除备忘录", description: "删除后无法恢复这条备忘录。", request: action("delete") });
              }}>删除</button>
              <button type="button" disabled={busy} onClick={() => void change(action("archive", { archived: !memo.archived }))}>{memo.archived ? "恢复" : "归档"}</button>
            </> : null}
            {!closed ? <button className="primary" type="submit" disabled={busy || !editor.title.trim() || (kind === "memo" && !editor.content.trim() && !editor.url.trim())}>{busy ? "保存中…" : "保存"}</button> : null}
          </footer>
        </form> : <>
          <div className="pa-record-toolbar">
            <WorkbenchFilterGroup label="列表筛选" value={view} onChange={next => { setError(""); setView(next); }}
              options={kind === "todo" ? [["current", "当前"], ["history", filterName], ["notices", "提醒记录"]] : [["current", "当前"], ["history", filterName]]}/>
            <button type="button" className="primary" onClick={() => { setError(""); setEditor(editorFor(null)); }}>新建</button>
          </div>
          {!notices ? <SearchField ariaLabel={`搜索${title}`} placeholder={`搜索${title}`} value={query} onChange={setQuery} clearable/> : null}
          <div className="pa-record-list" aria-busy={loading}>
            {!rows.length ? <p className="pa-record-empty">{loading ? "正在读取…" : error ? "读取失败" : (notices ? "暂无提醒记录" : `暂无${title}`)}</p> : null}
            {groups.map(({ name, items }) => <section key={name}><h3>{name}</h3>{items.map(row)}</section>)}
            {kind === "memo" && !notices ? rows.filter((item): item is Memo => "body" in item).map(row) : null}
            {notices ? rows.filter((item): item is Notice => "todoId" in item).map(notice =>
              <ConversationInfoRow className="pa-record-row" key={notice.id} onClick={() => void open(notice.todoId)}>
                <strong>{notice.title}<span className="pa-record-notice-state">{notice.missed ? "已错过" : "到点"}</span></strong>
                <small>{auditMoment(notice.originalAt)}{notice.count > 1 ? ` · 共 ${notice.count} 次` : ""}{desktopStatus[notice.desktop] ? ` · ${desktopStatus[notice.desktop]}` : ""}</small>
              </ConversationInfoRow>) : null}
            {page?.nextOffset != null ? <button className="pa-record-text-action" type="button" disabled={loading} onClick={() => void load(page.nextOffset!)}>加载更多</button> : null}
            {error ? <button className="pa-record-text-action" type="button" onClick={() => { setError(""); reload(); }}>重新读取</button> : null}
          </div>
        </>}
      </section>
    </div>
    {confirm ? <ConfirmDialog title={confirm.label} description={confirm.description} confirmLabel={confirm.label} danger busy={busy} notice={error ? { message: error } : null}
      onCancel={() => { setError(""); setConfirm(null); }} onConfirm={() => void change(confirm.request)}/> : null}
  </>, document.body);
}
