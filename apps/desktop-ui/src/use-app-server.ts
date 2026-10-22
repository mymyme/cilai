import { useCallback, useEffect, useState } from "react";
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

  useEffect(() => {
    const refresh = () => void invoke<AppServerStatus>("app_server_status")
      .then(setStatus)
      .catch((error) => onMessage(String(error), "error"));
    refresh();
    const timer = window.setInterval(refresh, 2_000);
    return () => window.clearInterval(timer);
  }, [onMessage]);

  const restart = useCallback(() => {
    void invoke<AppServerStatus>("app_server_restart").then(setStatus)
      .catch((error) => onMessage(String(error), "error"));
  }, [onMessage]);

  return { status, restart };
}
