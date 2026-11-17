import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { PageNoticeStack } from "./stack.js";
import type { PageNotice, PageNoticeStackProps } from "./stack.js";
import { createClosedStateRegistry } from "./closed-states.js";

type PresentationMessage = { protocolVersion: 1; documentId: string; revision: number; visible: boolean };
type PresentationChannel = { connect(): Promise<unknown>; onChange(listener: (message: unknown) => void): () => void };
export type ApplicationNoticeVisibility = "connecting" | "visible" | "hidden" | "unavailable";

function presentationMessage(value: unknown): value is PresentationMessage {
  if (!value || typeof value !== "object") return false;
  const state = value as Partial<PresentationMessage>;
  return state.protocolVersion === 1 && typeof state.documentId === "string" && Boolean(state.documentId)
    && Number.isSafeInteger(state.revision) && state.revision! >= 0 && typeof state.visible === "boolean";
}

export type ApplicationNoticeOrigin = {
  view: string;
  sameView(): boolean;
  show(notice: PageNotice, destination?: string): void;
};

export function useApplicationPageNotices(identity: string, view: string, connected: boolean) {
  const [visibility, setVisibility] = useState<ApplicationNoticeVisibility>("connecting");
  const host = useRef({ status: visibility });
  const scope = useRef({ identity, view });
  const connection = useRef({ connected });
  const [queue, setQueue] = useState<{ identity: string; view: string; items: PageNotice[] } | null>(null);
  const closedStates = useRef(createClosedStateRegistry());
  const stateIdentity = useRef(identity);

  useLayoutEffect(() => {
    if (stateIdentity.current !== identity) {
      stateIdentity.current = identity;
      closedStates.current.reset();
    }
    scope.current = { identity, view };
    setQueue((current) => current?.identity === identity && current.view === view ? current : null);
    return () => { scope.current = { identity, view }; };
  }, [identity, view]);
  useLayoutEffect(() => {
    connection.current = { connected };
    if (!connected) setQueue(null);
    return () => { connection.current = { connected: false }; };
  }, [connected]);

  useEffect(() => {
    const channel = (window as Window & { __CILAI_PRESENTATION__?: PresentationChannel }).__CILAI_PRESENTATION__;
    let latest: PresentationMessage | null = null;
    let verified = false;
    let disposed = false;
    const syncVisibility = (status: ApplicationNoticeVisibility) => {
      if (disposed || host.current.status === status) return;
      host.current = { status };
      if (status !== "visible") setQueue(null);
      setVisibility(status);
    };
    if (!channel || typeof channel.connect !== "function" || typeof channel.onChange !== "function") {
      syncVisibility("unavailable");
      return;
    }
    function updateDocumentVisibility() {
      if (verified && latest) syncVisibility(latest.visible && !document.hidden ? "visible" : "hidden");
    }
    const onChange = (value: unknown) => {
      if (disposed) return;
      if (!presentationMessage(value)) { verified = false; syncVisibility("unavailable"); return; }
      if (latest && (latest.documentId !== value.documentId || latest.revision >= value.revision)) return;
      latest = value;
      verified = true;
      updateDocumentVisibility();
    };
    const unsubscribe = channel.onChange(onChange);
    void channel.connect().then(onChange, () => { if (!latest) syncVisibility("unavailable"); });
    document.addEventListener("visibilitychange", updateDocumentVisibility);
    return () => {
      disposed = true;
      host.current = { status: "connecting" };
      unsubscribe();
      document.removeEventListener("visibilitychange", updateDocumentVisibility);
    };
  }, []);

  const begin = useCallback((): ApplicationNoticeOrigin => {
    const origin = scope.current;
    const presentation = host.current;
    const bridge = connection.current;
    const sameView = () => scope.current === origin;
    const isVisible = () => sameView() && bridge.connected && connection.current === bridge
      && host.current === presentation && presentation.status === "visible";
    return {
      view: origin.view,
      sameView,
      show(notice: PageNotice, destination = origin.view) {
        if (!isVisible()) return;
        if (!closedStates.current.admit(notice.id, notice.stateKey)) return;
        setQueue((current) => {
          if (!isVisible()) return current;
          const items = current?.identity === origin.identity && current.view === destination ? current.items : [];
          return { identity: origin.identity, view: destination, items: [notice, ...items.filter(({ id }) => id !== notice.id)] };
        });
      },
    };
  }, []);

  const dismiss = useCallback((id: string, reason?: "close" | "timeout") => {
    closedStates.current.dismissed(id, reason);
    setQueue((current) => current ? { ...current, items: current.items.filter((item) => item.id !== id) } : current);
  }, []);
  const notices = connected && queue?.identity === identity && queue.view === view ? queue.items : [];
  return { begin, dismiss, notices, visibility };
}

export function ApplicationPageNoticeLayer({
  main, appBar, view, notices, active = true, onDismiss, returnFocus,
}: Pick<PageNoticeStackProps, "notices" | "active" | "onDismiss" | "returnFocus"> & {
  main: RefObject<HTMLElement | null>;
  appBar: RefObject<HTMLElement | null>;
  view: string;
}) {
  const [modalOpen, setModalOpen] = useState(false);
  const hasNotices = notices.length > 0;
  useLayoutEffect(() => {
    const content = main.current;
    if (!content || !hasNotices || !active) return;
    const document = content.ownerDocument;
    let frame = 0;
    const update = () => {
      frame = 0;
      setModalOpen([...document.querySelectorAll('.cilai-app-modal-backdrop, [aria-modal="true"]')]
        .some((modal) => modal.getClientRects().length > 0));
    };
    function schedule() { if (!frame) frame = window.requestAnimationFrame(update); }
    update();
    const structure = new MutationObserver(schedule);
    structure.observe(content, { childList: true, subtree: true });
    structure.observe(document.body, { childList: true });
    return () => {
      if (frame) window.cancelAnimationFrame(frame);
      structure.disconnect();
    };
  }, [main, view, hasNotices, active]);
  return <PageNoticeStack anchor={appBar.current} content={main.current} active={active && !modalOpen} notices={notices} onDismiss={onDismiss}
    returnFocus={returnFocus ?? appBar.current?.querySelector<HTMLElement>('button[data-active="true"], button[aria-current="page"], a[aria-current="page"]')} />;
}
