import { useEffect, useRef, useState } from "react";
import { aiErrorMessage } from "./ai-runtime";
import { setConversationsArchived } from "./conversation-archive";
import type { MessageSink } from "./app-message";
import { ArchiveIcon, CloseIcon } from "./icons";
import "./archive-notice.css";

export interface ArchiveNoticeOperation { id: number; conversationIds: string[] }

export function ArchiveNotice({ operation, onView, onClose, onMessage }: {
  operation: ArchiveNoticeOperation;
  onView: (conversationId: string) => void;
  onClose: () => void;
  onMessage: MessageSink;
}) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [undoing, setUndoing] = useState(false);
  const undoInFlight = useRef(false);
  const mounted = useRef(true);
  const remaining = useRef(operation.conversationIds);
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    if (hovered || focused || undoing) return;
    const timer = window.setTimeout(() => close.current(), 15_000);
    return () => window.clearTimeout(timer);
  }, [hovered, focused, undoing]);
  const undo = async () => {
    if (undoInFlight.current) return;
    undoInFlight.current = true;
    setUndoing(true);
    try {
      const result = await setConversationsArchived(remaining.current, false);
      const restored = new Set(result.conversations.map((item) => item.conversationId));
      remaining.current = remaining.current.filter((id) => !restored.has(id));
      if (!mounted.current) return;
      if (result.failures.length) onMessage(`撤销未完成：${aiErrorMessage(result.failures[0].reason)}`, "error");
      else { onClose(); onMessage("已撤销归档"); }
    } catch (error) {
      if (mounted.current) onMessage(`撤销失败：${aiErrorMessage(error)}`, "error");
    } finally {
      undoInFlight.current = false;
      if (mounted.current) setUndoing(false);
    }
  };
  return <div className="archive-notice" role="status"
    onMouseEnter={() => setHovered(true)} onMouseLeave={() => setHovered(false)}
    onFocus={() => setFocused(true)} onBlur={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false); }}>
    <ArchiveIcon/>
    <span>{operation.conversationIds.length === 1 ? "已归档对话" : `已归档 ${operation.conversationIds.length} 条对话`}</span>
    <button className="archive-notice-view" disabled={undoing} onClick={() => onView(remaining.current[0])} type="button">查看</button>
    <button className="archive-notice-undo" disabled={undoing} onClick={() => void undo()} type="button">{undoing ? "撤销中…" : "撤销"}</button>
    <button aria-label="关闭归档提示" className="archive-notice-close" onClick={onClose} type="button"><CloseIcon/></button>
  </div>;
}
