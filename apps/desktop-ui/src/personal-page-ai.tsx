import { useEffect, useRef, useState, type RefObject } from "react";
import { invoke } from "./host";
import { useModal } from "./use-modal";
import { aiErrorMessage } from "./ai-runtime";

type Approval = { confirmationId: string; applicationName: string; toolName: string; toolTitle: string; description: string; riskLevel: string; arguments: unknown; executionStatus?: string; confirmationView?: { reviewMode?: string; inputSchema?: unknown } };
type Run = { runId: string; status: string; writeRequest?: { approval: Approval; validUntil: number } | null };
type Options = { frame: RefObject<HTMLIFrameElement | null>; channel: string; url: string; active: boolean; enabled: boolean };
const TERMINAL = new Set(["completed", "failed", "cancelled", "recovery_required"]);

export function usePersonalPageAi({ frame, channel, url, active, enabled }: Options) {
  const [runs, setRuns] = useState<Record<string, Run>>({});
  const [error, setError] = useState("");
  const [needsModel, setNeedsModel] = useState(false);
  const alive = useRef(false);
  const isActive = useRef(active);
  isActive.current = active;
  const pendingRequests = useRef(new Set<string>());
  const pageCall = (method: string, params: unknown, requestId = crypto.randomUUID()) => invoke<unknown>("personal_page_ai_dispatch", { url, request: { method, requestId, params } });
  const record = (value: unknown) => {
    if (value && typeof value === "object" && "runId" in value && "status" in value && typeof value.runId === "string" && typeof value.status === "string") {
      const run = value as Run;
      setRuns(current => ({ ...current, [run.runId]: run }));
    }
  };
  useEffect(() => {
    alive.current = true; setRuns({}); setError(""); setNeedsModel(false); pendingRequests.current.clear();
    return () => { alive.current = false; };
  }, [url, channel]);
  useEffect(() => {
    if (!enabled || !url) return;
    let current = true;
    const receive = (event: MessageEvent) => {
      const message = event.data;
      if (event.source !== frame.current?.contentWindow || !message || message.channel !== channel || message.type !== "cilai-personal-ai-request") return;
      if (typeof message.id !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(message.id)) return;
      const respond = (result?: unknown, reason?: string) => {
        if (current) frame.current?.contentWindow?.postMessage({ type: "cilai-personal-ai-result", channel, id: message.id, result, error: reason }, "*");
      };
      if (!isActive.current || pendingRequests.current.has(message.id) || pendingRequests.current.size >= 8) { respond(undefined, "应用当前不能处理请求"); return; }
      if (!["session.read", "context.publish", "context.revoke", "uiAction.register", "model.session.open", "model.session.read", "model.run.create", "model.run.get", "model.run.cancel", "conversation.handoff", "audit.list"].includes(message.method)) { respond(undefined, "页面不支持此请求"); return; }
      pendingRequests.current.add(message.id);
      void pageCall(message.method, message.params, message.id).then(result => {
        if (!current) return;
        record(result); setError(""); setNeedsModel(false); respond(result);
      }).catch(reason => {
        if (current && message.method.startsWith("model.")) { setError(aiErrorMessage(reason)); setNeedsModel(/model_(purpose|connection|credential|descriptor|validation)/.test(String(reason))); }
        respond(undefined, String(reason));
      }).finally(() => { pendingRequests.current.delete(message.id); });
    };
    window.addEventListener("message", receive);
    return () => { current = false; window.removeEventListener("message", receive); };
  }, [url, channel, enabled]);
  const activeRunIds = Object.values(runs).filter(run => !TERMINAL.has(run.status)).map(run => run.runId).sort().join(",");
  useEffect(() => {
    if (!enabled || !url || !activeRunIds) return;
    let current = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        for (const runId of activeRunIds.split(",")) {
          const result = await pageCall("model.run.get", { runId });
          if (!current) return;
          record(result);
        }
        if (current) { setError(""); timer = setTimeout(() => void poll(), 500); }
      } catch (reason) { if (current) { setError(aiErrorMessage(reason)); timer = setTimeout(() => void poll(), 2000); } }
    };
    void poll();
    return () => { current = false; clearTimeout(timer); };
  }, [activeRunIds, url, enabled]);
  const pending = Object.values(runs).find(run => run.writeRequest?.approval.executionStatus === "awaiting_confirmation");
  const resolve = async (decision: "approved" | "rejected", args?: unknown) => {
    if (!pending?.writeRequest) throw new Error("确认已失效");
    const result = await invoke<Run>("personal_page_ai_resolve", { url, runId: pending.runId, confirmationId: pending.writeRequest.approval.confirmationId, decision, ...(args === undefined ? {} : { arguments: args }) });
    if (alive.current) { record(result); setError(""); }
  };
  return { pending, resolve, error, needsModel };
}

export function PersonalPageAiStatus({ error, needsModel, onOpenModels }: { error: string; needsModel: boolean; onOpenModels?: () => void }) {
  return error ? <p className="personal-page-ai-status" role="alert">{error}{needsModel && onOpenModels ? <> <button type="button" onClick={onOpenModels}>设置模型</button></> : null}</p> : null;
}

export function PersonalPageAiConfirmation({ pending, resolve, active = true }: { pending: Run; resolve: (decision: "approved" | "rejected", args?: unknown) => Promise<void>; active?: boolean }) {
  const request = pending.writeRequest!;
  const approval = request.approval;
  const [text, setText] = useState(() => JSON.stringify(approval.arguments, null, 2));
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 500); return () => clearInterval(timer); }, []);
  const submit = async (decision: "approved" | "rejected") => {
    if (busy || uncertain || !active) return;
    let args: unknown;
    if (decision === "approved") {
      try { args = JSON.parse(text); } catch { setError("参数必须是有效 JSON"); return; }
      if (!args || typeof args !== "object" || Array.isArray(args)) { setError("参数必须是 JSON 对象"); return; }
    }
    setBusy(true); setError("");
    try { await resolve(decision, args); }
    catch (reason) { const message = String(reason); setError(message); setUncertain(!message.includes("application_write_arguments_invalid")); }
    finally { setBusy(false); }
  };
  const modal = useModal<HTMLDivElement>({ onClose: () => void submit("rejected"), busy: busy || uncertain, active });
  return <div className="agent-picker-backdrop" role="dialog" aria-modal="true" aria-label="确认页面操作" ref={modal}><section className="artifact-review">
    <header><h3>确认页面操作</h3><button disabled={busy || uncertain || !active} onClick={() => void submit("rejected")}>拒绝</button></header>
    <p>{approval.applicationName} · {approval.toolTitle || approval.toolName} · {approval.riskLevel}</p>
    {approval.description ? <p>{approval.description}</p> : null}
    <label>参数<textarea aria-label="页面操作参数" value={text} onChange={event => setText(event.target.value)} disabled={busy || uncertain || !active} spellCheck={false}/></label>
    {approval.confirmationView?.inputSchema ? <details><summary>参数约束</summary><pre className="personal-capability-result">{JSON.stringify(approval.confirmationView.inputSchema, null, 2)}</pre></details> : null}
    {error ? <p role="alert">{error}</p> : null}
    {uncertain ? <p role="status">结果未确认，请核对回执。</p> : request.validUntil <= now ? <p role="status">确认已过期</p> : null}
    <button className="primary" disabled={busy || uncertain || !active || request.validUntil <= now} onClick={() => void submit("approved")}>{busy ? "正在处理…" : "批准"}</button>
  </section></div>;
}
