import { useWindowsSandbox } from "./use-windows-sandbox";
import type { WorkspaceSummary } from "./generated/protocol";
import { WorkspacePermissions } from "./workspace-permissions";
import { PowerShellPolicySettings } from "./powershell-policy";
import { RuntimeStatusCard, StatusRefreshIcon } from "./runtime-status-card";
import { ModelConnectionSettings } from "./model-connection-settings";
import type { MessageSink } from "./app-message";

export function AiSettingsPage(props: { workspace: WorkspaceSummary | null; onMessage: MessageSink; hostPlatform: string | null }) {
  return <section className="runtime-manager-page model-settings-page">
    <header className="runtime-manager-header"><h1>模型设置</h1></header>
    <div className="runtime-manager-scroll">
      <ModelConnectionSettings/>
      {props.hostPlatform === "windows" ? <WindowsSandboxSettings workspaceRoot={props.workspace?.workspaceKind === "local" ? props.workspace.localRoot : null}/> : null}
      {props.hostPlatform === "windows" && props.workspace?.workspaceKind === "local" ? <WorkspacePermissions key={props.workspace.workspaceId} workspaceId={props.workspace.workspaceId}/> : null}
      {props.hostPlatform === "windows" ? <PowerShellPolicySettings/> : null}
    </div>
  </section>;
}

function WindowsSandboxSettings(props: { workspaceRoot: string | null }) {
  const {status,loading,settingUp,error,available,refresh:load,setup} = useWindowsSandbox("windows");

  const label = loading ? "正在检查…"
    : status?.status === "ready" ? "已初始化"
      : status?.status === "updateRequired" ? "需要更新"
        : status?.status === "notConfigured" ? "未初始化" : "不可用";
  const needsSetup = status?.status === "notConfigured" || status?.status === "updateRequired";
  const detail = error ?? status?.errorMessage;

  return <RuntimeStatusCard title="Windows 安全沙箱" icon={<SandboxIcon/>}
    description="初始化后，每次运行前还会检查当前项目的沙箱执行环境。" status={label}
    tone={loading || settingUp ? "neutral" : detail ? "error" : status?.status === "ready" ? "success" : "warning"}
    hint={needsSetup ? "初始化会请求一次管理员批准，日常运行无需管理员权限。" : "仅允许在提升权限沙箱中执行，不会降级为无沙箱运行。"}
    actions={<>
        <button disabled={loading || settingUp || !available} onClick={() => void load()} type="button"><StatusRefreshIcon spin={loading}/>{loading ? "检查中…" : "重新检查"}</button>
        {needsSetup ? <button className="primary" disabled={loading || settingUp || !available} onClick={() => void setup(props.workspaceRoot)} type="button"><SandboxIcon/>{settingUp ? "初始化中…" : status?.status === "updateRequired" ? "重新初始化" : "初始化"}</button> : null}
    </>}>
    {detail ? <p className="runtime-status-card-error" role="alert">{detail}</p> : null}
  </RuntimeStatusCard>;
}


function SandboxIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M12 3 19 6v5c0 4.7-2.8 8.1-7 10-4.2-1.9-7-5.3-7-10V6z"/><path d="m9 12 2 2 4-4"/></svg>; }
