import { useEffect, useRef, useState } from "react";
import { aiErrorMessage, checkAiRunRecovery, continueAiRun, type AiAgentRun, type AiRunRecoveryCheck } from "./ai-runtime";
import { ConversationActionButton } from "./conversation-view";

function RecoveryIcon({ spinning = false }: { spinning?: boolean }) {
  return <svg aria-hidden="true" className={spinning ? "ai-recovery-spinner" : undefined} viewBox="0 0 20 20"><path d="M16.5 9a6.5 6.5 0 1 0-1.8 5.6M16.5 4.5V9H12"/></svg>;
}

type RecoveryState = {
  key: string;
  phase: "checking" | "idle" | "continuing";
  checked: AiRunRecoveryCheck | null;
  error: string | null;
};

export function useRunRecovery({ run, scope, disabled, onContinued, onRefresh }: {
  run: Pick<AiAgentRun, "runId" | "status" | "threadWriterActive"> | null;
  scope: string | null;
  disabled: boolean;
  onContinued: (run: AiAgentRun) => void;
  onRefresh: () => void;
}) {
  const runId = run?.runId;
  const key = run && scope ? JSON.stringify([scope, runId, run.status, run.threadWriterActive]) : null;
  const [state, setState] = useState<RecoveryState | null>(null);
  const generation = useRef(0);
  const submitting = useRef(false);
  const current = state?.key === key ? state : null;
  const phase = current?.phase ?? "checking";
  const checked = current?.checked ?? null;
  const error = current?.error ?? null;

  const check = async () => {
    if (!key || !runId) return;
    const requested = ++generation.current;
    setState({ key, phase: "checking", error: null, checked: null });
    try {
      const result = await checkAiRunRecovery(runId);
      if (requested !== generation.current) return;
      if (result?.runId !== runId) throw new Error("ai_recovery_response_invalid");
      setState({ key, phase: "idle", checked: result, error: null });
    } catch (failure) {
      if (requested === generation.current) {
        setState({ key, phase: "idle", checked: null, error: aiErrorMessage(failure) });
      }
    }
  };

  useEffect(() => {
    if (key) void check(); else setState(null);
    return () => { generation.current += 1; };
  }, [key]);

  const resume = async () => {
    if (!key || !runId || submitting.current || disabled || phase !== "idle" || checked?.state !== "ready") return;
    submitting.current = true;
    const requested = generation.current;
    setState({ key, phase: "continuing", checked, error: null });
    try {
      const continued = await continueAiRun(runId);
      if (requested !== generation.current) return;
      setState({ key, phase: "idle", checked: null, error: null });
      onContinued(continued);
      onRefresh();
    } catch (failure) {
      if (requested !== generation.current) return;
      setState({ key, phase: "idle", checked: null, error: aiErrorMessage(failure) });
      onRefresh();
    } finally {
      submitting.current = false;
    }
  };

  return key ? { checked, phase, error, check, resume, onRefresh, disabled, isSubmitting: () => submitting.current } : null;
}

export type RunRecovery = NonNullable<ReturnType<typeof useRunRecovery>>;

export function PauseReplyButton({ disabled, onPause }: { disabled: boolean; onPause: () => void }) {
  return <ConversationActionButton stop aria-label="暂停回复" title="暂停回复，核对后可继续" disabled={disabled} onClick={onPause} type="button">
    <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M9 7v10M15 7v10"/></svg>
  </ConversationActionButton>;
}

export function ContinueReplyButton({ recovery, disabled }: { recovery: RunRecovery; disabled: boolean }) {
  const continuing = recovery.phase === "continuing";
  const label = continuing ? "正在继续" : "继续回复";
  return <ConversationActionButton aria-label={label} title={label} disabled={disabled || continuing}
    onClick={() => void recovery.resume()} type="button">
    <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M9 6.5 18 12l-9 5.5z"/></svg>
  </ConversationActionButton>;
}

export function RunRecoveryStatus({ recovery }: { recovery: RunRecovery }) {
  const { phase, checked, error, check, onRefresh, disabled } = recovery;
  const working = phase !== "idle";
  const label = phase === "checking" ? "正在核对" : phase === "continuing" ? "正在继续" : error ? "状态待核实"
    : checked?.state === "ready" ? "可继续原任务" : checked?.summary ?? "状态待核实";
  return <div aria-label="执行恢复" className="ai-run-recovery" data-working={working || undefined}>
    <span aria-live="polite" role="status"><RecoveryIcon spinning={working}/>{label}</span>
    {!working && checked?.state === "completed" ? <button onClick={onRefresh} type="button">刷新</button>
      : !working && checked?.state === "continued" ? <button onClick={onRefresh} type="button">查看</button>
      : !working && checked?.state !== "ready" ? <button disabled={disabled} onClick={() => void check()} type="button">重新核对</button> : null}
    {error ? <details className="ai-recovery-detail"><summary>详情</summary><p>{error}</p></details> : null}
  </div>;
}
