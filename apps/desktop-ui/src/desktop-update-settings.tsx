import { desktopUpdateActionLabel, desktopUpdateChannelLabel, formatUpdateSize, useDesktopUpdate, type DesktopUpdateChannel } from "./desktop-update";
import { Select } from "./select";

export function DesktopUpdateSettingsPage() {
  const { state, loading, busy, actionError, checkSelectedChannel: check, actSelectedChannel: act, selectChannel, pause, setAutoDownload } = useDesktopUpdate();
  const supported = Boolean(window.__CILAI_UPDATER__) && state?.supported !== false;
  const checking = state?.phase === "checking";
  const downloading = state?.phase === "downloading";
  const downloaded = state?.phase === "downloaded";
  const error = actionError ?? state?.error;
  const status = loading ? "正在读取更新状态…"
    : !supported ? "当前运行方式不支持应用内更新。"
    : error || state?.phase === "error" ? "更新未完成，请重试。"
    : checking ? "正在检查新版本…"
    : downloading ? "正在下载更新…"
    : state?.phase === 'paused' ? "下载已暂停，可随时继续。"
    : downloaded ? "下载完成，重启后完成更新。"
    : state?.phase === "available" ? "发现新版本"
    : state?.checkedAt ? `${desktopUpdateChannelLabel(state.channel)}暂无可用更新` : "点击检查更新，获取最新版本。";
  const hasUpdate = state?.availableVersion && ["available", "downloading", "paused", "downloaded", "error"].includes(state.phase);
  const transferred = formatUpdateSize(state?.transferred ?? null);
  const total = formatUpdateSize(state?.total ?? null);
  return <section aria-label="软件更新" className="software-update-settings-page">
    <header><h1>软件更新</h1></header>
    <div className="settings-group">
      <div className="settings-row"><div><span>词来工作台</span><p>当前版本：{state?.currentVersion || "暂未读取"}</p>
        {state ? <p>当前安装：{desktopUpdateChannelLabel(state.currentChannel)}</p> : null}</div></div>
      <div className="settings-row settings-update-channel"><div><label htmlFor="desktop-update-channel">更新通道</label>
        <p>自动检查跟随当前安装版本。跨通道只在此手动检查，切换安装允许版本号降低。</p></div>
        <Select id="desktop-update-channel" value={state?.selectedChannel ?? 'stable'} disabled={!supported || loading || busy || checking || downloading}
          options={[{ value: "stable", label: "正式版" }, { value: "testing", label: "测试版" }]}
          onChange={value => void selectChannel(value as DesktopUpdateChannel)}/>
      </div>
      <div className="settings-row"><div><label id="desktop-update-auto-download">自动下载更新</label>
        <p>启动后自动检查，运行期间每 4 小时检查一次。仅自动下载当前安装通道的更新，由你选择重启。</p>
        <p>使用热点或计费网络时可关闭；正在下载的更新可单独暂停。</p></div>
        <button aria-labelledby="desktop-update-auto-download" type="button" role="switch" className="settings-switch" aria-checked={state?.autoDownload ?? true}
          disabled={!supported || loading || busy} onClick={() => void setAutoDownload(!state?.autoDownload)}><span/></button>
      </div>
      <div className="settings-update-status" aria-live="polite" aria-busy={loading || checking || downloading || busy}>
        <strong>{status}</strong>
        {hasUpdate ? <p>可更新至 {state.availableVersion}（{desktopUpdateChannelLabel(state.channel)}）</p> : null}
        {state?.checkedAt && state.latestVersion && !hasUpdate ? <p>{desktopUpdateChannelLabel(state.channel)}最新版本：{state.latestVersion}</p> : null}
        {state?.releaseName && hasUpdate ? <p>{state.releaseName}</p> : null}
        {state?.releaseNotes && hasUpdate ? <p className="settings-update-notes">{state.releaseNotes}</p> : null}
        {downloading ? <div className="settings-update-progress">
          <span>{Math.round(state.percent ?? 0)}%{transferred && total ? ` · ${transferred} / ${total}` : ""}</span>
          <progress aria-label="更新下载进度" max="100" value={state.percent ?? 0}/>
        </div> : null}
        {error ? <p role="alert" className="settings-error">{error}</p> : null}
        {state?.checkedAt ? <p className="settings-footnote">上次检查：{new Date(state.checkedAt).toLocaleString()}</p> : null}
        <div className="settings-update-actions">
          <button className="secondary" type="button" disabled={!supported || loading || busy || checking || downloading || downloaded || state?.phase === 'paused'} onClick={() => void check()}>{checking ? "正在检查…" : "检查更新"}</button>
          {downloading ? <button className="secondary" type="button" disabled={busy} onClick={() => void pause()}>暂停下载</button> : null}
          {hasUpdate && state ? <button className="primary" type="button" disabled={loading || busy || downloading} onClick={() => void act()}>{busy && downloaded ? "正在重启…" : desktopUpdateActionLabel(state)}</button> : null}
        </div>
      </div>
    </div>
    {downloaded ? <p className="settings-footnote">重启前会保存对话草稿；有 AI 任务、排队消息或文件操作时，请先等待完成或停止任务。</p> : null}
  </section>;
}
