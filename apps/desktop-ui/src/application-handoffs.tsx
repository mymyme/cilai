import { useEffect, useRef, useState } from "react";
import { invoke } from "./host";
import { aiErrorMessage, type AiConversation } from "./ai-runtime";
import { useConversationSessionState, useConversationSessionFlush, useConversationSessionId, useConversationSessionRefresh } from "./conversation-session";
import { useModal } from "./use-modal";

export type ApplicationHandoff = { handoffId: string; applicationId: string; runId: string; requestId?: string; conversationId?: string; source: { applicationName: string; pageRevision?: number; contexts?: unknown[] }; output?: string; outputDigest?: string; receipt?: string; createdAt?: number };
type Project = { projectId: string; displayName: string; workspaceIds: string[]; primaryWorkspaceId: string | null; primaryWorkspaceRoot: string | null; primaryTeamSpaceId?: string | null };
export function ApplicationHandoffs(props: { projects: Project[]; projectsReady: boolean; projectsError: string; onRetryProjects: () => void; onOpen: (conversationId: string) => void }) {
  const [entries, setEntries] = useConversationSessionState<ApplicationHandoff[]>("applicationHandoffs", []);
  const [selected, setSelected] = useState<ApplicationHandoff | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [search, setSearch] = useState("");
  const handled = useRef(new Set<string>());
  const alive = useRef(true);
  const flush = useConversationSessionFlush();
  const refresh = useConversationSessionRefresh();
  const sessionId = useConversationSessionId();
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  async function remove(entry: ApplicationHandoff) {
    setEntries((values) => values.filter((value) => value.handoffId !== entry.handoffId || value.requestId !== entry.requestId));
    await flush();
  }
  async function cancel() {
    if (busy || !selected) return;
    setBusy(true); setError("");
    try {
      const result=await invoke<{status:string;conversationId?:string}>("ai_application_handoff_cancel",{request:{sessionId,handoffId:selected.handoffId,projectId:null}});
      await refresh();
      if (alive.current) {setSelected(null);if(result.status==="completed" && result.conversationId)props.onOpen(result.conversationId);}
    }
    catch (reason) { if (alive.current) setError(aiErrorMessage(reason)); }
    finally { if (alive.current) setBusy(false); }
  }
  const modal = useModal<HTMLDivElement>({ onClose: () => void cancel(), busy, active: selected !== null });
  useEffect(() => {
    if (selected || busy) return;
    const entry = entries.find((value) => value.requestId && !handled.current.has(value.requestId));
    if (!entry?.requestId) return;
    handled.current.add(entry.requestId);
    setError(""); setSearch("");
    if (!entry.conversationId) { setSelected(entry); return; }
    setBusy(true);
    void remove(entry).then(() => { if (alive.current) props.onOpen(entry.conversationId!); }).catch((reason) => { if (alive.current) { setError(aiErrorMessage(reason)); setSelected(entry); } }).finally(() => { if (alive.current) setBusy(false); });
  }, [entries, selected, busy]);
  async function choose(projectId: string | null) {
    if (!selected || busy) return;
    setBusy(true); setError("");
    try {
      await flush();
      const conversation = await invoke<AiConversation>("ai_application_handoff_commit", { request: { sessionId, handoffId: selected.handoffId, projectId } });
      await refresh();
      if (alive.current) { setSelected(null); props.onOpen(conversation.conversationId); }
    } catch (reason) { if (alive.current) setError(aiErrorMessage(reason)); }
    finally { if (alive.current) setBusy(false); }
  }
  const legacy = entries.filter((entry) => !entry.requestId);
  if (!legacy.length && !selected && !error) return null;
  return <aside className="application-handoffs" aria-label="应用转交">
    {legacy.map((entry) => <button type="button" key={entry.handoffId} onClick={() => { setSelected(entry); setSearch(""); setError(""); }}>{entry.source.applicationName} · 继续转交</button>)}
    {!selected && error ? <p role="alert">{error}</p> : null}
    {selected ? <div className="agent-picker-backdrop" role="dialog" aria-modal="true" aria-label="转交到主对话" ref={modal}>
      <section className="agent-picker conversation-project-picker">
        <header><h3>选择项目</h3><p>{selected.source.applicationName}</p></header>
        <div className="agent-picker-list">
          <button type="button" disabled={busy} onClick={() => void choose(null)}><span><strong>不关联项目</strong></span></button>
          {props.projectsReady && props.projects.length > 6 ? <input aria-label="搜索项目" placeholder="搜索项目" value={search} disabled={busy} onChange={(event) => setSearch(event.target.value)}/> : null}
          {props.projectsReady ? props.projects.filter((project) => project.displayName.includes(search.trim())).map((project) => <button key={project.projectId} type="button" disabled={busy || (project.workspaceIds.length > 0 && !(project.primaryWorkspaceId && (project.primaryWorkspaceRoot || project.primaryTeamSpaceId)))} onClick={() => void choose(project.projectId)}><span><strong>{project.displayName}</strong></span></button>) : props.projectsError ? <p role="alert">项目读取失败：{props.projectsError} <button type="button" disabled={busy} onClick={props.onRetryProjects}>重试</button></p> : <p role="status">正在读取项目…</p>}
        </div>
        {error ? <p role="alert">{error}</p> : null}
        {busy ? <p role="status">正在处理…</p> : null}
        <footer><button type="button" disabled={busy} onClick={() => void cancel()}>取消</button></footer>
      </section>
    </div> : null}
  </aside>;
}
