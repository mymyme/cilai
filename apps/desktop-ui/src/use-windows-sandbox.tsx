import { createContext, useContext, useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from "react";
import { aiErrorMessage, loadAiWindowsSandboxStatus, setupAiWindowsSandbox, type AiWindowsSandboxStatus } from "./ai-runtime";

interface Snapshot {
  status: AiWindowsSandboxStatus | null;
  loading: boolean;
  settingUp: boolean;
  error: string | null;
}
/** Initialization display state, scoped to the account and native server.
 * Every real run continues to perform its own native execution preflight.
 * 启动后的状态检查发现未初始化或凭据失效时，自动发起一次初始化（系统会弹出管理员批准）；
 * 每个账户与服务范围只自动尝试一次，取消或失败后保留明确状态，不反复请求提升权限。
 */
class SandboxStatus {
  private snapshot: Snapshot = {status:null,loading:false,settingUp:false,error:null};
  private listeners = new Set<() => void>();
  private pending: Promise<void> | null = null;
  private pendingKind: "read" | "setup" | null = null;
  private autoRepairAttempted = false;
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private update(change: Partial<Snapshot>) {
    this.snapshot = {...this.snapshot,...change};
    this.listeners.forEach(listener => listener());
  }
  private perform(kind: "read" | "setup", operation: () => Promise<AiWindowsSandboxStatus>) {
    this.pendingKind = kind;
    this.update({loading:kind === "read",settingUp:kind === "setup",error:null});
    this.pending = Promise.resolve().then(operation).then(status => {
      this.update({status,error:null});
    }).catch(reason => {
      const error = aiErrorMessage(reason);
      this.update(kind === "read" ? {status:{supported:true,status:"error",errorCode:null,errorMessage:error},error} : {error});
    }).finally(() => {
      this.pending = null;
      this.pendingKind = null;
      this.update({loading:false,settingUp:false});
    });
    return this.pending;
  }
  read = (force = false): Promise<void> => {
    if (this.pending) return this.pending;
    if (!force && this.snapshot.status !== null) return Promise.resolve();
    return this.perform("read",loadAiWindowsSandboxStatus).then(this.repairOnce);
  };
  private repairOnce = (): Promise<void> => {
    const state = this.snapshot.status?.status;
    if (this.autoRepairAttempted || (state !== "notConfigured" && state !== "updateRequired")) return Promise.resolve();
    this.autoRepairAttempted = true;
    return this.setup(null);
  };
  setup = (workspaceRoot: string | null): Promise<void> => {
    if (this.pendingKind === "setup") return this.pending!;
    if (this.pending) return this.pending.then(() => this.setup(workspaceRoot));
    return this.perform("setup",() => setupAiWindowsSandbox(workspaceRoot));
  };
}
const SandboxContext = createContext<{store:SandboxStatus;enabled:boolean} | null>(null);
export function WindowsSandboxProvider(props: { scopeKey: string; enabled?: boolean; children: ReactNode }) {
  const [scope,setScope] = useState(() => ({key:props.scopeKey,store:new SandboxStatus()}));
  if (scope.key !== props.scopeKey) setScope({key:props.scopeKey,store:new SandboxStatus()});
  const store = scope.store;
  const enabled = props.enabled !== false;
  const value = useMemo(() => ({store,enabled}),[store,enabled]);
  return <SandboxContext.Provider value={value}>{props.children}</SandboxContext.Provider>;
}
export function useWindowsSandbox(hostPlatform: string | null) {
  const context = useContext(SandboxContext);
  if (!context) throw new Error("windows_sandbox_provider_required");
  const {store,enabled} = context;
  const snapshot = useSyncExternalStore(store.subscribe,store.getSnapshot);
  useEffect(() => {
    if (hostPlatform === "windows" && enabled) void store.read();
  },[hostPlatform,enabled,store]);
  return {...snapshot,
    available:enabled,
    loading:hostPlatform === "windows" && (snapshot.loading || snapshot.status === null),
    refresh:() => enabled ? store.read(true) : Promise.resolve(),
    setup:(root: string | null) => enabled ? store.setup(root) : Promise.resolve(),
  };
}
