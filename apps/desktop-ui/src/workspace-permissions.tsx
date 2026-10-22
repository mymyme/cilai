import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "./host";
import { aiErrorMessage } from "./ai-runtime";
import { ConfirmDialog } from "./confirm-dialog";
import { RuntimeStatusCard, StatusRefreshIcon } from "./runtime-status-card";
import { ConfirmIcon, TreeFolderIcon } from "./icons";
import "./workspace-permissions.css";

interface PermissionStatus {
  status: "ready" | "repairRequired" | "notApplicable";
  directory?: string;
  userSid?: string;
  processElevated?: boolean;
  planId?: string | null;
}

/** Only mounted for a registered local Windows project. Checking never elevates. */
export function WorkspacePermissions(props: { workspaceId: string; compact?: boolean; disabled?: boolean }) {
  const [status, setStatus] = useState<PermissionStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [repairing, setRepairing] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [repaired, setRepaired] = useState(false);
  const generation = useRef(0);
  const load = useCallback(async () => {
    const request = ++generation.current;
    setLoading(true); setStatus(null); setError(null); setConfirming(false); setRepaired(false);
    try {
      const result = await invoke<PermissionStatus>("workspace_permissions_check", { workspaceId: props.workspaceId });
      if (generation.current === request) { setStatus(result); return result; }
    } catch (caught) { if (generation.current === request) setError(aiErrorMessage(caught)); }
    finally { if (generation.current === request) setLoading(false); }
    return null;
  }, [props.workspaceId]);
  const prepareRepair = async () => {
    // Cached display state may outlive the short-lived administrator plan.
    const current = await load();
    if (current?.status === "repairRequired" && current.planId) setConfirming(true);
  };
  useEffect(() => { setRepairing(false); void load(); return () => { generation.current += 1; }; }, [load]);
  const repair = async () => {
    if (!status?.planId || repairing || props.disabled) return;
    const request = generation.current;
    const planId = status.planId;
    setRepairing(true); setError(null);
    try {
      const result = await invoke<PermissionStatus>("workspace_permissions_repair", { workspaceId: props.workspaceId, planId });
      if (generation.current === request) { setStatus(result); setRepaired(result.status === "ready"); }
    } catch (caught) {
      if (generation.current === request) { setError(aiErrorMessage(caught)); setStatus(null); }
    } finally {
      if (generation.current === request) { setRepairing(false); setConfirming(false); }
    }
  };
  if (status?.status === "notApplicable" || (props.compact && !error && !repaired && (loading || status?.status === "ready"))) return null;
  const directory = displayDirectory(status?.directory);
  const label = repairing ? "正在修复…" : loading ? "正在检查…" : error ? "检查失败" : status?.status === "ready" ? "已授权" : "需要授权";
  return <>
    <RuntimeStatusCard className={props.compact ? "workspace-permissions-compact" : undefined}
      title="项目目录权限" icon={<TreeFolderIcon/>} status={label}
      tone={loading || repairing ? "neutral" : error ? "error" : status?.status === "ready" ? "success" : "warning"}
      description="用于初始化当前项目的沙箱访问权限。"
      hint={loading ? "正在检查项目目录…"
      : repaired ? "目录权限已修复，并已通过不依赖管理员权限的复查。请重试刚才的操作。"
        : status?.status === "ready" ? "当前用户可以更新项目目录的沙箱权限。"
          : error ? "请重新检查项目权限。" : "当前目录需要管理员授权，修复后可重新执行。"}
      actions={<>
      <button type="button" aria-label="重新检查项目权限" disabled={loading || repairing || props.disabled} onClick={() => void load()}><StatusRefreshIcon spin={loading}/>{loading ? "检查中…" : "重新检查"}</button>
      {status?.status === "repairRequired" && status.planId ? <button className="primary" type="button" disabled={repairing || props.disabled}
        onClick={() => void prepareRepair()}><ConfirmIcon/>{repairing ? "修复中…" : "修复项目权限"}</button> : null}
      </>}>
      {directory ? <div className="workspace-permissions-directory"><span>项目目录</span><span title={directory}>{directory}</span></div> : null}
      {error ? <p className="runtime-status-card-error" role="alert">{error}</p> : null}
    </RuntimeStatusCard>
    {confirming && status?.planId ? <ConfirmDialog title="修复项目目录权限" confirmLabel={status.processElevated ? "确认修复" : "请求管理员授权"} busy={repairing} confirmDisabled={props.disabled}
      onCancel={() => setConfirming(false)} onConfirm={() => void repair()} description={<>
        <p>将为当前 Windows 用户增加下列目录的“更改权限”权限，让沙箱能够完成权限初始化。</p>
        <code>{directory}</code><p>用户：{status.userSid}</p>
        <p>仅作用于这个目录，保留原有权限和所有者，不递归修改文件。工作台会先保存权限备份。</p>
        <p>{status.processElevated ? "工作台已获得管理员权限，确认后执行修复，无需重复授权。" : "确认后将显示 Windows 管理员授权提示。"}</p>
      </>}/> : null}
  </>;
}

// Display only; the native permission plan continues to use its canonical path.
function displayDirectory(path?: string) {
  if (path?.startsWith("\\\\?\\UNC\\")) return `\\\\${path.slice(8)}`;
  if (path?.startsWith("\\\\?\\")) return path.slice(4);
  return path;
}
