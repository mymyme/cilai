import { useEffect, useState } from "react";
import { invoke } from "./host";
import { aiErrorMessage } from "./ai-runtime";
import { CheckIcon, ConfirmIcon, InfoIcon, RunsIcon } from "./icons";
import { ConfirmDialog } from "./confirm-dialog";
import "./test-diagnostics.css";

interface DiagnosticSettings { enabled: boolean; error: string | null; maxBytes: number }
interface DiagnosticPreview { previewId: string; digest: string; expiresAt: number; bytes: number; text: string }

export function ExportDiagnosticsButton(props: { primary?: boolean } = {}) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [failed, setFailed] = useState(false);
  const [preview, setPreview] = useState<DiagnosticPreview | null>(null);
  const previewLogs = async () => {
    setBusy(true); setMessage(""); setFailed(false);
    try {
      const value = await invoke<DiagnosticPreview>("test_diagnostics_preview");
      if (!value || !/^[a-f0-9-]{36}$/.test(value.previewId) || !/^[a-f0-9]{64}$/.test(value.digest)
        || !Number.isSafeInteger(value.bytes) || value.bytes < 0 || value.bytes > 2 * 1024 * 1024
        || typeof value.text !== "string" || !Number.isSafeInteger(value.expiresAt)) throw new Error("诊断预览无效，请重新读取。");
      setPreview(value);
    } catch (error) { setFailed(true); setMessage(aiErrorMessage(error)); }
    finally { setBusy(false); }
  };
  const exportLogs = async () => {
    if (!preview || busy) return;
    setBusy(true); setMessage(""); setFailed(false);
    try {
      if (await invoke<boolean>("test_diagnostics_export", { previewId: preview.previewId, digest: preview.digest })) {
        setPreview(null);
        setMessage("诊断已导出。");
      }
    } catch (error) { setFailed(true); setMessage(aiErrorMessage(error)); setPreview(null); }
    finally { setBusy(false); }
  };
  return <div className="diagnostics-export-control">
    <button className={`diagnostics-button${props.primary ? " diagnostics-button-primary" : ""}`} type="button" disabled={busy} onClick={() => void previewLogs()}>
      <DownloadIcon/>{busy ? "正在读取…" : "导出诊断"}
    </button>
    {message ? <p className="diagnostics-result" data-failed={failed} role={failed ? "alert" : "status"}>
      {failed ? <InfoIcon/> : <CheckIcon/>}<span>{message}</span>
    </p> : null}
    {preview ? <ConfirmDialog title="诊断预览" confirmLabel="选择保存位置" busy={busy}
      description={<><p>仅保存以下内容，不自动上传。</p><pre className="diagnostics-preview">{preview.text}</pre></>}
      onCancel={() => setPreview(null)} onConfirm={() => void exportLogs()}/> : null}
  </div>;
}

export function TestDiagnosticsSettings() {
  const [settings, setSettings] = useState<DiagnosticSettings | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    let current = true;
    setError(null);
    void invoke<DiagnosticSettings>("test_diagnostics_settings")
      .then(result => { if (current) setSettings(result); })
      .catch(caught => { if (current) setError(aiErrorMessage(caught)); });
    return () => { current = false; };
  }, [retry]);
  const toggle = async () => {
    if (!settings || busy) return;
    setBusy(true); setError(null);
    try { setSettings(await invoke<DiagnosticSettings>("test_diagnostics_set_enabled", {enabled: !settings.enabled})); }
    catch (caught) { setError(aiErrorMessage(caught)); }
    finally { setBusy(false); }
  };
  const detail = error ?? settings?.error;
  const stateLabel = busy ? "正在保存…" : settings ? settings.enabled ? "已开启" : "已关闭" : error ? "读取失败" : "正在读取…";
  return <section className="diagnostics-settings-page" aria-label="诊断设置">
    <div className="diagnostics-settings-inner">
      <header className="diagnostics-page-heading">
        <div><h1>诊断</h1></div>
        <span className="diagnostics-local-badge"><ComputerIcon/>仅在本机</span>
      </header>

      <section className="diagnostics-card" aria-label="日志记录">
        <header className="diagnostics-card-heading">
          <span className="diagnostics-card-icon"><RunsIcon/></span>
          <div><h2>记录错误</h2></div>
          <div className="diagnostics-recording-state">
            <span data-enabled={Boolean(settings?.enabled)} aria-live="polite">{stateLabel}</span>
            <button className="diagnostics-switch" type="button" role="switch" aria-label="记录错误" aria-checked={settings?.enabled ?? false} disabled={!settings || busy} onClick={() => void toggle()}><span/></button>
          </div>
        </header>
        <dl className="diagnostics-recording-details">
          <div><dt>记录范围</dt><dd>配置、网络、认证、协议、模型、工具、权限和保存错误</dd></div>
          <div><dt>保存方式</dt><dd>本机循环保存，最多约 1 MB</dd></div>
        </dl>
        <p className="diagnostics-card-note">设置立即生效，重启后保留。关闭后停止新增记录，已有记录保留。</p>
        {detail ? <div className="diagnostics-settings-error" role="alert"><InfoIcon/><span>{detail}</span>
          <button className="diagnostics-button" type="button" onClick={() => setRetry(value => value + 1)}>重新读取</button></div> : null}
      </section>

      <section className="diagnostics-card diagnostics-export-card" aria-label="诊断导出">
        <header className="diagnostics-card-heading">
          <span className="diagnostics-card-icon"><DownloadIcon/></span>
          <div><h2>导出诊断</h2></div>
        </header>
        <ExportDiagnosticsButton primary/>
        <div className="diagnostics-export-details">
          <div><CheckIcon/><div><h3>预览后保存</h3><p>包含系统版本、错误分类、时间、次数和运行编号；未知错误明确标记未分类。检查成功不代表模型或业务任务通过。</p></div></div>
          <div><ConfirmIcon/><div><h3>保护工作内容</h3><p>不包含会话正文、文件内容、请求参数或登录凭据。没有已记录的错误时，文件中会明确说明。</p></div></div>
        </div>
      </section>
      <p className="diagnostics-page-note"><ConfirmIcon/>仅在本机保存。启动失败时也可导出。</p>
    </div>
  </section>;
}

function DownloadIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M12 3v12m-4-4 4 4 4-4M5 16v4h14v-4"/></svg>; }
function ComputerIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/></svg>; }
