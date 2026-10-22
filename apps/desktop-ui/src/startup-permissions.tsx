import { useState } from "react";
import { invoke } from "./host";
import { aiErrorMessage } from "./ai-runtime";
import { ConfirmDialog } from "./confirm-dialog";
import "./workspace-permissions.css";
import "./startup-permissions.css";

interface Inspection { status: "confirmationRequired" | "notApplicable"; planId?: string; directory?: string; userSid?: string; processElevated?: boolean }

export function StartupPermissions({ onRecovered }: { onRecovered?: () => void }) {
  const [plan, setPlan] = useState<Inspection | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [done, setDone] = useState(false);
  const check = async () => {
    setBusy(true); setError(""); setDone(false); setPlan(null);
    try { setPlan(await invoke<Inspection>("startup_permissions_check")); }
    catch (caught) { setError(aiErrorMessage(caught)); }
    finally { setBusy(false); }
  };
  const repair = async () => {
    if (!plan?.planId || busy) return;
    setBusy(true); setError("");
    try {
      const result = await invoke<{ status: string }>("startup_permissions_repair", { planId: plan.planId });
      if (result.status !== "ready") throw new Error("启动复查未通过，请导出错误日志反馈。");
      setDone(true); onRecovered?.();
    } catch (caught) { setError(aiErrorMessage(caught)); }
    finally { setBusy(false); setPlan(null); }
  };
  return <section className="startup-permissions" aria-label="启动数据权限修复" aria-busy={busy}>
    <button className="startup-permissions-button" type="button" disabled={busy} onClick={() => void check()}>
      <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m12 3 7 3v5c0 4.7-2.8 8.1-7 10-4.2-1.9-7-5.3-7-10V6z"/><path d="m9 12 2 2 4-4"/></svg>
      {busy ? "正在处理启动权限…" : "检查并修复启动权限"}
    </button>
    {error ? <p className="startup-permissions-result" data-tone="error" role="alert">{error}</p> : null}
    {done ? <p className="startup-permissions-result" data-tone="success" role="status">权限复查通过，本地服务已重新启动。</p> : null}
    {plan?.status === "confirmationRequired" && plan.planId ? <ConfirmDialog title="修复工作台启动数据权限" confirmLabel="修复并重试启动" busy={busy}
      onCancel={() => setPlan(null)} onConfirm={() => void repair()} description={<>
        <p>将检查并修复此账户的启动记录、数据库、凭据及其上级工作台数据目录的权限，保留会话和文件内容。</p>
        <code>{plan.directory}</code><p>Windows 用户：{plan.userSid}</p>
        <p>{plan.processElevated ? "当前工作台已具有管理员权限，确认后由独立修复助手处理此账户的数据权限，无需重复授权。" : "会先保存原权限；当前权限不足时，Windows 将提示管理员授权。"}修复后按原 Windows 账户重新检查权限并启动本地服务。</p>
        <p>发现其他账户所有的数据、目录链接或仍被服务占用时会停止修复。</p>
      </>}/> : null}
  </section>;
}
