import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { aiErrorMessage, aiRuntimeChangedEvent, emptyAiSnapshot, loadAiActivitySnapshot, loadAiApprovalSnapshot, loadAiHistoryPage, loadAiSnapshot, type AiHistoryCursor, type AiRunEvent, type AiRunEventCursor, type AiRuntimeActivitySnapshot, type AiRuntimeSnapshot } from "./ai-runtime";
import type { MessageSink } from "./app-message";
import { conversationArchiveChanged, mergeArchivedConversations, type ConversationArchiveChange } from "./conversation-archive";
export const lastViewedConversationStorageKey = "cilai.workbench.ai-last-viewed-conversation";
export const activeRunStatuses = ["queued", "running", "paused", "terminating", "waiting_permission"];
const activeRunRefreshInterval = 400;

export function readLastViewedConversationId(): string | null {
  const value = window.localStorage.getItem(lastViewedConversationStorageKey)?.trim();
  return value || null;
}

function mergeSnapshotEntities<T>(current: T[], updates: T[], key: (item: T) => string, missing = new Set<string>()): T[] {
  const byId = new Map(updates.map((item) => [key(item), item]));
  const kept = current.filter((item) => !missing.has(key(item))).map((item) => byId.get(key(item)) ?? item);
  const existing = new Set(kept.map(key));
  return [...kept, ...[...byId].filter(([id]) => !existing.has(id)).map(([, item]) => item)];
}

type EventFrame = { cursor: AiRunEventCursor; events: AiRunEvent[] };

function applyRunEventPages(activities: AiRuntimeActivitySnapshot[], previous: Map<string, EventFrame>) {
  const frames = new Map<string, EventFrame>();
  const runs = activities.flatMap(activity => activity.runs.map(run => {
    const page = activity.eventPages[run.runId];
    const base = previous.get(run.runId);
    if (!page || !Number.isSafeInteger(page.retained) || page.retained < 0
        || page.retained > 0 && base?.events.length !== page.retained
        || page.cursor.count !== page.retained + run.events.length
        || !/^[a-f0-9]{64}$/.test(page.cursor.digest)) throw new Error("ai_activity_events_invalid");
    const events = page.retained > 0 ? [...base!.events, ...run.events] : run.events;
    frames.set(run.runId, { events, cursor: page.cursor });
    return { ...run, events };
  }));
  return { frames, runs };
}

type EarlierMessages = {
  cursor: AiHistoryCursor;
  next: AiHistoryCursor | null;
  messages: AiRuntimeSnapshot["messages"];
  runs: AiRuntimeSnapshot["runs"];
  permissionRequests: AiRuntimeSnapshot["permissionRequests"];
};

export function useAiRuntimeSnapshot({ workspaceId, runtimeRevision, onMessage, selectedConversationId, setSelectedConversationId }: {
  workspaceId?: string;
  runtimeRevision: number;
  selectedConversationId: string | null;
  onMessage: MessageSink;
  setSelectedConversationId: Dispatch<SetStateAction<string | null>>;
}) {
  const workspaceIdRef = useRef(workspaceId);
  workspaceIdRef.current = workspaceId;
  const selectedRef = useRef(selectedConversationId);
  selectedRef.current = selectedConversationId;
  const observedSelection = useRef(selectedConversationId);
  const earlier = useRef<EarlierMessages | null>(null);
  const latestSnapshot = useRef<AiRuntimeSnapshot>(emptyAiSnapshot);
  const olderRequest = useRef(0);
  const [olderLoading, setOlderLoading] = useState(false);
  const [olderError, setOlderError] = useState<string | null>(null);
  const observedRuntimeRevision = useRef(runtimeRevision);
  const [snapshot, setSnapshot] = useState<AiRuntimeSnapshot>(emptyAiSnapshot);
  latestSnapshot.current = snapshot;
  const [snapshotLoaded, setSnapshotLoaded] = useState(false);
  const [snapshotError, setSnapshotError] = useState<string | null>(null);
  const snapshotRequest = useRef(0);
  const snapshotInFlight = useRef(false);
  // null 表示没有后续刷新；true 的目录核对需求优先于仅轮询本机状态的 false。
  const snapshotRefreshQueued = useRef<boolean | null>(null);
  const snapshotRefreshWaiters = useRef<Array<(value: AiRuntimeSnapshot | null) => void>>([]);
  const latestArchiveRevision = useRef(0);
  const runtimeSession = useRef<string | undefined>(undefined);
  const snapshotFrame = useRef(0);
  useEffect(() => {
    const changed = (event: Event) => {
      const result = (event as CustomEvent<ConversationArchiveChange>).detail;
      latestArchiveRevision.current = Math.max(latestArchiveRevision.current, result.stateRevision);
      setSnapshot((current) => mergeArchivedConversations(current, result));
    };
    window.addEventListener(conversationArchiveChanged, changed);
    return () => window.removeEventListener(conversationArchiveChanged, changed);
  }, []);
  const refresh = async (refreshNativeCatalog = true): Promise<AiRuntimeSnapshot | null> => {
    /*
     * 已有刷新在途时不另发一次，改为排到它后面，并把本次调用挂在等待队列上。
     * 从前这里直接返回，调用方的 await 立刻兑现却什么也没拿到——发送完成后据此
     * 判断"消息是否已被服务端记下"，判断的其实是刷新前的旧快照。
     */
    if (snapshotInFlight.current) {
      snapshotRefreshQueued.current = snapshotRefreshQueued.current === true || refreshNativeCatalog;
      return new Promise<AiRuntimeSnapshot | null>((resolve) => { snapshotRefreshWaiters.current.push(resolve); });
    }
    snapshotInFlight.current = true;
    const requestedWorkspaceId = workspaceIdRef.current;
    const selected = selectedRef.current;
    const request = ++snapshotRequest.current;
    let delivered: AiRuntimeSnapshot | null = null;
    try {
      let result = await loadAiSnapshot(refreshNativeCatalog, selected);
      if (request !== snapshotRequest.current || requestedWorkspaceId !== workspaceIdRef.current || selected !== selectedRef.current) return null;
      if (runtimeSession.current && result.runtimeSessionId && result.runtimeSessionId !== runtimeSession.current) {
        latestArchiveRevision.current = 0;
        earlier.current = null;
      }
      if (result.stateRevision < latestArchiveRevision.current) {
        setSnapshotError("对话状态已变化，请重试。");
        return null;
      }
      runtimeSession.current = result.runtimeSessionId;
      const history = result.historyWindow;
      const cached = earlier.current;
      if (cached && history?.sessionId === cached.cursor.sessionId && history.conversationId === cached.cursor.conversationId
          && history.previousCursor?.beforeMessageId === cached.cursor.beforeMessageId) {
        result = { ...result,
          messages: mergeSnapshotEntities(cached.messages, result.messages, item => item.messageId),
          runs: mergeSnapshotEntities(cached.runs, result.runs, item => item.runId),
          permissionRequests: mergeSnapshotEntities(cached.permissionRequests, result.permissionRequests, item => item.permissionRequestId),
          historyWindow: { ...history, previousCursor: cached.next },
        };
      } else if (cached && history?.previousCursor && history.sessionId === cached.cursor.sessionId
          && history.conversationId === cached.cursor.conversationId) {
        const previous = latestSnapshot.current;
        const boundary = previous.messages.findIndex(item => item.messageId === history.previousCursor!.beforeMessageId);
        if (boundary > 0) {
          const messages = previous.messages.slice(0, boundary);
          const messageIds = new Set(messages.map(item => item.messageId));
          const runIds = new Set(messages.map(item => item.runId));
          const runs = previous.runs.filter(item => runIds.has(item.runId) || messageIds.has(item.inputMessageId));
          const relatedRuns = new Set(runs.map(item => item.runId));
          const permissionRequests = previous.permissionRequests.filter(item => relatedRuns.has(item.runId));
          earlier.current = { ...cached, cursor: history.previousCursor, messages, runs, permissionRequests };
          result = { ...result,
            messages: mergeSnapshotEntities(messages, result.messages, item => item.messageId),
            runs: mergeSnapshotEntities(runs, result.runs, item => item.runId),
            permissionRequests: mergeSnapshotEntities(permissionRequests, result.permissionRequests, item => item.permissionRequestId),
            historyWindow: { ...history, previousCursor: cached.next },
          };
        } else {
          earlier.current = null;
        }
      } else if (cached) {
        earlier.current = null;
      }
      delivered = result;
      snapshotFrame.current += 1;
      setSnapshot(result);
      setSnapshotLoaded(true);
      setSnapshotError(null);
      setSelectedConversationId((current) => {
        const currentConversation = result.conversations.find((item) => item.conversationId === current);
        if (currentConversation && !currentConversation.archived) return current;
        const remembered = readLastViewedConversationId();
        const rememberedConversation = result.conversations.find((item) => item.conversationId === remembered);
        if (rememberedConversation && !rememberedConversation.archived) return remembered;
        window.localStorage.removeItem(lastViewedConversationStorageKey);
        return null;
      });
    } catch (error) {
      if (request === snapshotRequest.current && requestedWorkspaceId === workspaceIdRef.current && selected === selectedRef.current) setSnapshotError(aiErrorMessage(error));
    } finally {
      snapshotInFlight.current = false;
      if (snapshotRefreshQueued.current !== null) {
        const queuedNativeCatalog = snapshotRefreshQueued.current;
        snapshotRefreshQueued.current = null;
        void refresh(queuedNativeCatalog);
      } else {
        const waiters = snapshotRefreshWaiters.current;
        snapshotRefreshWaiters.current = [];
        for (const resolve of waiters) resolve(delivered);
      }
    }
    return delivered;
  };

  const loadOlder = async (): Promise<boolean> => {
    const window = snapshot.historyWindow;
    const cursor = window?.previousCursor;
    if (olderLoading || !window || !cursor || window.conversationId !== selectedRef.current) return false;
    const request = ++olderRequest.current;
    const generation = snapshotFrame.current;
    const isCurrent = () => request === olderRequest.current && selectedRef.current === cursor.conversationId
      && snapshotFrame.current === generation;
    setOlderLoading(true);
    setOlderError(null);
    try {
      const page = await loadAiHistoryPage(cursor.conversationId, false, cursor);
      if (!isCurrent()) return false;
      const cached = earlier.current;
      earlier.current = {
        cursor: cached?.cursor ?? cursor,
        next: page.previousCursor,
        messages: mergeSnapshotEntities(page.messages, cached?.messages ?? [], item => item.messageId),
        runs: mergeSnapshotEntities(page.runs, cached?.runs ?? [], item => item.runId),
        permissionRequests: mergeSnapshotEntities(page.permissionRequests, cached?.permissionRequests ?? [], item => item.permissionRequestId),
      };
      setSnapshot(current => ({
        ...current,
        messages: mergeSnapshotEntities(page.messages, current.messages, item => item.messageId),
        runs: mergeSnapshotEntities(page.runs, current.runs, item => item.runId),
        permissionRequests: mergeSnapshotEntities(page.permissionRequests, current.permissionRequests, item => item.permissionRequestId),
        historyWindow: { ...window, previousCursor: page.previousCursor },
      }));
      return true;
    } catch (error) {
      if (isCurrent()) setOlderError(aiErrorMessage(error));
      return false;
    } finally {
      if (request === olderRequest.current) setOlderLoading(false);
    }
  };

  useEffect(() => {
    if (observedSelection.current === selectedConversationId) return;
    observedSelection.current = selectedConversationId;
    earlier.current = null;
    olderRequest.current += 1;
    setOlderError(null);
    setOlderLoading(false);
    void refresh(false);
  }, [selectedConversationId]);

  useEffect(() => {
    if (observedRuntimeRevision.current === runtimeRevision) return;
    observedRuntimeRevision.current = runtimeRevision;
    void refresh();
  }, [runtimeRevision]);

  useEffect(() => {
    void refresh();
    const changed = () => { void refresh(); };
    window.addEventListener(aiRuntimeChangedEvent, changed);
    return () => { snapshotRequest.current += 1; window.removeEventListener(aiRuntimeChangedEvent, changed); };
  }, [workspaceId]);
  const activeRunIdsKey = snapshot.runs.filter((run) => run.threadWriterActive || activeRunStatuses.includes(run.status)).map((run) => run.runId).join("|");
  useEffect(() => {
    if (!activeRunIdsKey) return;
    const runIds = activeRunIdsKey.split("|");
    let cancelled = false;
    let timer: number;
    const batches = Array.from(
      { length: Math.ceil(runIds.length / 32) },
      (_, index) => runIds.slice(index * 32, (index + 1) * 32),
    );
    let reportedFailure = false;
    let observedFrame = -1;
    let eventFrames = new Map<string, EventFrame>();
    const report = (error: unknown) => {
      if (!cancelled && !reportedFailure) {
        reportedFailure = true;
        onMessage(aiErrorMessage(error), "error");
      }
    };
    const poll = async () => {
      const generation = snapshotFrame.current;
      if (observedFrame !== generation) { eventFrames.clear(); observedFrame = generation; }
      const current = () => !cancelled && snapshotFrame.current === generation;
      const activity = Promise.all(batches.map(ids => loadAiActivitySnapshot(ids, Object.fromEntries(ids.flatMap(id => {
        const entry = eventFrames.get(id);
        return entry ? [[id, entry.cursor]] : [];
      }))))).then((activities) => {
        if (!current() || activities.some(item => item.stateRevision < latestArchiveRevision.current)) return;
        const missing = new Set(activities.flatMap(item => item.missingRunIds));
        const conversations = activities.flatMap(item => item.conversations);
        const messages = activities.flatMap(item => item.messages);
        const { frames, runs } = applyRunEventPages(activities, eventFrames);
        eventFrames = frames;
        setSnapshot(value => ({
          ...value,
          stateRevision: Math.max(value.stateRevision, ...activities.map(item => item.stateRevision)),
          conversations: mergeSnapshotEntities(value.conversations, conversations, item => item.conversationId),
          messages: mergeSnapshotEntities(value.messages,
            messages.filter(item => item.conversationId === selectedRef.current
              && (!value.historyWindow || value.historyWindow.conversationId === item.conversationId)),
            item => item.messageId),
          runs: mergeSnapshotEntities(value.runs, runs, item => item.runId, missing),
        }));
        if (missing.size || runs.some(run => !run.threadWriterActive && !activeRunStatuses.includes(run.status))) void refresh(false);
      }).catch(error => { eventFrames.clear(); if (current()) report(error); });
      const controls = Promise.all(batches.map(loadAiApprovalSnapshot)).then((approvals) => {
        if (!current() || approvals.some(item => item.stateRevision < latestArchiveRevision.current)) return;
        const requested = new Set(runIds);
        const permissionRequests = approvals.flatMap(item => item.permissionRequests);
        setSnapshot(value => ({
          ...value,
          stateRevision: Math.max(value.stateRevision, ...approvals.map(item => item.stateRevision)),
          permissionRequests: [...value.permissionRequests.filter(item => !requested.has(item.runId)), ...permissionRequests],
        }));
      }).catch(error => { if (current()) report(error); });
      try { await Promise.all([activity, controls]); }
      finally { if (!cancelled) timer = window.setTimeout(() => void poll(), activeRunRefreshInterval); }
    };
    timer = window.setTimeout(() => void poll(), activeRunRefreshInterval);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [activeRunIdsKey, workspaceId]);

  return { snapshot, setSnapshot, snapshotLoaded, snapshotError, refresh, loadOlder, olderLoading, olderError };
}
