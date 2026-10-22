import { conversationArchiveChanged, mergeArchivedConversations, setConversationsArchived, type ConversationArchiveChange } from "./conversation-archive";
import { useEffect, useMemo, useRef, useState } from "react";
import { aiErrorMessage, deleteAiConversations, emptyAiSnapshot, loadAiSnapshot, type AiConversation, type AiRuntimeSnapshot } from "./ai-runtime";
import { assistantMessageBody, conversationTopic, MessageAttachments, type AiProjectOption } from "./ai-workbench";
import type { MessageSink } from "./app-message";
import { ConfirmDialog } from "./confirm-dialog";
import { auditMoment } from "./datetime";
import { ArchiveIcon, BackIcon, ChevronIcon, TreeFolderIcon } from "./icons";
import { MarkdownContent } from "./markdown-content";
import { SearchField } from "./search-field";
import { Select } from "./select";
import "./archived-conversations.css";

interface PendingDelete {
  ids: string[];
  title: string | null;
}

function ArchiveTrashIcon() {
  return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M4 6h12M8 3h4l1 3H7l1-3ZM6 6l1 11h6l1-11M9 9v5M11 9v5"/></svg>;
}

export function ArchivedConversationsPage(props: { projects: AiProjectOption[]; onMessage: MessageSink; openIntent?: { conversationId: string; token: number } | null }) {
  const [snapshot, setSnapshot] = useState<AiRuntimeSnapshot>(emptyAiSnapshot);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const [query, setQuery] = useState("");
  const [projectFilter, setProjectFilter] = useState("all");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(props.openIntent?.conversationId ?? null);
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [pendingDelete, setPendingDelete] = useState<PendingDelete | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [restoringId, setRestoringId] = useState<string | null>(null);
  const restoring = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null);

  const latestArchiveRevision = useRef(0);
  useEffect(() => { setSelectedId(props.openIntent?.conversationId ?? null); }, [props.openIntent]);
  useEffect(() => {
    const changed = (event: Event) => {
      const result = (event as CustomEvent<ConversationArchiveChange>).detail;
      latestArchiveRevision.current = Math.max(latestArchiveRevision.current, result.stateRevision);
      setSnapshot((current) => mergeArchivedConversations(current, result));
      if (!result.archived) {
        const restored = new Set(result.conversations.map((item) => item.conversationId));
        setSelectedId((current) => current && restored.has(current) ? null : current);
      }
    };
    window.addEventListener(conversationArchiveChanged, changed);
    return () => window.removeEventListener(conversationArchiveChanged, changed);
  }, []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    void loadAiSnapshot().then((result) => {
      if (cancelled) return;
      if (result.stateRevision < latestArchiveRevision.current) { setRetry((current) => current + 1); return; }
      setSnapshot(result);
    }).catch((reason: unknown) => {
      if (!cancelled) setError(aiErrorMessage(reason));
    }).finally(() => {
      if (!cancelled) setLoading(false);
    });
    return () => { cancelled = true; };
  }, [retry]);

  useEffect(() => { heading.current?.focus(); }, [selectedId]);

  const messagesByConversation = useMemo(() => {
    const result = new Map<string, AiRuntimeSnapshot["messages"]>();
    for (const message of snapshot.messages) {
      const messages = result.get(message.conversationId) ?? [];
      messages.push(message);
      result.set(message.conversationId, messages);
    }
    for (const messages of result.values()) messages.sort((left, right) => left.createdAt - right.createdAt);
    return result;
  }, [snapshot.messages]);

  const groups = useMemo(() => {
    const result = new Map<string, { key: string; name: string; items: { conversation: AiConversation; title: string }[] }>();
    const projectNames = new Map(props.projects.map((project) => [project.projectId, project.displayName]));
    const archived = snapshot.conversations.filter((conversation) => conversation.archived)
      .sort((left, right) => right.updatedAt - left.updatedAt);
    for (const conversation of archived) {
      const projectId = conversation.context.projectId;
      const key = projectId ? `project:${projectId}` : "unassigned";
      const name = projectId ? projectNames.get(projectId) ?? conversation.context.projectName ?? "原项目" : "未分类";
      const group = result.get(key) ?? { key, name, items: [] };
      group.items.push({ conversation, title: conversationTopic(conversation, messagesByConversation.get(conversation.conversationId) ?? []) });
      result.set(key, group);
    }
    return [...result.values()];
  }, [snapshot.conversations, props.projects, messagesByConversation]);

  const normalizedQuery = query.trim().toLocaleLowerCase("zh-CN");
  const filteredGroups = groups.filter((group) => projectFilter === "all" || group.key === projectFilter)
    .map((group) => ({ ...group, items: group.items.filter(({ conversation, title }) =>
      [title, conversation.title, group.name].some((text) => text.toLocaleLowerCase("zh-CN").includes(normalizedQuery))) }))
    .filter((group) => group.items.length > 0);
  const count = groups.reduce((total, group) => total + group.items.length, 0);
  const filteredCount = filteredGroups.reduce((total, group) => total + group.items.length, 0);
  const filteredIds = filteredGroups.flatMap((group) => group.items.map((item) => item.conversation.conversationId));
  const allFilteredSelected = filteredIds.length > 0 && filteredIds.every((id) => selectedIds.has(id));
  const selected = snapshot.conversations.find((conversation) => conversation.conversationId === selectedId && conversation.archived);
  const selectedMessages = selected ? messagesByConversation.get(selected.conversationId) ?? [] : [];
  const selectedGroup = selected ? groups.find((group) => group.items.some((item) => item.conversation.conversationId === selected.conversationId)) : null;

  const clearSelection = () => {
    setSelectionMode(false);
    setSelectedIds(new Set());
  };

  const changeQuery = (value: string) => {
    setQuery(value);
    setSelectedIds(new Set());
  };

  const changeProjectFilter = (value: string) => {
    setProjectFilter(value);
    setSelectedIds(new Set());
  };

  const toggleSelected = (conversationId: string) => setSelectedIds((current) => {
    const next = new Set(current);
    if (next.has(conversationId)) next.delete(conversationId); else next.add(conversationId);
    return next;
  });

  const toggleAllFiltered = () => setSelectedIds((current) => {
    const next = new Set(current);
    if (allFilteredSelected) filteredIds.forEach((id) => next.delete(id));
    else filteredIds.forEach((id) => next.add(id));
    return next;
  });

  const restore = async (conversation: AiConversation) => {
    if (restoring.current || deleting) return;
    restoring.current = true;
    setRestoringId(conversation.conversationId);
    try {
      const result = await setConversationsArchived([conversation.conversationId], false);
      if (result.failures.length) throw new Error(result.failures[0].reason);
      if (groups.find((group) => group.key === projectFilter)?.items.every((item) => item.conversation.conversationId === conversation.conversationId)) setProjectFilter("all");
      if (selectedId === conversation.conversationId) setSelectedId(null);
      props.onMessage("已取消归档，可在 AI 工作台的会话列表中继续对话。");
    } catch (reason) {
      props.onMessage(`取消归档失败：${aiErrorMessage(reason)}`, "error");
    } finally {
      restoring.current = false;
      setRestoringId(null);
    }
  };

  const requestDelete = (ids: string[], title: string | null = null) => {
    if (ids.length > 0) setPendingDelete({ ids, title });
  };

  const confirmDelete = async () => {
    if (!pendingDelete || deleting) return;
    setDeleting(true);
    const deletingIds = new Set(pendingDelete.ids);
    try {
      const result = await deleteAiConversations(pendingDelete.ids);
      setSnapshot((current) => pruneDeletedConversations(current, deletingIds));
      if (selectedId && deletingIds.has(selectedId)) setSelectedId(null);
      if (projectFilter !== "all") {
        const projectStillAvailable = snapshot.conversations.some((conversation) => conversation.archived
          && !deletingIds.has(conversation.conversationId)
          && groupKey(conversation) === projectFilter);
        if (!projectStillAvailable) setProjectFilter("all");
      }
      clearSelection();
      setPendingDelete(null);
      if (result.cleanupWarning) {
        props.onMessage(`已删除 ${result.deletedCount} 条会话，但部分本机文件未能清理，请检查应用数据目录权限。`, "error");
      } else {
        props.onMessage(`已彻底删除 ${result.deletedCount} 条会话。`);
      }
    } catch (reason) {
      props.onMessage(`删除失败：${aiErrorMessage(reason)}`, "error");
      setRetry((value) => value + 1);
    } finally {
      setDeleting(false);
    }
  };

  const restoreButton = (conversation: AiConversation) => <button
    aria-label={`取消归档：${conversationTopic(conversation, messagesByConversation.get(conversation.conversationId) ?? [])}`}
    className="archive-restore" disabled={restoringId !== null || deleting}
    onClick={() => void restore(conversation)} type="button">
    {restoringId === conversation.conversationId ? "正在恢复…" : "取消归档"}
  </button>;

  const deleteButton = (conversation: AiConversation, title: string) => <button
    aria-label={`彻底删除：${title}`} className="archive-delete"
    disabled={restoringId !== null || deleting}
    onClick={() => requestDelete([conversation.conversationId], title)} type="button">
    <ArchiveTrashIcon/><span>彻底删除</span>
  </button>;

  return <>
    <section aria-label="已归档会话" className="archive-page">
      {selectedId && !selected && !loading && !error ? <p role="alert">该对话已不在归档中。</p> : null}
      {selected ? <>
        <header className="archive-detail-header">
          <button className="archive-back" onClick={() => setSelectedId(null)} type="button"><BackIcon/>返回归档清单</button>
          <div className="archive-detail-title"><div>
            <h1 ref={heading} tabIndex={-1}>{conversationTopic(selected, selectedMessages)}</h1>
            <p>{selectedGroup?.name} · 已归档 · 更新于 {auditMoment(selected.updatedAt)}</p>
          </div><div className="archive-row-actions">{restoreButton(selected)}{deleteButton(selected, conversationTopic(selected, selectedMessages))}</div></div>
          <p className="archive-readonly"><ArchiveIcon/>当前为只读查看，取消归档后可继续对话。</p>
        </header>
        <div className="archive-messages" aria-label="归档会话消息">
          {selectedMessages.length ? selectedMessages.map((message) => <article className="archive-message" data-role={message.role} key={message.messageId}>
            <header><strong>{message.role === "user" ? "你" : "助手"}</strong><time dateTime={new Date(message.createdAt).toISOString()}>{auditMoment(message.createdAt)}</time></header>
            {message.attachments?.length ? <MessageAttachments attachments={message.attachments}/> : null}
            {message.role === "assistant" ? <MarkdownContent diagramIdPrefix="archive">{assistantMessageBody(message.content)}</MarkdownContent> : <p className="archive-user-text">{message.content}</p>}
          </article>) : <div className="archive-empty"><p>此会话尚无消息</p></div>}
        </div>
      </> : <>
        <header className="archive-header"><div><h1 ref={heading} tabIndex={-1}>已归档会话</h1>{!loading && !error ? <span>{count} 条会话</span> : null}</div>
          {!loading && !error && count > 0 ? selectionMode
            ? <button className="archive-finish-select" onClick={clearSelection} type="button">完成</button>
            : <button className="archive-start-select" onClick={() => setSelectionMode(true)} type="button">批量选择</button> : null}
        </header>
        <div className="archive-filters">
          <SearchField clearable ariaLabel="搜索已归档会话" placeholder="搜索已归档会话" value={query} onChange={changeQuery}/>
          <Select ariaLabel="按项目筛选归档会话" value={projectFilter} onChange={changeProjectFilter}
            options={[{ value: "all", label: "所有项目" }, ...groups.map((group) => ({ value: group.key, label: group.name }))]}/>
        </div>
        {selectionMode ? <div aria-label="批量选择操作" className="archive-selection-bar" role="toolbar">
          <button disabled={filteredIds.length === 0 || deleting} onClick={toggleAllFiltered} type="button">{allFilteredSelected ? "取消全选" : "全选"}</button>
          <span aria-live="polite">已选 {selectedIds.size} 条</span>
          <button className="archive-batch-delete" disabled={selectedIds.size === 0 || deleting} onClick={() => requestDelete([...selectedIds])} type="button"><ArchiveTrashIcon/>批量删除</button>
        </div> : null}
        <div className="archive-list" aria-busy={loading}>
          {loading ? <div className="archive-empty" role="status"><p>正在读取已归档会话…</p></div>
            : error ? <div className="archive-empty" role="alert"><h2>无法读取已归档会话</h2><p>{error}</p><button onClick={() => setRetry((value) => value + 1)} type="button">重新加载</button></div>
            : count === 0 ? <div className="archive-empty"><ArchiveIcon/><h2>暂无已归档会话</h2><p>归档后的会话会保留在这里，随时可以查看和恢复。</p></div>
            : filteredCount === 0 ? <div className="archive-empty"><h2>未找到匹配的会话</h2><p>试试其他关键词或项目。</p></div>
            : filteredGroups.map((group) => <section aria-label={`${group.name}的归档会话`} className="archive-group" key={group.key}>
              <h2><button aria-expanded={!collapsed.has(group.key)} className="archive-group-toggle" onClick={() => setCollapsed((current) => {
                const next = new Set(current);
                if (next.has(group.key)) next.delete(group.key); else next.add(group.key);
                return next;
              })} type="button"><TreeFolderIcon/><span>{group.name}</span><small>{group.items.length} 条会话</small><ChevronIcon collapsed={collapsed.has(group.key)}/></button></h2>
              {!collapsed.has(group.key) ? <ul className="archive-rows">{group.items.map(({ conversation, title }) => <li className="archive-row" data-selected={selectedIds.has(conversation.conversationId)} key={conversation.conversationId}>
                {selectionMode ? <input aria-label={`选择会话：${title}`} checked={selectedIds.has(conversation.conversationId)} className="archive-checkbox" onChange={() => toggleSelected(conversation.conversationId)} type="checkbox"/> : null}
                <button aria-pressed={selectionMode ? selectedIds.has(conversation.conversationId) : undefined} className="archive-open" onClick={() => selectionMode ? toggleSelected(conversation.conversationId) : setSelectedId(conversation.conversationId)} type="button" title={title}>
                  <strong>{title}</strong><span>更新于 <time dateTime={new Date(conversation.updatedAt).toISOString()}>{auditMoment(conversation.updatedAt)}</time></span>
                </button>{!selectionMode ? <div className="archive-row-actions">{restoreButton(conversation)}{deleteButton(conversation, title)}</div> : null}
              </li>)}</ul> : null}
            </section>)}
        </div>
      </>}
    </section>
    {pendingDelete ? <ConfirmDialog
      busy={deleting}
      confirmLabel="彻底删除"
      danger
      description={<><p>{pendingDelete.ids.length === 1 && pendingDelete.title ? `会话“${pendingDelete.title}”` : `选中的 ${pendingDelete.ids.length} 条会话`}及其消息、运行记录和已保存附件将从本机删除。</p><p><strong>该操作不可恢复。</strong>项目、任务、交付物和项目文件不会被删除。</p></>}
      onCancel={() => setPendingDelete(null)}
      onConfirm={() => void confirmDelete()}
      title={pendingDelete.ids.length === 1 ? "彻底删除这个会话？" : `彻底删除选中的 ${pendingDelete.ids.length} 条会话？`}/>: null}
  </>;
}

function groupKey(conversation: AiConversation): string {
  return conversation.context.projectId ? `project:${conversation.context.projectId}` : "unassigned";
}

function pruneDeletedConversations(snapshot: AiRuntimeSnapshot, deletedIds: Set<string>): AiRuntimeSnapshot {
  const deletedRunIds = new Set(snapshot.runs.filter((run) => deletedIds.has(run.conversationId)).map((run) => run.runId));
  return {
    ...snapshot,
    conversations: snapshot.conversations.filter((conversation) => !deletedIds.has(conversation.conversationId)),
    messages: snapshot.messages.filter((message) => !deletedIds.has(message.conversationId)),
    runs: snapshot.runs.filter((run) => !deletedIds.has(run.conversationId)),
    permissionRequests: snapshot.permissionRequests.filter((request) => !deletedRunIds.has(request.runId)),
  };
}
