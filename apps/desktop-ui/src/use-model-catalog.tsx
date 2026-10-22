import "./model-catalog.css";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { aiErrorMessage, type AiModelCatalogStatus, type AiModelInfo } from "./ai-runtime";
import { invoke, listen } from "./host";

export interface ModelCatalogSnapshot {
  generation: number;
  revision: number;
  status: AiModelCatalogStatus | null;
  checkedAt: number | null;
  revalidateAfter: number | null;
  error: string | null;
}
export interface ModelCatalogUpdate { profileId: string; catalog: ModelCatalogSnapshot }
interface ModelCatalog {
  status: AiModelCatalogStatus | null;
  models: AiModelInfo[];
  displayModels: AiModelInfo[];
  loading: boolean;
  loaded: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  refreshOnOpen: () => void;
  ensureLoaded: () => void;
}
const emptyModels: AiModelInfo[] = [];
const CatalogContext = createContext<ModelCatalog | null>(null);
type ReadReason = "startup" | "picker" | "retry";

type ProviderProps = { profileId: string; enabled?: boolean; children: ReactNode };
export function ModelCatalogProvider(props: ProviderProps) {
  return <ModelCatalogSession key={props.profileId} {...props}/>;
}

function ModelCatalogSession(props: ProviderProps) {
  const [snapshot, setSnapshot] = useState<ModelCatalogSnapshot | null>(null);
  const [displayModels, setDisplayModels] = useState<AiModelInfo[]>(emptyModels);
  const [requesting, setRequesting] = useState(false);
  const [requestError, setRequestError] = useState<string | null>(null);
  const [subscriptionError, setSubscriptionError] = useState<string | null>(null);
  const [subscriptionAttempt, setSubscriptionAttempt] = useState(0);
  const current = useRef<ModelCatalogSnapshot | null>(null);
  const inFlight = useRef<Promise<void> | null>(null);
  const lifecycle = useRef(0);
  const requested = useRef(false);
  const pendingUpdate = useRef<ModelCatalogUpdate | null>(null);
  const enabled = props.enabled !== false;
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;
  const accept = useCallback((update: ModelCatalogUpdate) => {
    if (update.profileId !== props.profileId) return;
    const previous = current.current;
    const incoming = update.catalog;
    if (previous && (incoming.generation < previous.generation || (incoming.generation === previous.generation && incoming.revision < previous.revision))) return;
    const sameScope = previous?.generation === incoming.generation;
    const status = sameScope && previous?.status && JSON.stringify(previous.status) === JSON.stringify(incoming.status)
      ? previous.status : incoming.status;
    const next = { ...incoming, status };
    current.current = next;
    setSnapshot(next);
    setRequestError(null);
    if (!sameScope && !incoming.error) setDisplayModels(status?.models ?? emptyModels);
    else if (status) setDisplayModels(status.models);
  }, [props.profileId]);
  const read = useCallback((reason: ReadReason) => {
    if (!enabledRef.current) return Promise.resolve();
    if (inFlight.current) return inFlight.current;
    requested.current = true;
    const epoch = lifecycle.current;
    const before = current.current;
    setRequesting(true);
    const pending = (async () => {
      try {
        const update = await invoke<ModelCatalogUpdate>("ai_model_catalog_read", { reason });
        if (lifecycle.current === epoch) {
          accept(update);
          if (pendingUpdate.current) { accept(pendingUpdate.current); pendingUpdate.current = null; }
        }
      } catch (cause) {
        if (lifecycle.current === epoch && current.current === before) {
          setRequestError(aiErrorMessage(cause));
        }
      } finally {
        if (lifecycle.current === epoch) {
          inFlight.current = null;
          setRequesting(false);
        }
      }
    })();
    inFlight.current = pending;
    return pending;
  }, [accept]);
  const refresh = useCallback(() => {
    if (subscriptionError) { setSubscriptionAttempt((value) => value + 1); return Promise.resolve(); }
    return read("retry");
  }, [read, subscriptionError]);
  const refreshOnOpen = useCallback(() => { void read("picker"); }, [read]);
  const ensureLoaded = useCallback(() => {
    if (!requested.current) void read("startup");
  }, [read]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<ModelCatalogUpdate>("model-catalog-changed", (update) => {
      if (disposed || update.profileId !== props.profileId) return;
      if (!current.current) {
        const queued = pendingUpdate.current?.catalog;
        if (!queued || update.catalog.generation > queued.generation || (update.catalog.generation === queued.generation && update.catalog.revision >= queued.revision)) pendingUpdate.current = update;
        return;
      }
      accept(update);
      if (current.current.generation === update.catalog.generation && !current.current.status && !current.current.error && requested.current) {
        void (async () => {
          await inFlight.current;
          if (!disposed && !current.current?.status && !current.current?.error) void read("startup");
        })();
      }
    }).then((stop) => {
      if (disposed) stop();
      else {
        unlisten = stop;
        setSubscriptionError(null);
        if (requested.current) void read("startup");
      }
    }).catch((cause) => { if (!disposed) setSubscriptionError(aiErrorMessage(cause)); });
    return () => {
      disposed = true;
      lifecycle.current += 1;
      inFlight.current = null;
      unlisten?.();
    };
  }, [accept, read, subscriptionAttempt]);
  useEffect(() => { if (enabled) ensureLoaded(); }, [enabled, ensureLoaded]);
  const valid = enabled && Boolean(snapshot?.status) && !snapshot?.error && !requestError && !subscriptionError;
  const error = subscriptionError ?? requestError ?? (snapshot?.error ? aiErrorMessage(snapshot.error) : null);
  const value = useMemo<ModelCatalog>(() => ({
    status: valid ? snapshot!.status : null,
    models: valid ? snapshot!.status!.models : emptyModels,
    displayModels: enabled ? displayModels : emptyModels,
    loading: enabled && requesting && !snapshot?.status && !error,
    loaded: valid,
    error,
    refresh,
    refreshOnOpen,
    ensureLoaded,
  }), [valid, snapshot, enabled, displayModels, requesting, error, refresh, refreshOnOpen, ensureLoaded]);
  return <CatalogContext.Provider value={value}>{props.children}</CatalogContext.Provider>;
}

export function useModelCatalog(enabled: boolean) {
  const catalog = useContext(CatalogContext);
  if (!catalog) throw new Error("model_catalog_provider_required");
  useEffect(() => { if (enabled) catalog.ensureLoaded(); }, [enabled, catalog.ensureLoaded]);
  return catalog;
}

export function ModelCatalogNotice({ catalog, variant }: { catalog: Pick<ModelCatalog, "error" | "refresh">; variant?: "conversation" }) {
  return catalog.error ? <div className={`model-catalog-error${variant === "conversation" ? " model-catalog-error-conversation" : ""}`} role="alert">{variant === "conversation" ? <svg aria-hidden="true" viewBox="0 0 20 20"><circle cx="10" cy="10" r="7.5"/><path d="M10 5.8v5M10 14h.01"/></svg> : null}<span>{catalog.error}</span><button type="button" onClick={() => void catalog.refresh()}>重试</button></div> : null;
}
