import { useEffect, useRef, useState } from "react";
import type { DurableEventItem } from "./generated/protocol";
import { flushConversationSessions } from "./conversation-session";

export type DesktopUpdatePhase = "idle" | "checking" | "available" | "downloading" | "paused" | "downloaded" | "error";
export type DesktopUpdateChannel = 'stable' | 'testing';
export const desktopUpdateChannelLabel = (channel: DesktopUpdateChannel) => channel === 'testing' ? '测试版' : '正式版';

export interface DesktopUpdateState {
  supported: boolean;
  autoDownload: boolean;
  channel: DesktopUpdateChannel;
  selectedChannel: DesktopUpdateChannel;
  currentChannel: DesktopUpdateChannel;
  phase: DesktopUpdatePhase;
  currentVersion: string;
  availableVersion: string | null;
  latestVersion: string | null;
  releaseName: string | null;
  releaseNotes: string | null;
  percent: number | null;
  transferred: number | null;
  total: number | null;
  bytesPerSecond: number | null;
  error: string | null;
  checkedAt: number | null;
  trigger: string | null;
}

interface DesktopUpdaterBridge {
  getState: () => Promise<DesktopUpdateState>;
  check: () => Promise<DesktopUpdateState>;
  checkSelectedChannel: () => Promise<DesktopUpdateState>;
  selectChannel: (channel: DesktopUpdateChannel) => Promise<DesktopUpdateState>;
  download: () => Promise<DesktopUpdateState>;
  pause: () => Promise<DesktopUpdateState>;
  setAutoDownload: (enabled: boolean) => Promise<DesktopUpdateState>;
  install: () => Promise<null>;
  onState: (handler: (state: DesktopUpdateState) => void) => () => void;
}

declare global {
  interface Window {
    __CILAI_UPDATER__?: DesktopUpdaterBridge;
  }
}

export const clientUpdatePublishedEvent = "workbench.client_update_published";

export function latestClientUpdateSignal(events: DurableEventItem[], since: number): number {
  return events.reduce(
    (latest, event) => event.eventType === clientUpdatePublishedEvent && event.occurredAt >= since
      ? Math.max(latest, event.sequence)
      : latest,
    0,
  );
}

function UpdateIcon(props: { progress: number | null }) {
  if (props.progress !== null) {
    const progress = Math.max(0, Math.min(100, props.progress));
    const dash = `${progress} ${100 - progress}`;
    return <svg aria-hidden="true" className="desktop-update-progress-icon" viewBox="0 0 24 24">
      <circle className="desktop-update-progress-track" cx="12" cy="12" pathLength="100" r="8.5"/>
      <circle className="desktop-update-progress-value" cx="12" cy="12" pathLength="100" r="8.5" strokeDasharray={dash}/>
      <path d="M12 7.5v7m0 0 2.6-2.6M12 14.5l-2.6-2.6"/>
    </svg>;
  }
  return <svg aria-hidden="true" viewBox="0 0 24 24">
    <path d="M12 3.75v10.5m0 0 3.75-3.75M12 14.25 8.25 10.5M5.25 18.25h13.5"/>
  </svg>;
}

function statusTitle(state: DesktopUpdateState) {
  if (state.phase === "downloading") return "正在下载";
  if (state.phase === "paused") return "下载已暂停";
  if (state.phase === "downloaded") return "可以更新";
  if (state.phase === "error") return "更新失败";
  return "发现新版本";
}

export function desktopUpdateActionLabel(state: DesktopUpdateState) {
  if (state.phase === "downloading") return `正在下载 ${Math.round(state.percent ?? 0)}%`;
  if (state.phase === "paused") return "继续下载";
  if (state.phase === "downloaded") return "重启更新";
  if (state.phase === "error") return state.availableVersion ? "重试下载" : "重新检查";
  return "下载更新";
}

export function formatUpdateSize(value: number | null) {
  if (value === null || value <= 0) return null;
  if (value >= 1024 * 1024 * 1024) return `${(value / 1024 / 1024 / 1024).toFixed(1)} GB`;
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB`;
  return `${Math.ceil(value / 1024)} KB`;
}

export function useDesktopUpdate() {
  const [state, setState] = useState<DesktopUpdateState | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [loading, setLoading] = useState(Boolean(window.__CILAI_UPDATER__));
  const [busy, setBusy] = useState(false);
  const acting = useRef(false);
  const active = useRef(false);

  useEffect(() => {
    const bridge = window.__CILAI_UPDATER__;
    if (!bridge) return;
    active.current = true;
    let subscribed = true;
    let receivedState = false;
    const dispose = bridge.onState((value) => {
      if (!subscribed) return;
      receivedState = true;
      setState(value);
      setActionError(null);
      setLoading(false);
    });
    void bridge.getState().then((value) => {
      if (subscribed && !receivedState) setState(value);
    }).catch((error) => {
      if (subscribed && !receivedState) setActionError(String(error));
    }).finally(() => { if (subscribed) setLoading(false); });
    return () => { subscribed = false; active.current = false; dispose(); };
  }, []);

  const perform = async (checkOnly: boolean, channel?: DesktopUpdateChannel, settings = false) => {
    const bridge = window.__CILAI_UPDATER__;
    if (!bridge || acting.current || state?.supported === false
      || state?.phase === "checking" || state?.phase === "downloading"
      || (checkOnly && !channel && state?.phase === "downloaded")) return;
    acting.current = true;
    setBusy(true);
    setActionError(null);
    try {
      if (channel) {
        const next = await bridge.selectChannel(channel);
        if (active.current) setState(next);
      } else if (!checkOnly && state?.phase === "downloaded") {
        await flushConversationSessions();
        await bridge.install();
      } else {
        const next = checkOnly || !state || (state.phase === "error" && !state.availableVersion)
          ? await (settings ? bridge.checkSelectedChannel() : bridge.check()) : await bridge.download();
        if (active.current) setState(next);
      }
    } catch (error) {
      if (active.current) setActionError(String(error));
    } finally {
      acting.current = false;
      if (active.current) setBusy(false);
    }
  };
  const change = async (action: 'pause' | 'autoDownload', enabled?: boolean) => {
    const bridge = window.__CILAI_UPDATER__;
    if (!bridge || acting.current || !state?.supported) return;
    acting.current = true;
    setBusy(true);
    setActionError(null);
    try {
      const next = action === 'pause' ? await bridge.pause() : await bridge.setAutoDownload(Boolean(enabled));
      if (active.current) setState(next);
    } catch (error) { if (active.current) setActionError(String(error)); }
    finally { acting.current = false; if (active.current) setBusy(false); }
  };
  return { state, actionError, loading, busy, check: () => perform(true), act: () => perform(false),
    pause: () => change('pause'), setAutoDownload: (enabled: boolean) => change('autoDownload', enabled),
    checkSelectedChannel: () => perform(true, undefined, true), actSelectedChannel: () => perform(false, undefined, true),
    selectChannel: (channel: DesktopUpdateChannel) => perform(true, channel) };
}

export function DesktopUpdateControl(props: { platformSignal: number }) {
  const { state, actionError, busy, act, pause } = useDesktopUpdate();
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const lastPlatformSignal = useRef(0);

  useEffect(() => {
    const bridge = window.__CILAI_UPDATER__;
    if (!bridge || props.platformSignal <= lastPlatformSignal.current) return;
    lastPlatformSignal.current = props.platformSignal;
    void bridge.check().catch(() => undefined);
  }, [props.platformSignal]);

  useEffect(() => {
    if (!open) return;
    const closeOnOutside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("pointerdown", closeOnOutside);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", closeOnOutside);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  const visible = Boolean(
    state?.supported
    && (state.availableVersion || state.phase === "error")
    && ["available", "downloading", "paused", "downloaded", "error"].includes(state.phase),
  );
  if (!visible || !state) return null;

  const downloading = state.phase === "downloading";
  const progress = downloading ? state.percent ?? 0 : null;
  const total = formatUpdateSize(state.total);
  const transferred = formatUpdateSize(state.transferred);
  const triggerClassName = [
    "desktop-update-trigger",
    state.phase === "downloading" ? "desktop-update-downloading" : "",
    state.phase === "downloaded" ? "desktop-update-downloaded" : "",
  ].filter(Boolean).join(" ");

  return <div className="desktop-update-control" ref={root}>
    <button
      aria-expanded={open}
      aria-haspopup="dialog"
      aria-label={state.phase === "downloaded" ? "重启更新词来工作台" : state.phase === "error" ? "词来工作台更新失败" : "词来工作台有新版本"}
      className={triggerClassName}
      onClick={() => setOpen((current) => !current)}
      title={state.phase === "downloaded" ? "重启更新" : state.phase === "error" ? "更新失败" : `发现新版本 ${state.availableVersion}`}
      type="button">
      <UpdateIcon progress={progress}/><i aria-hidden="true"/>
    </button>
    {open ? <section aria-label="客户端更新" className="desktop-update-popover" role="dialog">
      <header><span><strong>{statusTitle(state)}</strong>{state.availableVersion ? <small>{desktopUpdateChannelLabel(state.channel)} {state.availableVersion}</small> : null}</span></header>
      {state.releaseName ? <p className="desktop-update-name">{state.releaseName}</p> : null}
      {state.releaseNotes ? <p className="desktop-update-notes">{state.releaseNotes}</p> : null}
      {downloading ? <div className="desktop-update-download">
        <span><b>{Math.round(state.percent ?? 0)}%</b><small>{transferred && total ? `${transferred} / ${total}` : "正在下载"}</small></span>
        <progress max="100" value={state.percent ?? 0}/>
      </div> : null}
      {state.error || actionError ? <p className="desktop-update-error">{actionError ?? state.error}</p> : null}
      <footer>
        <button className="secondary" onClick={() => setOpen(false)} type="button">稍后</button>
        {downloading ? <button className="secondary" disabled={busy} onClick={() => void pause()} type="button">暂停下载</button> : null}
        <button className="primary" disabled={downloading || busy} onClick={() => void act()} type="button">{busy && state.phase === "downloaded" ? "正在重启…" : desktopUpdateActionLabel(state)}</button>
      </footer>
    </section> : null}
  </div>;
}
