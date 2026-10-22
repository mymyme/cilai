import { useEffect, useState, type ReactNode } from "react";
import { createPortal } from "./page-portal";
import { useModal } from "./use-modal";
import "./confirm-dialog.css";

/**
 * Confirmation for actions the user cannot simply undo. Destructive actions state what is removed
 * and what is kept, so the consequence is visible before the click rather than after it.
 *
 * `commentLabel` turns the dialog into the collection point for an action's own written record.
 * Keeping that text here rather than in a shared side panel is what stops one conclusion from
 * being attached to several unrelated events.
 */
export function ConfirmDialog(props: {
  title: string;
  description?: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  confirmDisabled?: boolean;
  busy?: boolean;
  commentLabel?: string;
  commentPlaceholder?: string;
  onConfirm: (comment: string) => void;
  onCancel: () => void;
}) {
  const [comment, setComment] = useState("");
  const commentMissing = Boolean(props.commentLabel) && !comment.trim();
  const modalRef = useModal<HTMLDivElement>({ onClose: props.onCancel, busy: props.busy });

  return createPortal(<div
    aria-label={props.title}
    aria-modal="true"
    className="confirm-backdrop"
    ref={modalRef}
    // Only a press that starts on the backdrop dismisses, so dragging a selection out of the
    // dialog does not close it.
    onMouseDown={(event) => { if (event.target === event.currentTarget && !props.busy) props.onCancel(); }}
    role="dialog">
    <section className="confirm-dialog">
      <h3>{props.title}</h3>
      {props.description != null ? <div className="confirm-body">{props.description}</div> : null}
      {props.commentLabel ? <label className="confirm-comment">{props.commentLabel}<textarea
        autoFocus
        disabled={props.busy}
        onChange={(event) => setComment(event.target.value)}
        placeholder={props.commentPlaceholder}
        rows={3}
        value={comment}/></label> : null}
      <div className="confirm-actions">
        <button disabled={props.busy} onClick={props.onCancel} type="button">取消</button>
        <button
          autoFocus={!props.commentLabel}
          className={props.danger ? "danger" : "primary"}
          disabled={props.busy || props.confirmDisabled || commentMissing}
          onClick={() => props.onConfirm(comment.trim())}
          type="button">{props.busy ? "处理中…" : props.confirmLabel}</button>
      </div>
    </section>
  </div>, document.body);
}

/**
 * 失败提示的落点。和确认弹窗共用外壳与样式，区别只在于它没有可选项：动作已经没做成，
 * 用户唯一能做的就是知道这件事。用 alertdialog 而不是 dialog，读屏器会立即播报。
 */
export function AlertDialog(props: { title: string; message: string; onClose: () => void }) {
  const modalRef = useModal<HTMLDivElement>({ onClose: props.onClose });

  return createPortal(<div className="confirm-backdrop" ref={modalRef} role="alertdialog" aria-modal="true" aria-label={props.title}>
    <div className="confirm-dialog alert-dialog">
      <h3>{props.title}</h3>
      <div className="confirm-body"><p>{props.message}</p></div>
      <div className="confirm-actions">
        <button autoFocus className="primary" onClick={props.onClose} type="button">知道了</button>
      </div>
    </div>
  </div>, document.body);
}
