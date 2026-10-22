import { useState } from "react";
import { invoke } from "./host";

export function MacosDiskAccessSettings() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const run = async (action: "settings" | "application") => {
    if (busy) return;
    setBusy(true); setError("");
    try {
      if (action === "settings") await invoke("macos_disk_access_open_settings");
      else await invoke("macos_disk_access_reveal_application");
    } catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setBusy(false); }
  };
  return <section className="disk-access-settings-page" aria-label="磁盘访问">
    <header><h1>磁盘访问</h1></header>
    <div className="settings-group"><div className="settings-row"><div><span>完全磁盘访问</span><p>在系统设置中添加并开启词来工作台，减少逐目录访问提示。</p></div></div></div>
    <div className="settings-update-actions">
      <button type="button" className="primary" disabled={busy} onClick={() => void run("settings")}>打开设置</button>
      <button type="button" disabled={busy} onClick={() => void run("application")}>显示应用</button>
    </div>
    <p className="settings-footnote">完成设置后，请完全退出并重开词来工作台。其他系统隐私权限需单独设置。</p>
    {error ? <p className="settings-error" role="alert">{error}</p> : null}
  </section>;
}
