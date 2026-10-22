import { invoke } from "./host";
import { useState } from "react";
import { createPortal } from "./page-portal";
import type { ConfirmFileParams, ConfirmFileResult } from "./generated/protocol";
import { randomUuid } from "./uuid";
import { useModal } from "./use-modal";

const CONFIRMATION_ERRORS: Record<string, string> = {
  invalid_file_confirmation_request: "确认请求不完整，请刷新文件状态后重试。",
  confirmation_file_not_found: "文件记录已不存在，请刷新工作区。",
  confirmation_file_not_ready: "文件尚未完成扫描或当前不可读取。",
  confirmation_checksum_mismatch: "文件内容已变化，请重新查看差异后确认。",
  content_changed_during_confirmation: "确认期间文件发生变化，请重新查看差异。",
  confirmation_baseline_changed: "确认基线已经变化，请重新查看差异后确认。",
  confirmation_state_changed: "文件状态已经变化，请刷新后重新确认。",
  confirmation_not_allowed: "当前状态或权限不允许确认该文件。",
  confirmation_identity_unavailable: "本地确认身份不可用，请检查应用数据状态。",
  idempotency_conflict: "本次操作标识已经绑定另一份确认请求。",
  confirmation_checksum_failed: "无法重新读取文件并计算确认摘要。",
  confirmation_snapshot_failed: "无法生成本次确认所需的文本快照，请重新扫描后再试。",
  confirmation_projection_failed: "确认已保存，但本地确认清单尚未导出；可使用同一操作重试。",
};


export interface FileConfirmationDialogProps {
  workspaceId: string;
  localFileId: string;
  relativePath: string;
  checksumAlgorithm: string;
  checksumValue: string;
  baselineVersionId: string | null;
  onCancel: () => void;
  onConfirmed?: () => Promise<void> | void;
  onMessage: (message: string, tone?: "info" | "error") => void;
}

export function FileConfirmationDialog(props: FileConfirmationDialogProps) {
  const [busy, setBusy] = useState<boolean>(false);
  const [confirmationSaved, setConfirmationSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmOperationId] = useState(() => `desktop-confirm-${randomUuid()}`);
  const modalRef = useModal<HTMLDivElement>({ onClose: props.onCancel, busy });

  const confirmFile = async () => {
    if (confirmationSaved) return false;
    const params: ConfirmFileParams = {
      workspaceId: props.workspaceId,
      localFileId: props.localFileId,
      expectedChecksumAlgorithm: props.checksumAlgorithm,
      expectedChecksumValue: props.checksumValue,
      expectedBaselineVersionId: props.baselineVersionId,
      clientOperationId: confirmOperationId,
    };
    await invoke<ConfirmFileResult>("file_confirm", { params });
    setConfirmationSaved(true);
    return true;
  };

  const run = async () => {
    if (busy) return;
    let savedInThisAction = false;
    setBusy(true);
    setError(null);
    try {
      savedInThisAction = await confirmFile();
      if (savedInThisAction) await props.onConfirmed?.();
      props.onMessage("本次文件内容已确认。");
      props.onCancel();
    } catch (caught) {
      setError(confirmationError(caught, confirmationSaved || savedInThisAction));
    } finally {
      setBusy(false);
    }
  };

  return createPortal(<div
    aria-label="是否确认文件内容？"
    aria-modal="true"
    className="confirm-backdrop"
    onMouseDown={(event) => { if (event.target === event.currentTarget && !busy) props.onCancel(); }}
    ref={modalRef}
    role="dialog">
    <section className="confirm-dialog file-confirmation-dialog">
      <h3>是否确认文件内容？</h3>
      <div className="confirm-body">
        <p><strong>{props.relativePath}</strong></p>
        <p>记录本次内容核对结果。</p>
        {error ? <p className="confirmation-dialog-error" role="alert">{error}</p> : null}
      </div>
      <div className="confirm-actions confirmation-decision-actions">
        <button disabled={busy} onClick={props.onCancel} type="button">取消</button>
        <button autoFocus className="primary" disabled={busy || confirmationSaved} onClick={() => void run()} type="button">{busy ? "确认中…" : "确认"}</button>
      </div>
    </section>
  </div>, document.body);
}

function confirmationError(error: unknown, confirmationSaved: boolean) {
  const text = String(error);
  const confirmationReason = Object.keys(CONFIRMATION_ERRORS).find((reason) => text.includes(reason));
  if (confirmationReason) return CONFIRMATION_ERRORS[confirmationReason];
  const prefix = confirmationSaved ? "文件内容已确认，但" : "";
  return `${prefix || "操作"}未完成，请刷新文件状态后重试。`;
}
