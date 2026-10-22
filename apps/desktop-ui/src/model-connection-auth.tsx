import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "./host";
import { aiErrorMessage } from "./ai-runtime";
import { ConfirmDialog } from "./confirm-dialog";
import type { ModelRuntimeKind, RootAuthChange } from "./generated/protocol";

interface AuthStatus {
  account: { type: string; email?: string | null } | null;
  runtimeKind: ModelRuntimeKind;
  pendingLoginId?: string | null;
  operationId?: string;
  loginStatus?: string;
}
interface AuthStart { operationId: string; loginId: string; runtimeKind: ModelRuntimeKind; verificationUrl: string; userCode: string }
const pendingChange = (change: RootAuthChange) => ["prepared", "in_progress", "uncertain"].includes(change.state);
const runtimeLabel = (kind: ModelRuntimeKind) => kind === "workbench" ? "主对话" : "页面 AI";
const operationLabel = (change: RootAuthChange) => change.intent === "chatgpt_logout" ? "退出尚未完成"
  : change.intent === "chatgpt_status" ? "授权状态核对尚未完成" : change.intent === "model_catalog" ? "目录读取尚未完成"
    : `${runtimeLabel(change.runtimeKind ?? "workbench")}授权尚未完成`;

export function ModelConnectionAuth(props: { connectionId: string; disabled: boolean; onChanged?: () => Promise<void> }) {
  const [changes, setChanges] = useState<RootAuthChange[]>([]);
  const [ready, setReady] = useState(false);
  const [revision, setRevision] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [logout, setLogout] = useState<{ runtimeKind: ModelRuntimeKind; operationId?: string } | null>(null);
  const active = useRef(false);
  const epoch = useRef(0);
  const reload = useCallback(async () => {
    const current = epoch.current;
    const value = await invoke<RootAuthChange[]>("model_connection_auth_changes", { request: { connectionId: props.connectionId } });
    if (current === epoch.current) { setChanges(value.filter(pendingChange)); setReady(true); }
  }, [props.connectionId]);
  useEffect(() => {
    const current = ++epoch.current; setReady(false); setChanges([]); setError(null); setLogout(null);
    void reload().catch((reason) => { if (current === epoch.current) setError(aiErrorMessage(reason)); });
    return () => { ++epoch.current; };
  }, [reload]);
  const run = async (action: () => Promise<void>, refreshStatus = true) => {
    if (active.current) return;
    active.current = true; setBusy(true); setError(null);
    const current = epoch.current;
    try { await action(); }
    catch (reason) { if (current === epoch.current) setError(aiErrorMessage(reason)); }
    finally {
      if (current === epoch.current) {
        try { await reload(); } catch (reason) { setError(aiErrorMessage(reason)); }
        if (refreshStatus) setRevision((value) => value + 1);
        setBusy(false);
      }
      active.current = false;
    }
  };
  const changed = async () => { await reload(); await props.onChanged?.(); };
  return <section className="personal-model-auth" aria-label="ChatGPT 授权"><h2>ChatGPT 授权</h2>
    {(["workbench", "application"] as const).map((runtimeKind) => <Authorization key={`${props.connectionId}:${runtimeKind}`}
      connectionId={props.connectionId} runtimeKind={runtimeKind} disabled={busy || props.disabled || !ready} revision={revision}
      pending={changes.find((change) => change.intent === "chatgpt_login" && change.runtimeKind === runtimeKind)}
      startDisabled={changes.length > 0} onRun={run} onChanged={changed} onStatusRead={reload} onLogout={() => setLogout({ runtimeKind })}/>)}
    {changes.length ? <div aria-label="未完成授权操作">{changes.map((change) => <div key={change.operationId}>
      <strong>{operationLabel(change)}</strong>
      <code>{change.operationId}</code>
      {change.steps.map((step) => <p key={`${step.runtimeKind}:${step.authGeneration}`}>{runtimeLabel(step.runtimeKind)} · {step.state === "signed_out" ? "已退出" : step.state === "confirmed" ? "已确认" : step.state === "uncertain" || step.state === "failed" ? "待核对" : "进行中"}</p>)}
      <button type="button" disabled={busy || props.disabled} onClick={() => void run(async () => {
        await invoke("model_connection_auth_resume", { request: { connectionId: props.connectionId, operationId: change.operationId } });
      }, false)}>核对状态</button>
      {change.intent === "chatgpt_logout" ? <button type="button" disabled={busy || props.disabled} onClick={() => setLogout({ runtimeKind: "workbench", operationId: change.operationId })}>继续退出</button> : null}
      {change.intent === "chatgpt_status" || change.intent === "model_catalog" ? <button type="button" disabled={busy || props.disabled} onClick={() => void run(async () => {
        await invoke("model_connection_auth_cancel_readonly", { request: { connectionId: props.connectionId, operationId: change.operationId } });
        await changed();
      }, false)}>结束未完成核对</button> : null}
    </div>)}</div> : null}
    {error ? <p className="settings-error" role="alert">{error}</p> : null}
    {logout ? <ConfirmDialog title="退出此连接的 ChatGPT 授权？" confirmLabel={logout.operationId ? "继续退出" : "退出"} danger busy={busy}
      description="将退出此连接的主对话、页面 AI 及先前授权。项目和会话保留。" onCancel={() => setLogout(null)} onConfirm={() => {
        const target = logout; setLogout(null);
        void run(async () => {
          await invoke(target.operationId ? "model_connection_auth_finish_logout" : "model_connection_auth_logout", { request: target.operationId
            ? { connectionId: props.connectionId, operationId: target.operationId }
            : { connectionId: props.connectionId, runtimeKind: target.runtimeKind } });
          await changed();
        });
      }}/> : null}
  </section>;
}

function Authorization(props: { connectionId: string; runtimeKind: ModelRuntimeKind; disabled: boolean; revision: number;
  pending?: RootAuthChange; startDisabled: boolean; onRun: (action: () => Promise<void>, refreshStatus?: boolean) => Promise<void>;
  onChanged: () => Promise<void>; onStatusRead: () => Promise<void>; onLogout: () => void }) {
  const [status, setStatus] = useState<AuthStatus | null>(null);
  const [device, setDevice] = useState<AuthStart | null>(null);
  const [reading, setReading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  const target = { connectionId: props.connectionId, runtimeKind: props.runtimeKind };
  const read = async () => {
    const value = await invoke<AuthStatus>("model_connection_auth_read", { request: target });
    if (value.runtimeKind !== props.runtimeKind) throw new Error("model_auth_target_invalid");
    return value;
  };
  useEffect(() => {
    const current = ++generation.current;
    setReading(true);
    void read().then(async (value) => {
      if (current !== generation.current) return;
      setStatus(value); setError(null);
      setDevice((old) => old && value.pendingLoginId === old.loginId ? old : null);
      if (value.loginStatus === "authenticated" && props.pending) await props.onChanged();
    }).catch((reason) => { if (current === generation.current) { setStatus(null); setError(aiErrorMessage(reason)); } })
      .finally(async () => {
        if (current !== generation.current) return;
        try { await props.onStatusRead(); }
        catch (reason) { if (current === generation.current) setError(aiErrorMessage(reason)); }
        finally { if (current === generation.current) setReading(false); }
      });
    return () => { ++generation.current; };
  }, [props.connectionId, props.runtimeKind, props.revision]);
  const pendingId = device?.loginId ?? status?.pendingLoginId;
  const pendingOperation = props.pending?.operationId ?? status?.operationId;
  const authorized = status?.account?.type === "chatgpt";
  const label = error ? "读取失败" : reading && !status ? "正在读取…" : authorized ? "已授权" : status?.loginStatus === "unconfirmed" ? "待核对" : props.pending || pendingId ? "授权进行中" : "未授权";
  const disabled = props.disabled || reading;
  return <div className="personal-model-auth-row" role="group" aria-label={props.runtimeKind === "workbench" ? "主对话授权" : "页面授权"}>
    <header><h3>{runtimeLabel(props.runtimeKind)}</h3><span role="status">{label}</span></header>
    <div className="personal-model-auth-actions">
      <button type="button" disabled={disabled} onClick={() => void props.onRun(async () => {})}>刷新状态</button>
      {!authorized && !pendingId && !props.pending ? <button type="button" disabled={disabled || props.startDisabled || Boolean(error)} onClick={() => void props.onRun(async () => {
        const value = await invoke<AuthStart>("model_connection_auth_start", { request: target });
        if (value.runtimeKind !== props.runtimeKind || !value.operationId || !value.loginId || !value.verificationUrl || !value.userCode) throw new Error("model_auth_login_response_invalid");
        setDevice(value);
      })}>开始授权</button> : null}
      {pendingId || props.pending ? <button type="button" disabled={disabled} onClick={() => void props.onRun(async () => {
        await invoke("model_connection_auth_cancel", { request: { ...target, ...(pendingId ? { loginId: pendingId } : {}), ...(pendingOperation ? { operationId: pendingOperation } : {}) } });
        setDevice(null); await props.onChanged();
      })}>取消授权</button> : null}
      {authorized ? <button type="button" disabled={disabled || props.startDisabled} onClick={props.onLogout}>退出授权</button> : null}
    </div>
    {device ? <div className="personal-model-device-code"><span>在官方授权页输入</span><output aria-label="授权码">{device.userCode}</output><button type="button" disabled={disabled} onClick={() => void props.onRun(async () => { await invoke("model_connection_auth_open", { url: device.verificationUrl }); }, false)}>打开授权页</button></div> : null}
    {(pendingId || props.pending) && !device ? <p className="settings-footnote">完成授权后刷新状态；取消后可重新获取授权码。</p> : null}
    {error ? <p className="settings-error" role="alert">{error}</p> : null}
  </div>;
}
