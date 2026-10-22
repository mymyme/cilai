import { useEffect, useRef, useState } from "react";
import { invoke } from "./host";
import { aiErrorMessage } from "./ai-runtime";
import { ConfirmDialog } from "./confirm-dialog";
import { CodeIcon, ConfirmIcon } from "./icons";
import { RuntimeStatusCard, StatusRefreshIcon } from "./runtime-status-card";
import "./powershell-policy.css";

type Policy = "Undefined" | "Restricted" | "AllSigned" | "RemoteSigned" | "Unrestricted" | "Bypass";
interface EngineState { engine: "windowsPowerShell" | "powerShell7"; path: string; scopes: {
  localMachine: Policy; currentUser: Policy; machinePolicy: Policy; userPolicy: Policy; effective: Policy;
} }
interface Status {
  status: "notApplicable" | "managed" | "restricted" | "configured";
  engines: EngineState[];
  planId?: string | null;
  processElevated?: boolean;
  restorePlanId?: string | null;
  restoreNote?: string | null;
  result?: "updated" | "restored";
  changes: Array<{engine:EngineState["engine"];before:Policy;after:Policy}>;
  restoreChanges: Array<{engine:EngineState["engine"];before:Policy;after:Policy}>;
}
const engineName = (engine: EngineState) => engine.engine === "windowsPowerShell" ? "Windows PowerShell" : "PowerShell 7";
const policyName = (value: Policy) => ({Undefined:"未设置",Restricted:"禁止脚本",AllSigned:"仅签名脚本",RemoteSigned:"允许本地脚本",Unrestricted:"允许脚本",Bypass:"跳过策略检查"})[value];

export function PowerShellPolicySettings() {
  const [status,setStatus] = useState<Status | null>(null);
  const [loading,setLoading] = useState(true);
  const [busy,setBusy] = useState(false);
  const [error,setError] = useState<string | null>(null);
  const [confirm,setConfirm] = useState<"enable" | "restore" | null>(null);
  const active = useRef(true);
  const generation = useRef(0);
  const load = async () => {
    const id = ++generation.current;
    setLoading(true); setError(null); setStatus(null); setConfirm(null);
    try { const next = await invoke<Status>("powershell_policy_check"); if(active.current && id === generation.current) { setStatus(next); return next; } }
    catch(cause) { if(active.current && id === generation.current) setError(aiErrorMessage(cause)); }
    finally { if(active.current && id === generation.current) setLoading(false); }
    return null;
  };
  const prepareConfirmation = async (action: "enable" | "restore") => {
    const current = await load();
    if (current && (action === "restore" ? current.restorePlanId : current.planId)) setConfirm(action);
  };
  useEffect(() => { active.current = true; void load(); return () => { active.current = false; generation.current++; }; }, []);
  const apply = async () => {
    const planId = confirm === "restore" ? status?.restorePlanId : status?.planId;
    if (!planId || busy) return;
    const id = generation.current;
    setBusy(true); setError(null);
    try { const next = await invoke<Status>("powershell_policy_apply",{planId}); if(active.current && id === generation.current) setStatus(next); }
    catch(cause) { if(active.current && id === generation.current) {setError(aiErrorMessage(cause));setStatus(null);} }
    finally { if(active.current && id === generation.current) {setBusy(false);setConfirm(null);} }
  };
  if(status?.status === "notApplicable") return null;
  const label = busy ? "正在修改…" : loading ? "正在检查…" : error ? "检查未完成" : status?.status === "managed" ? "企业管理" : status?.status === "configured" ? "已配置" : "需调整";
  return <>
    <RuntimeStatusCard title="脚本执行策略" className="powershell-policy" icon={<CodeIcon/>}
      description="管理标准安装的 Windows PowerShell 与 PowerShell 7。"
      status={label} tone={busy || loading ? "neutral" : error ? "error" : status?.status === "configured" ? "success" : "warning"}
      hint={status?.status === "managed" ? "企业组策略已接管，请联系管理员调整。" : "修改本机所有用户的系统默认策略，需要管理员授权。"}
      actions={<>
        <button type="button" disabled={loading || busy} onClick={() => void load()}><StatusRefreshIcon spin={loading}/>{loading ? "检查中…" : "重新检查策略"}</button>
        {status?.restorePlanId ? <button type="button" disabled={busy || loading} onClick={() => void prepareConfirmation("restore")}>恢复原策略</button> : null}
        {status?.planId ? <button className="primary" type="button" disabled={busy || loading} onClick={() => void prepareConfirmation("enable")}><ConfirmIcon/>允许本地脚本</button> : null}
      </>}>
      {status?.engines?.length ? <div className="powershell-policy-engines">{status.engines.map(engine => <div key={engine.engine}>
        <div className="powershell-policy-engine-title"><strong>{engineName(engine)}</strong><span title={engine.scopes.localMachine}>{policyName(engine.scopes.localMachine)}</span></div>
        <p title={engine.path}>{engine.path}</p>
        <details><summary>查看策略来源</summary><dl>
          <div><dt>系统默认 LocalMachine</dt><dd>{engine.scopes.localMachine}</dd></div>
          <div><dt>当前桌面用户 CurrentUser</dt><dd>{engine.scopes.currentUser}</dd></div>
          <div><dt>企业计算机策略</dt><dd>{engine.scopes.machinePolicy}</dd></div>
          <div><dt>企业用户策略</dt><dd>{engine.scopes.userPolicy}</dd></div>
          <div><dt>当前账号生效策略</dt><dd>{engine.scopes.effective}</dd></div>
        </dl></details>
      </div>)}</div> : null}
      {status?.result ? <p className="powershell-policy-result" role="status"><ConfirmIcon/>{status.result === "restored" ? "已恢复修改前的系统策略。" : "系统默认策略已更新为 RemoteSigned。请重试脚本操作。"}</p> : null}
      {status ? <p className="powershell-policy-note">RemoteSigned 允许本地脚本，来自互联网的脚本仍需签名。用户或工具进程的策略可能优先于系统默认；非标准安装的 PowerShell 需单独处理。</p> : null}
      {status?.restoreNote ? <p className="runtime-status-card-error" role="alert">{status.restoreNote}</p> : null}
      {error ? <p className="runtime-status-card-error" role="alert">{error}</p> : null}
    </RuntimeStatusCard>
    {confirm && status ? <ConfirmDialog title={confirm === "restore" ? "恢复原脚本策略" : "允许本地 PowerShell 脚本"}
      confirmLabel={status.processElevated ? "确认修改" : "请求管理员授权"} busy={busy} onCancel={() => setConfirm(null)} onConfirm={() => void apply()}
      description={<>
        <p>{confirm === "restore" ? "将恢复工作台修改前备份的 LocalMachine 策略。外部修改过的策略不会被覆盖。" : "将未允许本地脚本的标准 PowerShell 系统默认策略设为 RemoteSigned。"}</p>
        <p>影响范围：本机所有 Windows 用户，包括工作台的独立沙箱账号。此设置也会影响其他使用相同 PowerShell 的软件。</p>
        <ul>{(confirm === "restore" ? status.restoreChanges : status.changes).map(change => <li key={change.engine}>{change.engine === "windowsPowerShell" ? "Windows PowerShell" : "PowerShell 7"}：{change.before} → {change.after}</li>)}</ul>
        <p>工作台会先保存原策略。{status.processElevated ? "工作台已获得管理员权限，确认后执行修改，无需重复授权。" : "确认后将显示 Windows 管理员授权提示。"}企业组策略保持优先。</p>
      </>}/> : null}
  </>;
}
