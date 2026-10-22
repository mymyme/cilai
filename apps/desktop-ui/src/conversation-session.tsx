import { createContext, useCallback, useContext, useEffect, useRef, useState, useSyncExternalStore, type ReactNode, type SetStateAction } from "react";
import { aiErrorMessage, aiRuntimeChangedEvent } from "./ai-runtime";
import type { MessageSink } from "./app-message";
import { loadConversationSession, patchConversationSession, type ConversationSessionSnapshot } from "./conversation-session-api";
import type { DraftAttachment, QueuedFollowUp } from "./ai-workbench";

const persistedKeys = new Set(["applicationHandoffs", "drafts", "queuedFollowUps", "selectedConversationId", "starterAgentId", "starterBusinessId", "starterFile", "starterModelOverride", "starterReasoningEffort", "starterPermissionMode", "applicationContextSelections", "skillSelections", "draftAttachmentGroups", "draftMaterialGroups", "pendingCreate", "initializedSelection", "pendingOutgoing", "recoveredOutgoing"]);
function serializable(value: unknown): unknown {
  return JSON.parse(JSON.stringify(value ?? null, (key, item) => key === "previewUrl" || key === "localPath" ? "" : item));
}
const same = (a: unknown, b: unknown) => JSON.stringify(serializable(a)) === JSON.stringify(serializable(b));

class ConversationSession {
  private values = new Map<string, unknown>();
  private dirtyKeys = new Set<string>();
  private listeners = new Map<string, Set<() => void>>();
  private baseline: ConversationSessionSnapshot;
  private pending: Promise<void> | null = null;
  private timer?: ReturnType<typeof setTimeout>;
  private disposed = false;
  private failed = false;
  private refreshing = false;
  sessionId() { return this.baseline.sessionId; }
  constructor(snapshot: ConversationSessionSnapshot, private report: (error: string | null) => void) {
    this.baseline = snapshot;
    Object.entries(snapshot.values).forEach(([key, value]) => this.values.set(key, value));
  }
  read<T>(key: string, initial: T | (() => T)): T {
    if (!this.values.has(key)) this.values.set(key, typeof initial === "function" ? (initial as () => T)() : initial);
    return this.values.get(key) as T;
  }
  write<T>(key: string, initial: T | (() => T), value: SetStateAction<T>) {
    const current = this.read(key, initial);
    const next = typeof value === "function" ? (value as (current: T) => T)(current) : value;
    if (Object.is(current, next)) return;
    this.values.set(key, next);
    this.dirtyKeys.add(key);
    this.emit(key);
    if (persistedKeys.has(key) && !this.failed && !this.disposed) {
      clearTimeout(this.timer);
      this.timer = setTimeout(() => { void this.flush().catch(() => {}); }, 120);
    }
  }
  private emit(key: string) { this.listeners.get(key)?.forEach((listener) => listener()); }
  subscribe(key: string, listener: () => void) {
    const listeners = this.listeners.get(key) ?? new Set();
    this.listeners.set(key, listeners);
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  }
  private changes() {
    return [...persistedKeys].filter((key) => this.dirtyKeys.has(key) && !same(this.values.get(key), this.baseline.values[key]))
      .map((key) => ({ key, previous: this.baseline.values[key] ?? null, value: serializable(this.values.get(key)) }));
  }
  private accept(snapshot: ConversationSessionSnapshot, sent: Map<string, unknown> = new Map()) {
    if (this.disposed || snapshot.sessionId !== this.baseline.sessionId || snapshot.revision < this.baseline.revision) return;
    let queueChanged = false;
    for (const key of persistedKeys) {
      const current = this.values.get(key);
      if (!this.dirtyKeys.has(key) || same(current, sent.has(key) ? sent.get(key) : this.baseline.values[key])) {
        this.dirtyKeys.delete(key);
        const remote = snapshot.values[key];
        if (remote !== undefined && !same(current, remote)) {
          this.values.set(key, remote);
          this.emit(key);
          queueChanged ||= key === "queuedFollowUps";
        }
      }
    }
    this.baseline = snapshot;
    if (queueChanged) {
      this.write("runtimeRevision", 0, (value: number) => value + 1);
      window.dispatchEvent(new Event(aiRuntimeChangedEvent));
    }
  }
  async refresh() {
    if (this.refreshing || this.pending || this.failed || this.disposed || this.changes().length) return;
    this.refreshing = true;
    try { this.accept(await loadConversationSession()); } finally { this.refreshing = false; }
  }
  async flush(): Promise<void> {
    clearTimeout(this.timer);
    if (this.pending) { await this.pending; return this.flush(); }
    if (this.disposed) throw new Error("输入会话已关闭。");
    if (this.failed) throw new Error("草稿尚未保存，请先重试保存。");
    const changes = this.changes();
    if (!changes.length) return;
    this.pending = (async () => {
      try {
        const snapshot = await patchConversationSession(this.baseline.sessionId, changes);
        this.accept(snapshot, new Map(changes.map((change) => [change.key, change.value])));
        this.report(null);
      } catch (error) {
        this.failed = true;
        if (String(error).includes("conversation_session_conflict") || String(error).includes("conversation_queue_busy")) {
          try {
            const snapshot = await loadConversationSession();
            if (!this.disposed && snapshot.sessionId === this.baseline.sessionId) {
              const queue = snapshot.values.queuedFollowUps ?? [];
              this.values.set("queuedFollowUps", queue);
              this.dirtyKeys.delete("queuedFollowUps");
              this.emit("queuedFollowUps");
              if (this.dirtyKeys.has("applicationHandoffs")) {
                const id = (value: unknown) => { const entry = value as {handoffId?:string;requestId?:string}; return `${entry.handoffId}:${entry.requestId ?? ""}`; };
                const local = this.values.get("applicationHandoffs") as unknown[] ?? [];
                const previous = this.baseline.values.applicationHandoffs as unknown[] ?? [];
                const removed = new Set(previous.filter((value) => !local.some((entry) => id(entry) === id(value))).map(id));
                const remote = snapshot.values.applicationHandoffs as unknown[] ?? [];
                this.values.set("applicationHandoffs", remote.filter((value) => !removed.has(id(value))));
                this.emit("applicationHandoffs");
              }
              this.baseline = snapshot;
            }
          } catch (lookupError) { this.report(`队列状态无法核实：${aiErrorMessage(lookupError)}`); }
        }
        this.report(`草稿未保存：${aiErrorMessage(error)}`);
        throw error;
      } finally { this.pending = null; }
    })();
    await this.pending;
    if (this.changes().length) await this.flush();
  }
  async retry() { this.failed = false; await this.flush(); this.report(null); }
  hasUnsaved() { return this.failed || Boolean(this.pending) || this.changes().length > 0; }
  async close() {
    clearTimeout(this.timer);
    try { await this.flush(); } catch {}
    this.disposed = true;
    const drafts = this.read<Record<string, DraftAttachment[]>>("draftAttachmentGroups", {});
    const queue = this.read<QueuedFollowUp[]>("queuedFollowUps", []);
    new Set([...Object.values(drafts).flat(), ...queue.flatMap((item) => item.attachments)].map((item) => item.previewUrl)).forEach((url) => { if (url) URL.revokeObjectURL(url); });
  }
}

const sessions = new Set<ConversationSession>();
export async function flushConversationSessions() { await Promise.all([...sessions].map((session) => session.flush())); }

const SessionContext = createContext<ConversationSession | null>(null);
function useSession() {
  const session = useContext(SessionContext);
  if (!session) throw new Error("conversation_session_provider_required");
  return session;
}
export function useConversationSessionState<T>(key: string, initial: T | (() => T)): [T, (value: SetStateAction<T>) => void] {
  const session = useSession();
  const initialRef = useRef(initial);
  const subscribe = useCallback((listener: () => void) => session.subscribe(key, listener), [session, key]);
  const read = useCallback(() => session.read(key, initialRef.current), [session, key]);
  const write = useCallback((value: SetStateAction<T>) => session.write(key, initialRef.current, value), [session, key]);
  return [useSyncExternalStore(subscribe, read), write];
}
export function useConversationSessionFlush() {
  const session = useSession();
  return useCallback(() => session.retry(), [session]);
}
export function useConversationSessionId() { return useSession().sessionId(); }
export function useConversationSessionRefresh() {
  const session = useSession();
  return useCallback(() => session.refresh(), [session]);
}

export function ConversationSessionProvider(props: { hostPlatform: string | null; onMessage: MessageSink; dispatchEnabled?: boolean; children: ReactNode }) {
  const [session, setSession] = useState<ConversationSession | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let disposed = false;
    let loaded: ConversationSession | undefined;
    void loadConversationSession().then((snapshot) => {
      if (disposed) return;
      loaded = new ConversationSession(snapshot, setError);
      sessions.add(loaded);
      setError(null);
      setSession(loaded);
    }).catch((reason) => { if (!disposed) setError(`无法恢复草稿：${aiErrorMessage(reason)}`); });
    return () => { disposed = true; if (loaded) { sessions.delete(loaded); void loaded.close(); } };
  }, [props.hostPlatform, attempt]);
  useEffect(() => {
    if (!session || props.dispatchEnabled === false) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try { await session.refresh(); } catch (reason) { if (!disposed) setError(`无法同步队列：${aiErrorMessage(reason)}`); }
      if (!disposed) timer = setTimeout(() => void poll(), 1_500);
    };
    const refresh = () => { clearTimeout(timer); void poll(); };
    const leaving = (event: BeforeUnloadEvent) => { if (session.hasUnsaved()) { event.preventDefault(); event.returnValue = ""; void session.flush().catch(() => {}); } };
    timer = setTimeout(() => void poll(), 1_500);
    window.addEventListener("beforeunload", leaving);
    window.addEventListener("focus", refresh);
    window.addEventListener(aiRuntimeChangedEvent, refresh);
    return () => { disposed = true; clearTimeout(timer); window.removeEventListener("beforeunload", leaving); window.removeEventListener("focus", refresh); window.removeEventListener(aiRuntimeChangedEvent, refresh); };
  }, [session, props.dispatchEnabled]);
  return <>
    {error ? <div role="alert" className="conversation-session-error"><span>{error}</span><button type="button" onClick={() => { if (session) void session.retry().then(() => setError(null)).catch(() => {}); else setAttempt((value) => value + 1); }}>重试</button></div> : null}
    {session ? <SessionContext.Provider value={session}>{props.children}</SessionContext.Provider> : !error ? <div role="status" className="conversation-session-loading">正在恢复工作…</div> : null}
  </>;
}
