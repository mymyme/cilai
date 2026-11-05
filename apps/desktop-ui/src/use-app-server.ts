import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "./host";
import type { MessageSink } from "./app-message";

export type AppServerPhase = "stopped" | "starting" | "running" | "stopping" | "failed";

export interface AppServerStatus {
  phase: AppServerPhase;
  pid: number | null;
  serverInstanceId: string | null;
  protocolVersion: string | null;
  lastExitCode: number | null;
  errorReason: string | null;
  runtimeInitializing?: boolean;
  runtimeInitializationError?: string | null;
  personalDataState?: "idle" | "applying" | "restart_required";
  stderrTail: string[];
  unsolicitedMessageCount: number;
  localIpcAvailable: boolean;
  connectionMode: "owned" | "attached" | null;
  serverLifecycleMode: "owned" | "transient" | null;
}

/**
 * 本地服务进程的状态。前端这一侧没有别的办法知道进程是死是活——它由 Electron 宿主托管，
 * 崩溃时不会有事件推过来，所以只能定时问。
 *
 * 两秒一次是权衡后的值：服务重启通常在一两秒内完成，问得再密只是徒增 IPC，
 * 再稀就会让"启动中"这个状态在界面上停留得比实际久。
 */
export function useAppServer(onMessage: MessageSink) {
  const [status, setStatus] = useState<AppServerStatus | null>(null);

  const generation = useRef(0);
  const restarting = useRef(false);

  useEffect(() => {
    let active = true;
    let reading = false;
    const refresh = async () => {
      if (reading || restarting.current) return;
      reading = true;
      const current = generation.current;
      try {
        const next = await invoke<AppServerStatus>("app_server_status");
        if (active && generation.current === current) setStatus(next);
      } catch (error) {
        if (active && generation.current === current) onMessage(String(error), "error");
      } finally {
        reading = false;
      }
    };
    void refresh();
    const timer = window.setInterval(refresh, 2_000);
    return () => { active = false; window.clearInterval(timer); };
  }, [onMessage]);

  const restart = useCallback(() => {
    if (restarting.current) return;
    restarting.current = true;
    const current = ++generation.current;
    setStatus(previous => previous && ({ ...previous, runtimeInitializing: true, runtimeInitializationError: null }));
    void invoke<AppServerStatus>("app_server_restart").then(next => {
      if (generation.current === current) setStatus(next);
    }).catch(error => {
      if (generation.current !== current) return;
      setStatus(previous => previous && ({ ...previous, runtimeInitializing: false, runtimeInitializationError: String(error) }));
      onMessage(String(error), "error");
    }).finally(() => { restarting.current = false; });
  }, [onMessage]);

  return { status, restart };
}
