import { useEffect, useRef, useState } from "react";
import type { ModelExecutionSnapshot, ModelSelection } from "./generated/protocol";
import { ConfirmDialog } from "./confirm-dialog";
import { invoke } from "./host";

export interface ModelHistoryTransferConsent {
  connectionId: string; configRevision: number; authGeneration: number; modelId: string;
  runtimeKind: "workbench" | "application"; snapshotDigest: string; historyDigest: string; confirmedAt: number;
}
interface TransferPreview extends Omit<ModelHistoryTransferConsent, "confirmedAt"> {
  messageCount: number; attachmentCount: number; serviceName: string; baseUrl: string | null;
  historyScope: string; mayIncurCost: boolean;
}
interface ExecutionPreflight { snapshot: ModelExecutionSnapshot; snapshotDigest: string; historyTransfer: TransferPreview | null }
type Approval = { allowed: true; consent: ModelHistoryTransferConsent | null } | { allowed: false };

export function useModelHistoryTransfer(scopeKey: string) {
  const [preview, setPreview] = useState<TransferPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const active = useRef(false);
  const generation = useRef(0);
  const resolve = useRef<((approved: boolean) => void) | null>(null);
  useEffect(() => {
    setPreview(null); setBusy(false); active.current = false;
    return () => { generation.current += 1; resolve.current?.(false); resolve.current = null; };
  }, [scopeKey]);
  const approve = async (selection: ModelSelection | null | undefined, conversationId: string | null): Promise<Approval> => {
    if (!selection) return { allowed: true, consent: null };
    if (active.current) throw new Error("model_history_preflight_in_progress");
    active.current = true; setBusy(true);
    const epoch = generation.current;
    try {
      const result = await invoke<ExecutionPreflight>("model_execution_preflight", { runtimeKind: "workbench", request: {
        purpose: "foreground", bindingId: null, selection, conversationId,
      } });
      if (epoch !== generation.current) return { allowed: false };
      if (result.snapshot.connectionId !== selection.connectionId || result.snapshot.modelId !== selection.modelId) throw new Error("model_execution_snapshot_changed");
      const transfer = result.historyTransfer;
      if (!transfer) return { allowed: true, consent: null };
      if (transfer.connectionId !== selection.connectionId || transfer.modelId !== selection.modelId || transfer.snapshotDigest !== result.snapshotDigest) throw new Error("model_execution_snapshot_changed");
      setPreview(transfer);
      const approved = await new Promise<boolean>((complete) => { resolve.current = complete; });
      if (!approved || epoch !== generation.current) return { allowed: false };
      return { allowed: true, consent: {
        connectionId: transfer.connectionId, configRevision: transfer.configRevision,
        authGeneration: transfer.authGeneration, modelId: transfer.modelId, runtimeKind: transfer.runtimeKind,
        snapshotDigest: transfer.snapshotDigest, historyDigest: transfer.historyDigest, confirmedAt: Math.floor(Date.now() / 1000),
      } };
    } finally {
      if (epoch === generation.current) { setPreview(null); resolve.current = null; active.current = false; setBusy(false); }
    }
  };
  return { busy, approve, dialog: preview ? <ConfirmDialog title="向此连接发送历史？" confirmLabel="确认发送"
    description={<><p>目标：{preview.serviceName}{preview.baseUrl ? ` · ${preview.baseUrl}` : ""}</p><p>{preview.historyScope}</p><p>{preview.messageCount} 条消息，{preview.attachmentCount} 个附件。{preview.mayIncurCost ? "可能产生服务费用。" : ""}</p></>}
    onCancel={() => resolve.current?.(false)} onConfirm={() => resolve.current?.(true)}/> : null };
}
