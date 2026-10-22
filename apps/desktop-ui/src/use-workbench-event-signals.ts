import { useEffect, useRef, useState } from "react";
import type { EventReplayResult } from "./generated/protocol";
import { rpc } from "./app-server-rpc";
import type { MessageSink } from "./app-message";
import { latestClientUpdateSignal } from "./desktop-update";

const pollInterval = 3_000;

export interface WorkbenchEventSignals {
  nextSequence: number;
  clientUpdateSignal: number;
}

export function isEventSignalTimeout(error: unknown): boolean {
  return String(error).includes("App Server request timed out: event/replay");
}

export function eventSignalNotice(error: unknown): string {
  return isEventSignalTimeout(error)
    ? "工作台状态同步延迟，正在重试。"
    : "工作台状态暂时无法同步，正在重试。";
}

export function useWorkbenchEventSignals(options: {
  active: boolean;
  selectedWorkspaceId: string | null;
  serverInstanceId: string | null;
  onSelectedWorkspaceChanged: () => void;
  onMessage: MessageSink;
}): WorkbenchEventSignals {
  const { active, selectedWorkspaceId, serverInstanceId, onSelectedWorkspaceChanged, onMessage } = options;
  const [signals, setSignals] = useState<WorkbenchEventSignals>({ nextSequence: 0, clientUpdateSignal: 0 });
  const cursor = useRef(0);
  const replayServer = useRef<string | null>(null);
  const clientStartedAt = useRef(Date.now());
  const clientUpdateSequence = useRef(0);
  const changedRef = useRef(onSelectedWorkspaceChanged);
  changedRef.current = onSelectedWorkspaceChanged;
  const messageRef = useRef(onMessage);
  messageRef.current = onMessage;

  useEffect(() => {
    if (!active) return;
    if (replayServer.current !== serverInstanceId) {
      replayServer.current = serverInstanceId;
      cursor.current = 0;
      clientUpdateSequence.current = 0;
      setSignals({ nextSequence: 0, clientUpdateSignal: 0 });
    }
    let cancelled = false;
    let timer: number | null = null;
    let replayDelayReported = false;

    const replay = async () => {
      try {
        let hasMore = true;
        let touchedSelected = false;
        let nextClientUpdateSequence = clientUpdateSequence.current;
        while (hasMore && !cancelled) {
          const result = await rpc<EventReplayResult>("event/replay", { afterSequence: cursor.current || null, limit: 100 });
          replayDelayReported = false;
          cursor.current = result.nextSequence;
          touchedSelected ||= Boolean(selectedWorkspaceId && result.events.some((event) => event.workspaceId === selectedWorkspaceId));
          nextClientUpdateSequence = Math.max(
            nextClientUpdateSequence,
            latestClientUpdateSignal(result.events, clientStartedAt.current),
          );
          hasMore = result.hasMore;
        }
        if (cancelled) return;
        clientUpdateSequence.current = nextClientUpdateSequence;
        setSignals((current) => current.nextSequence === cursor.current && current.clientUpdateSignal === nextClientUpdateSequence
          ? current
          : { nextSequence: cursor.current, clientUpdateSignal: nextClientUpdateSequence });
        if (touchedSelected) changedRef.current();
      } catch (error) {
        if (!cancelled && !replayDelayReported) {
          replayDelayReported = true;
          messageRef.current(eventSignalNotice(error));
        }
      } finally {
        if (!cancelled) timer = window.setTimeout(() => void replay(), pollInterval);
      }
    };

    void replay();
    return () => { cancelled = true; if (timer !== null) window.clearTimeout(timer); };
  }, [active, selectedWorkspaceId, serverInstanceId]);

  return signals;
}
