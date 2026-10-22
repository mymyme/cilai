import { useCallback, useEffect, useRef, useState } from "react";
import { aiErrorMessage } from "./ai-runtime";
import { hasWorkbenchCredentialLocation } from "./credential-location";
import { ConfirmDialog } from "./confirm-dialog";
import { invoke } from "./host";
import type { ModelConnection, ModelCredentialCleanupCandidate, ModelCredentialCleanupPlan } from "./generated/protocol";
import "./model-connection-settings.css";

function localReference(candidate: ModelCredentialCleanupCandidate): boolean {
  return hasWorkbenchCredentialLocation(candidate.credentialRef, "model") || hasWorkbenchCredentialLocation(candidate.credentialRef, "model-network");
}

function canClean(candidate: ModelCredentialCleanupCandidate): boolean {
  return localReference(candidate) && candidate.canPrepare && !candidate.blockedReferences.length && !candidate.activeLeaseIds.length
    && candidate.state !== "confirmed" && candidate.state !== "not_required" && candidate.plan?.state !== "confirmed";
}

function blockedReason(candidate: ModelCredentialCleanupCandidate): string | null {
  if (candidate.activeLeaseIds.length) return `仍有 ${candidate.activeLeaseIds.length} 个任务占用`;
  if (candidate.blockedReferences.length) return `仍有 ${candidate.blockedReferences.length} 项引用`;
  if (candidate.state === "not_required") return "当前连接仍引用此凭据";
  if (!candidate.canPrepare && candidate.state !== "confirmed" && candidate.plan?.state !== "confirmed") return "尚不能清理，请刷新核对";
  return null;
}

function matchesPlan(candidate: ModelCredentialCleanupCandidate, plan: ModelCredentialCleanupPlan): boolean {
  return plan.cleanupId === candidate.cleanupId && plan.profileId === candidate.profileId
    && plan.connectionId === candidate.connectionId && plan.authGeneration === candidate.authGeneration
    && plan.referenceDigest === candidate.referenceDigest && plan.credentialRef.service === candidate.credentialRef.service
    && plan.credentialRef.account === candidate.credentialRef.account && ["pending", "confirmed"].includes(plan.state);
}

export function ModelCredentialCleanup(props: {
  connections: ModelConnection[];
  disabled: boolean;
  onChanged?: () => Promise<void>;
}) {
  const [candidates, setCandidates] = useState<ModelCredentialCleanupCandidate[]>([]);
  const [reading, setReading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<ModelCredentialCleanupCandidate | null>(null);
  const [attempted, setAttempted] = useState<Set<string>>(new Set());
  const mounted = useRef(false);
  const sequence = useRef(0);
  const active = useRef(false);
  const reload = useCallback(async () => {
    const current = ++sequence.current;
    setReading(true); setReadError(null);
    try {
      const value = await invoke<ModelCredentialCleanupCandidate[]>("model_credential_cleanup_list", { request: {} });
      if (mounted.current && current === sequence.current) setCandidates(value);
    } catch (reason) {
      if (mounted.current && current === sequence.current) setReadError(aiErrorMessage(reason));
    } finally {
      if (mounted.current && current === sequence.current) setReading(false);
    }
  }, []);
  useEffect(() => {
    mounted.current = true;
    void reload();
    return () => { mounted.current = false; ++sequence.current; };
  }, [reload]);
  const name = (candidate: ModelCredentialCleanupCandidate) => props.connections.find((connection) => connection.connectionId === candidate.connectionId)?.config.name
    || candidate.connectionId.slice(0, 8);
  const pending = (candidate: ModelCredentialCleanupCandidate) => candidate.state === "pending" || candidate.plan?.state === "pending" || attempted.has(candidate.cleanupId);
  const clean = async (candidate: ModelCredentialCleanupCandidate) => {
    if (active.current || props.disabled || !canClean(candidate)) return;
    active.current = true; setBusy(true); setError(null); setConfirm(null);
    setAttempted((previous) => new Set(previous).add(candidate.cleanupId));
    let confirmed = false;
    try {
      const plan = await invoke<ModelCredentialCleanupPlan>("model_credential_cleanup_execute", { request: {
        cleanupId: candidate.cleanupId, expectedReferenceDigest: candidate.referenceDigest,
      } });
      if (!mounted.current) return;
      if (!matchesPlan(candidate, plan)) throw new Error("model_credential_scope_invalid");
      confirmed = plan.state === "confirmed";
      setCandidates((previous) => previous.map((item) => item.cleanupId === candidate.cleanupId ? { ...item, plan, state: plan.state } : item));
      if (!confirmed) setError("清理尚未确认，请核对后继续清理。");
      await reload();
      if (confirmed && mounted.current) await props.onChanged?.();
    } catch (reason) {
      if (!mounted.current) return;
      setError(confirmed ? "清理已确认，连接列表尚未刷新。" : aiErrorMessage(reason));
      if (!confirmed) await reload();
    } finally {
      active.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  const disabled = props.disabled || busy || reading;
  return <section className="personal-model-auth" aria-label="历史凭据引用">
    <header><h2>历史凭据引用</h2><button type="button" disabled={disabled} onClick={() => void reload()}><RefreshIcon/>刷新</button></header>
    {reading ? <p role="status" className="settings-footnote">正在读取…</p> : null}
    {!reading && !readError && !candidates.length ? <p className="settings-footnote">暂无历史引用</p> : null}
    {candidates.map((candidate) => {
      const blocked = blockedReason(candidate);
      const local = localReference(candidate);
      const confirmed = local && (candidate.state === "confirmed" || candidate.plan?.state === "confirmed");
      return <div className="personal-model-auth-row" key={candidate.cleanupId} role="group" aria-label={`${name(candidate)} · 授权代次 ${candidate.authGeneration}`}>
        <header><h3>{name(candidate)}</h3><span role="status">{!local ? "需手工核对" : confirmed ? "已确认清理" : candidate.state === "not_required" ? "保留中" : "待清理"}</span></header>
        <p className="settings-footnote">授权代次 {candidate.authGeneration}{candidate.deletedConnection ? " · 连接已删除" : ""}</p>
        {!local ? <p className="settings-footnote">旧系统凭据需手工清理，工作台未确认删除。</p> : null}
        {blocked ? <p className="settings-footnote">{blocked}</p> : null}
        {local && !confirmed && candidate.state !== "not_required" ? <div className="personal-model-auth-actions"><button type="button" className="danger" disabled={disabled || !canClean(candidate)} onClick={() => setConfirm(candidate)}>{pending(candidate) ? "继续清理" : "清理"}</button></div> : null}
      </div>;
    })}
    {readError ? <p className="settings-error" role="alert">{readError}</p> : null}
    {error ? <p className="settings-error" role="alert">{error}</p> : null}
    {confirm ? <ConfirmDialog title="清理历史凭据？" confirmLabel={pending(confirm) ? "继续清理" : "清理"} danger busy={busy}
      confirmDisabled={props.disabled || reading || !canClean(confirm)}
      description={<><p>{name(confirm)} · 授权代次 {confirm.authGeneration}</p><p>仅清理此历史引用对应的工作台本地加密凭据，保留项目、会话和当前连接。是否已删除，以清理回执为准。</p></>}
      onCancel={() => setConfirm(null)} onConfirm={() => { void clean(confirm); }}/>
      : null}
  </section>;
}

function RefreshIcon() {
  return <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M16.5 8a6.5 6.5 0 1 0-.7 5.6M16.5 3.5V8H12"/></svg>;
}
