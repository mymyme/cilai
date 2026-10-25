import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { aiErrorMessage, aiRuntimeChangedEvent, emptyAiSnapshot, loadAiActivitySnapshot, loadAiSnapshot, type AiRuntimeSnapshot } from "./ai-runtime";
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

export function useAiRuntimeSnapshot({ workspaceId, runtimeRevision, onMessage, setSelectedConversationId }: {
  workspaceId?: string;
  runtimeRevision: number;
  onMessage: MessageSink;
  setSelectedConversationId: Dispatch<SetStateAction<string | null>>;
}) {
  const workspaceIdRef = useRef(workspaceId);
  workspaceIdRef.current = workspaceId;
  const observedRuntimeRevision = useRef(runtimeRevision);
  const [snapshot, setSnapshot] = useState<AiRuntimeSnapshot>(emptyAiSnapshot);
  const [snapshotLoaded, setSnapshotLoaded] = useState(false);
  const [snapshotError, setSnapshotError] = useState<string | null>(null);
  const snapshotRequest = useRef(0);
  const snapshotInFlight = useRef(false);
  // null 表示没有后续刷新；true 的目录核对需求优先于仅轮询本机状态的 false。
  const snapshotRefreshQueued = useRef<boolean | null>(null);
  const snapshotRefreshWaiters = useRef<Array<(value: AiRuntimeSnapshot | null) => void>>([]);
  const latestArchiveRevision = useRef(0);
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
    const request = ++snapshotRequest.current;
    let delivered: AiRuntimeSnapshot | null = null;
    try {
      const result = await loadAiSnapshot(refreshNativeCatalog);
      if (request !== snapshotRequest.current || requestedWorkspaceId !== workspaceIdRef.current) return null;
      if (result.stateRevision < latestArchiveRevision.current) {
        snapshotRefreshQueued.current = true;
        return null;
      }
      delivered = result;
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
      if (request === snapshotRequest.current && requestedWorkspaceId === workspaceIdRef.current) setSnapshotError(aiErrorMessage(error));
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
    let reportedFailure = false;
    const poll = async () => {
      try {
        const activities = await Promise.all(Array.from(
          { length: Math.ceil(runIds.length / 32) },
          (_, index) => loadAiActivitySnapshot(runIds.slice(index * 32, (index + 1) * 32)),
        ));
        if (cancelled || activities.some((activity) => activity.stateRevision < latestArchiveRevision.current)) return;
        const missing = new Set(activities.flatMap((activity) => activity.missingRunIds));
        const conversations = activities.flatMap((activity) => activity.conversations);
        const messages = activities.flatMap((activity) => activity.messages);
        const runs = activities.flatMap((activity) => activity.runs);
        const permissionRequests = activities.flatMap((activity) => activity.permissionRequests);
        setSnapshot((current) => ({
          ...current,
          stateRevision: Math.max(current.stateRevision, ...activities.map((activity) => activity.stateRevision)),
          conversations: mergeSnapshotEntities(current.conversations, conversations, (item) => item.conversationId),
          messages: mergeSnapshotEntities(current.messages, messages, (item) => item.messageId),
          runs: mergeSnapshotEntities(current.runs, runs, (item) => item.runId, missing),
          permissionRequests: mergeSnapshotEntities(current.permissionRequests, permissionRequests, (item) => item.permissionRequestId),
        }));
        if (missing.size || runs.some((run) => !activeRunStatuses.includes(run.status))) void refresh();
      } catch (error) {
        if (!cancelled && !reportedFailure) {
          reportedFailure = true;
          onMessage(aiErrorMessage(error), "error");
        }
      } finally {
        if (!cancelled) timer = window.setTimeout(() => void poll(), activeRunRefreshInterval);
      }
    };
    timer = window.setTimeout(() => void poll(), activeRunRefreshInterval);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [activeRunIdsKey, workspaceId]);

  return { snapshot, setSnapshot, snapshotLoaded, snapshotError, refresh };
}
