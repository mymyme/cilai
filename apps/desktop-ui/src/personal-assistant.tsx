import { useModelHistoryTransfer } from "./model-history-transfer";
import type { AssistantWorkItem, WorkSupplement } from "./assistant-work";
import { AssistantActivity, type AssistantActivityTab, type AssistantTrackingCursor, type AssistantTrackingHistory, type AssistantTrackingHistoryView, type AssistantTrackingItem } from "./assistant-activity";
import { Fragment, useCallback, useEffect, useLayoutEffect, useRef, useState, type FormEvent } from "react";
import { aiErrorMessage, controlAiRun, discardAiAttachment, stageAiAttachment, type AiAgentRun, type AiConversationMessage, type AiReasoningEffort } from "./ai-runtime";
import { ContinueReplyButton, PauseReplyButton, RunRecoveryStatus, useRunRecovery } from "./ai-run-recovery";
import { DraftAttachments, MessageAttachments, type DraftAttachment } from "./conversation-attachments";
import { ConversationActionButton, ConversationComposer, ConversationComposerAdd, ConversationMessage, ConversationMessageBody, ConversationMessageList, ConversationLatestButton, ConversationStage } from "./conversation-view";
import { auditMoment, conversationMessageMoment, nextLocalMidnight } from "./datetime";
import { ConversationDraftInput, isConversationSendKey } from "./conversation-draft-input";
import { ConversationModelPicker } from "./conversation-model-picker";
import { useConversationSessionDispatchReady, useConversationSessionFlush, useConversationSessionId, useConversationSessionReadScope, useConversationSessionState } from "./conversation-session";
import { invoke } from "./host";
import { MarkdownContent } from "./markdown-content";
import { usePageActive } from "./page-visibility";
import { ModelCatalogNotice, useModelCatalog } from "./use-model-catalog";
import { PersonalRecordsDialog, type PersonalRecordKind, type PersonalReminderIntent } from "./personal-records";
import { Select } from "./select";
import "./personal-assistant.css";

const draftKey = "conversation_personal_assistant";
const historyWheelPauseMs = 180;
const historyLoadingMinMs = 240;
const atTranscriptTop = (panel: HTMLDivElement) => panel.scrollTop <= 1;
const activeStatuses = new Set(["queued", "running", "paused", "terminating", "waiting_permission", "permission_approved"]);
const recoverableStatuses = new Set(["failed", "interrupted", "terminated"]);
const labels: Record<string, string> = {
  pending: "待受理", queued: "已排队", running: "执行中", completed: "执行结束",
  result_available: "成果可用", stage_result: "阶段结果", waiting_user: "等待补充", waiting_permission: "等待确认", permission_approved: "已确认",
  paused: "已暂停", terminating: "正在停止", terminated: "已停止", failed: "执行失败",
  interrupted: "已中断", delivery_error: "送达待核对", unavailable: "对话不可用", archived: "已归档",
};

interface AssistantView {
  runtimeSessionId: string;
  messages: (AiConversationMessage & { workRunId?: string | null; retainedReply?: string })[];
  before: string | null;
  work?: AssistantWorkItem[];
  run: (Pick<AiAgentRun, "runId" | "inputMessageId" | "status" | "partialOutput" | "threadWriterActive"> & { foreground: boolean }) | null;
  tracking: AssistantTrackingItem[];
  noticeError?: string | null;
}
interface PendingSend {
  connectionId: string | null; messageId: string; draft: string; attachmentIds: string[]; modelId: string; effort: AiReasoningEffort | null;
  workTarget?: WorkSupplement;
}

function hasRecordedInput(view: AssistantView | null, attempt: PendingSend) {
  return view?.run?.inputMessageId === attempt.messageId || view?.messages.some(message => message.messageId === attempt.messageId);
}

export function PersonalAssistantIcon() {
  return <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.65" strokeLinecap="round" strokeLinejoin="round">
    <circle cx="11" cy="9" r="3.25"/><path d="M4.5 19v-1.25c0-2.25 2.9-4 6.5-4s6.5 1.75 6.5 4V19M19 4v5m-2.5-2.5h5"/>
  </svg>;
}

function assistantError(error: unknown) {
  const value = String(error).replace(/^Error: /, "");
  if (value === "codex_harness_closed" || value === "模型 Harness 执行失败：codex_harness_closed") {
    return "回复连接已中断，请核对原消息是否已送达。";
  }
  const messages: Record<string, string> = {
    personal_assistant_target_busy: "目标对话暂时无法接收补充。",
    personal_assistant_work_input_stale: "这项工作已更新，请查看当前待处理事项。",
    personal_assistant_target_archived: "请先恢复原对话。",
    personal_assistant_target_unavailable: "目标对话不可用。",
    personal_assistant_uncontrolled_tools: "无法核验助理的工具范围。",
    personal_assistant_dispatch_unavailable: "暂时无法分派。",
    message_delivery_uncertain: "送达待核对，请查看原对话。",
    personal_assistant_history_anchor_missing: "找不到这条助理消息。",
    model_memory_scope_unavailable: "请先使用选定的模型连接完成一次助理对话，再管理该连接的记忆。",
    ai_memory_disabled: "当前记忆已关闭，请在设置中开启。",
  };
  return messages[value] ?? aiErrorMessage(error);
}

export function PersonalAssistantPage(props: {
  onOpenBoard?: () => void;
  supplement?: WorkSupplement | null;
  onSupplementChange?: (target: WorkSupplement | null) => void;
  recordProjects?: { projectId: string; displayName: string }[];
  reminderIntent?: PersonalReminderIntent | null;
  onReminderIntentConsumed?: (token: string) => void;
}) {
  const active = usePageActive();
  const identity = useConversationSessionReadScope();
  const boundSessionId = useConversationSessionId();
  const sessionId = identity ? boundSessionId : null;
  const dispatchReady = useConversationSessionDispatchReady();
  const historyTransfer = useModelHistoryTransfer(boundSessionId);
  const flush = useConversationSessionFlush();
  const catalog = useModelCatalog(active);
  const [drafts, setDrafts] = useConversationSessionState<Record<string, string>>("drafts", {});
  const [attachmentGroups, setAttachmentGroups] = useConversationSessionState<Record<string, DraftAttachment[]>>("draftAttachmentGroups", {});
  const [pending, setPending] = useConversationSessionState<PendingSend | null>("personalAssistantOutgoing", null);
  const [view, setView] = useState<AssistantView | null>(null);
  const [historyPage, setHistoryPage] = useState<Pick<AssistantView, "messages" | "before"> | null>(null);
  const [highlight, setHighlight] = useState<{ id: string } | null>(null);
  const [now, setNow] = useState(Date.now);
  const [older, setOlder] = useState<AssistantView["messages"]>([]);
  const [olderBefore, setOlderBefore] = useState<string | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [atHistoryTop, setAtHistoryTop] = useState(false);
  const [noteOpen, setNoteOpen] = useState(false);
  const [records, setRecords] = useState<{ kind: PersonalRecordKind; session: string; recordId?: string; token?: string } | null>(null);
  const [noteKind, setNoteKind] = useState("remember");
  const [note, setNote] = useState("");
  const [noteStatus, setNoteStatus] = useState<string | null>(null);
  const [noteSaving, setNoteSaving] = useState(false);
  const noteOperation = useRef<{ id: string; createdAt: number } | null>(null);
  const [panelOpen, setPanelOpen] = useState(() => window.innerWidth >= 900);
  const [panelTab, setPanelTab] = useState<AssistantActivityTab>("overview");
  const [model, setModel] = useConversationSessionState<string | null>("personalAssistantModel", null);
  const [effort, setEffort] = useConversationSessionState<AiReasoningEffort | null>("personalAssistantEffort", null);
  const transcript = useRef<HTMLDivElement>(null);
  const messageFlow = useRef<HTMLDivElement>(null);
  const historyBounce = useRef<Animation | null>(null);
  const historyRequest = useRef<symbol | null>(null);
  const historyScrollAnchor = useRef<{ message: HTMLElement; top: number } | null>(null);
  const lastHistoryWheel = useRef(-Infinity);
  const historyTouchStart = useRef<{ x: number; y: number } | null>(null);
  const toggler = useRef<HTMLButtonElement>(null);
  const panelClose = useRef<HTMLButtonElement>(null);
  const files = useRef<HTMLInputElement>(null);
  const sending = useRef(false);
  const scopeRef = useRef(sessionId);
  scopeRef.current = sessionId;
  const followScroll = useRef(true);
  const [followingLatest, setFollowingLatest] = useState(true);
  const [unseenOutput, setUnseenOutput] = useState(false);
  const current = sessionId && view?.runtimeSessionId === sessionId ? view : null;
  const recordsUpdateKey = `${current?.messages.at(-1)?.messageId ?? ""}:${current?.run?.runId ?? ""}:${current?.run?.status ?? ""}`;
  const draft = drafts[draftKey] ?? "";
  const attachments = attachmentGroups[draftKey] ?? [];
  const [connectionId, setConnectionId] = useConversationSessionState<string | null>("personalAssistantConnection", null);
  const selectedModel = catalog.models.find(item => item.modelId === model && item.connectionId === connectionId);
  const running = Boolean(current?.run && (activeStatuses.has(current.run.status) || current.run.threadWriterActive));
  const before = olderBefore === undefined ? (historyPage ? historyPage.before : current?.before) : olderBefore;
  const work = current?.work ?? [];
  const supplement = props.supplement;
  const supplementCurrent = Boolean(supplement && work.some(item => item.runId === supplement.runId && item.waiting));
  const reminderIntent = props.reminderIntent;
  useEffect(() => {
    if (!active || !sessionId || reminderIntent?.runtimeSessionId !== sessionId) return;
    setRecords({ kind: "todo", session: sessionId, recordId: reminderIntent.todoId, token: reminderIntent.token });
    props.onReminderIntentConsumed?.(reminderIntent.token);
  }, [active, sessionId, reminderIntent, props.onReminderIntentConsumed]);

  function openRecords(kind: PersonalRecordKind, recordId?: string) {
    if (!sessionId || !dispatchReady) return;
    setNoteOpen(false);
    setRecords({ kind, session: sessionId, recordId });
  }

  function closeRecords() {
    setRecords(null);
    requestAnimationFrame(() => {
      if (document.activeElement === document.body) (panelClose.current ?? toggler.current)?.focus();
    });
  }

  function openPanel() {
    setNoteOpen(false);
    setPanelOpen(true);
    requestAnimationFrame(() => panelClose.current?.focus());
  }

  const requestView = useCallback(async (anchor: string | null = null, atMessage?: string) => {
    if (!sessionId) throw new Error("ai_read_scope_changed");
    const value = await invoke<AssistantView>("ai_personal_assistant_view", { runtimeSessionId: sessionId, before: anchor, ...(atMessage ? { atMessage } : {}) });
    if (scopeRef.current !== sessionId || value.runtimeSessionId !== sessionId) throw new Error("ai_read_scope_changed");
    return value;
  }, [sessionId]);

  const refreshCurrent = async () => {
    try {
      setView(await requestView());
      setError(null);
    } catch (failure) {
      if (scopeRef.current === sessionId) setError(assistantError(failure));
    }
  };

  const incompleteRun = current?.run?.foreground && recoverableStatuses.has(current.run.status) ? current.run : null;
  const recovery = useRunRecovery({
    run: active && !running ? incompleteRun : null,
    scope: sessionId,
    disabled: !dispatchReady || busy || uploading || Boolean(pending),
    onContinued: run => {
      setView(value => value?.runtimeSessionId === sessionId
        ? { ...value, run: { runId: run.runId, inputMessageId: run.inputMessageId, status: run.status,
          partialOutput: run.partialOutput, threadWriterActive: run.threadWriterActive, foreground: true } } : value);
      returnToLatest();
    },
    onRefresh: () => void refreshCurrent(),
  });
  const continuing = recovery?.phase === "continuing";
  const canContinue = recovery?.checked?.state === "ready" && !draft.trim() && !attachments.length && !supplement && !pending;

  const loadTrackingHistory = useCallback(async (view: AssistantTrackingHistoryView, before: AssistantTrackingCursor | null) => {
    if (!sessionId) throw new Error("ai_read_scope_changed");
    const page = await invoke<AssistantTrackingHistory & { runtimeSessionId: string }>("ai_personal_assistant_view", {
      runtimeSessionId: sessionId, trackingHistory: view, trackingBefore: before,
    });
    if (scopeRef.current !== sessionId || page.runtimeSessionId !== sessionId) throw new Error("ai_read_scope_changed");
    return page;
  }, [sessionId]);

  useEffect(() => {
    historyRequest.current = null;
    historyScrollAnchor.current = null;
    historyBounce.current?.cancel();
    setLoadingOlder(false);
    setOlder([]);
    setOlderBefore(undefined);
    setHistoryPage(null);
    setHighlight(null);
    setError(null);
  }, [sessionId]);

  useEffect(() => {
    if (!active) return;
    let timer: number;
    const update = () => {
      const currentTime = Date.now();
      setNow(currentTime);
      window.clearTimeout(timer);
      timer = window.setTimeout(update, nextLocalMidnight(currentTime) - currentTime);
    };
    update();
    window.addEventListener("focus", update);
    return () => { window.clearTimeout(timer); window.removeEventListener("focus", update); };
  }, [active]);

  useEffect(() => {
    if (!active || !sessionId) return;
    let disposed = false;
    let timer: number | undefined;
    async function refresh() {
      try {
        const value = await requestView();
        if (!disposed) { setView(value); setError(null); }
      } catch (error) {
        if (!disposed) setError(assistantError(error));
      }
      if (!disposed) timer = window.setTimeout(() => void refresh(), running ? 500 : 1500);
    }
    void refresh();
    return () => { disposed = true; if (timer !== undefined) window.clearTimeout(timer); };
  }, [active, sessionId, requestView, running]);

  const acknowledge = useCallback((accepted: PendingSend) => {
    setDrafts(values => values[draftKey] === accepted.draft ? { ...values, [draftKey]: "" } : values);
    setAttachmentGroups(values => ({ ...values, [draftKey]: (values[draftKey] ?? []).filter(item => !accepted.attachmentIds.includes(item.attachment.attachmentId)) }));
    setPending(value => value?.messageId === accepted.messageId ? null : value);
    setActionError(null);
    if (accepted.workTarget) props.onSupplementChange?.(null);
  }, [setDrafts, setAttachmentGroups, setPending, props.onSupplementChange]);

  useEffect(() => {
    if (pending && hasRecordedInput(current, pending)) acknowledge(pending);
  }, [acknowledge, pending, current]);

  useLayoutEffect(() => {
    const panel = transcript.current;
    if (!active || !panel) return;
    const anchor = historyScrollAnchor.current;
    historyScrollAnchor.current = null;
    if (anchor && panel.contains(anchor.message)) {
      panel.scrollTop += anchor.message.getBoundingClientRect().top - anchor.top;
    } else if (!historyPage && followScroll.current) {
      panel.scrollTop = panel.scrollHeight;
    }
    setAtHistoryTop(atTranscriptTop(panel));
    setFollowingLatest(!historyPage && followScroll.current);
  }, [active, before, historyPage, older, current?.messages.at(-1)?.messageId, current?.run?.partialOutput]);

  useEffect(() => {
    if (!highlight || !active) return;
    const message = Array.from(transcript.current?.querySelectorAll<HTMLElement>("[data-message-id]") ?? [])
      .find(node => node.dataset.messageId === highlight.id);
    message?.scrollIntoView?.({ block: "center" });
    const timer = window.setTimeout(() => setHighlight(null), 2200);
    return () => window.clearTimeout(timer);
  }, [active, highlight]);

  useEffect(() => {
    if (!followScroll.current) setUnseenOutput(true);
  }, [current?.messages.at(-1)?.messageId, current?.run?.partialOutput]);

  const returnToLatest = () => {
    historyRequest.current = null;
    historyScrollAnchor.current = null;
    historyBounce.current?.cancel();
    setLoadingOlder(false);
    followScroll.current = true;
    setFollowingLatest(true);
    setUnseenOutput(false);
    if (transcript.current) transcript.current.scrollTop = transcript.current.scrollHeight;
    setHistoryPage(null);
    setOlder([]);
    setOlderBefore(undefined);
    setHighlight(null);
  };

  const reconcile = async (attempt: PendingSend) => {
    if (!sessionId) return;
    try {
      const receipt = await invoke<{status: string; conversationId?: string; reason?: string}>("ai_personal_assistant_receipt", { runtimeSessionId: sessionId, clientMessageId: attempt.messageId, conversationId: attempt.workTarget?.id ?? draftKey });
      if (scopeRef.current !== sessionId) return;
      if (receipt?.status === "recorded" && receipt.conversationId === (attempt.workTarget?.id ?? draftKey)) {
        acknowledge(attempt);
        await refreshCurrent();
        return;
      }
      const latest = await requestView();
      setView(latest);
      if (hasRecordedInput(latest, attempt)) acknowledge(attempt);
      else if (receipt?.status === "failed" && receipt.reason !== "message_delivery_uncertain") {
        setPending(null);
        setActionError(assistantError(receipt.reason ?? "发送失败"));
      } else setActionError(receipt?.status === "preparing" ? "正在准备发送。" : "送达尚未确认。原消息保留待核对。");
    } catch (error) { if (scopeRef.current === sessionId) setActionError(assistantError(error)); }
  };

  const send = async (event?: FormEvent) => {
    event?.preventDefault();
    if (!sessionId || !dispatchReady || sending.current || running || uploading || recovery?.isSubmitting()) return;
    if (pending) { await reconcile(pending); return; }
    if (supplement ? !supplementCurrent || !draft.trim() || attachments.length
      : !selectedModel || (!draft.trim() && !attachments.length)) return;
    const attempt = {
      messageId: `${supplement ? "pa-work" : "pa-user"}-${crypto.randomUUID()}`, draft,
      modelId: selectedModel?.modelId ?? "", connectionId: selectedModel?.connectionId ?? null, effort,
      attachmentIds: attachments.map(item => item.attachment.attachmentId),
      workTarget: supplement ?? undefined,
    };
    sending.current = true;
    setBusy(true);
    setActionError(null);
    let dispatched = false;
    try {
      const approval = attempt.workTarget ? {allowed:true as const,consent:null} : await historyTransfer.approve({connectionId:attempt.connectionId!,modelId:attempt.modelId,reasoningEffort:attempt.effort},current?.messages.length ? draftKey : null);
      if (!approval.allowed || scopeRef.current !== sessionId) return;
      setPending(attempt);
      await flush();
      if (scopeRef.current !== sessionId) return;
      dispatched = true;
      const receipt = await invoke<AiAgentRun | { status: "rejected"; reason: string }>("ai_personal_assistant_send", {
        runtimeSessionId: sessionId, clientMessageId: attempt.messageId, content: attempt.draft,
        connectionId: attempt.connectionId, modelId: attempt.modelId, reasoningEffort: attempt.effort, attachmentIds: attempt.attachmentIds, historyConsent: approval.consent,
        ...(attempt.workTarget ? { workRunId: attempt.workTarget.runId } : {}),
      });
      if (scopeRef.current === sessionId) {
        if (receipt.status === "rejected") {
          setPending(null);
          setActionError(assistantError(receipt.reason));
          return;
        }
        acknowledge(attempt);
        returnToLatest();
        await refreshCurrent();
      }
    } catch (error) {
      if (scopeRef.current === sessionId) {
        if (dispatched && attempt.workTarget && String(error).includes("personal_assistant_work_input_stale")) {
          setPending(null);
          setActionError(assistantError("personal_assistant_work_input_stale"));
          await refreshCurrent();
        } else if (dispatched) await reconcile(attempt);
        else {
          setPending(null);
          setActionError(assistantError(error));
        }
      }
    } finally {
      sending.current = false;
      setBusy(false);
    }
  };

  const pauseReply = async () => {
    if (!current?.run?.foreground || busy || !dispatchReady) return;
    setBusy(true);
    setActionError(null);
    try {
      await controlAiRun(current.run.runId, "terminate");
      if (scopeRef.current === sessionId) await refreshCurrent();
    } catch (failure) {
      if (scopeRef.current === sessionId) setActionError(assistantError(failure));
    } finally {
      setBusy(false);
    }
  };

  const loadEarlier = async () => {
    if (!active || !before || historyRequest.current || historyBounce.current?.playState === "running") return;
    const panel = transcript.current;
    const request = Symbol();
    historyRequest.current = request;
    followScroll.current = false;
    setLoadingOlder(true);
    setActionError(null);
    try {
      const [page] = await Promise.all([
        requestView(before),
        new Promise<void>(resolve => window.setTimeout(resolve, historyLoadingMinMs)),
      ]);
      if (historyRequest.current !== request) return;
      const message = panel?.querySelector<HTMLElement>("[data-message-id]");
      if (message) historyScrollAnchor.current = { message, top: message.getBoundingClientRect().top };
      setOlder(value => [...page.messages, ...value]);
      setOlderBefore(page.before);
    } catch (error) {
      if (historyRequest.current === request) setActionError(assistantError(error));
    } finally {
      if (historyRequest.current === request) {
        historyRequest.current = null;
        setLoadingOlder(false);
      }
    }
  };

  const stageFiles = async (selected: FileList | null) => {
    if (!selected || !sessionId || !dispatchReady || pending || uploading) return;
    setUploading(true);
    try {
      for (const file of Array.from(selected).slice(0, Math.max(0, 8 - attachments.length))) {
        const attachment = await stageAiAttachment(file);
        if (scopeRef.current !== sessionId) return;
        setAttachmentGroups(value => ({ ...value, [draftKey]: [...(value[draftKey] ?? []), { attachment, previewUrl: "" }] }));
      }
    } catch (error) { setActionError(assistantError(error)); }
    finally { setUploading(false); if (files.current) files.current.value = ""; }
  };

  const act = async (command: string, args: Record<string, unknown>) => {
    setActionError(null);
    try { await invoke(command, args); setView(await requestView()); return true; }
    catch (error) { setActionError(assistantError(error)); return false; }
  };

  const submitNote = async (event: FormEvent) => {
    event.preventDefault();
    if (!sessionId || !dispatchReady || !note.trim() || noteSaving) return;
    const operation = noteOperation.current ?? { id: crypto.randomUUID(), createdAt: Date.now() };
    noteOperation.current = operation;
    setNoteSaving(true);
    try {
      await invoke("ai_personal_assistant_memory_note", {
        runtimeSessionId: sessionId, operationId: operation.id, createdAt: operation.createdAt, kind: noteKind, content: note,
      });
      if (scopeRef.current !== sessionId) return;
      setNote("");
      setNoteStatus("说明已受理，等待原生记忆整理。");
      noteOperation.current = null;
    } catch (error) {
      if (scopeRef.current === sessionId) setNoteStatus(assistantError(error));
    } finally { setNoteSaving(false); }
  };

  const closePanel = () => {
    setPanelOpen(false);
    toggler.current?.focus();
  };
  const displayed = historyPage?.messages ?? current?.messages ?? [];
  const entries = [...new Map([...older, ...displayed].map(message => [message.messageId, message])).values()]
    .sort((a, b) => a.createdAt - b.createdAt);
  const waitingWork = work.filter(item => item.waiting !== null);
  const closeNarrowPanel = () => {
    const width = transcript.current?.closest(".pa-layout")?.clientWidth;
    if (width && width <= 760) setPanelOpen(false);
  };
  const selectSupplement = (item: WorkSupplement) => {
    if (pending || busy) return;
    props.onSupplementChange?.({ id: item.id, runId: item.runId, title: item.title });
    closeNarrowPanel();
    requestAnimationFrame(() => document.querySelector<HTMLElement>('.pa-page [aria-label="给个人助理发消息"]')?.focus());
  };
  const locateMessage = async (id: string) => {
    if (historyRequest.current) return;
    const request = Symbol();
    historyRequest.current = request;
    setActionError(null);
    followScroll.current = false;
    setLoadingOlder(true);
    try {
      if (!entries.some(message => message.messageId === id)) {
        const page = await requestView(null, id);
        if (historyRequest.current !== request) return;
        setOlder([]);
        setOlderBefore(undefined);
        setHistoryPage(page);
      }
      setHighlight({ id });
      closeNarrowPanel();
    } catch (error) {
      if (historyRequest.current === request) setActionError(assistantError(error));
    } finally {
      if (historyRequest.current === request) {
        historyRequest.current = null;
        setLoadingOlder(false);
      }
    }
  };

  return <section className="pa-page" onKeyDown={event => {
    if (event.key !== "Escape" || (event.target instanceof Element && event.target.closest('[role="menu"], [aria-haspopup="menu"][aria-expanded="true"]'))) return;
    if (records && records.session === sessionId) return;
    if (noteOpen) setNoteOpen(false);
    else if (panelOpen) closePanel(); }}>
    <header className="pa-header">
      <div className="pa-identity"><PersonalAssistantIcon/><h1>个人助理</h1></div>
      <div className="pa-actions">
        {props.onOpenBoard ? <button className="pa-button" type="button" onClick={props.onOpenBoard}>工作看板</button> : null}
        <div className="pa-settings">
          <button className="pa-button" type="button" ref={toggler} aria-expanded={noteOpen} onClick={() => setNoteOpen(value => !value)}>助理设置</button>
          {noteOpen ? <form className="pa-note" onSubmit={event => void submitNote(event)} aria-label="补充记忆">
            <button className="pa-button" type="button" aria-controls="personal-assistant-activity" aria-expanded={panelOpen}
              onClick={() => { if (panelOpen) closePanel(); else openPanel(); }}>{panelOpen ? "收起个人事项" : "显示个人事项"}</button>
            <strong>补充记忆</strong>
            <Select ariaLabel="记忆操作" value={noteKind} disabled={noteSaving}
              options={[{ value: "remember", label: "添加" }, { value: "correct", label: "更正" }, { value: "forget", label: "遗忘" }]}
              onChange={kind => { setNoteKind(kind); noteOperation.current = null; }}/>
            <textarea aria-label="记忆说明" placeholder="记忆说明" rows={3} maxLength={1500} value={note} disabled={noteSaving}
              onChange={event => { setNote(event.currentTarget.value); noteOperation.current = null; }}/>
            <button className="pa-button" type="submit" disabled={!dispatchReady || !note.trim() || noteSaving}>提交说明</button>
            {noteStatus ? <p role="status">{noteStatus}</p> : null}
          </form> : null}
        </div>
      </div>
    </header>
    <div className="pa-layout ai-workbench conversation-rail-collapsed">
      <ConversationStage panelOpen={panelOpen} docked panel={<AssistantActivity key={sessionId} items={current?.tracking ?? []} open={panelOpen} pageActive={active}
        tab={panelTab} onTabChange={setPanelTab} recordsSessionId={active && dispatchReady ? sessionId : null}
        recordsUpdateKey={recordsUpdateKey} onOpenRecords={openRecords} onLoadHistory={loadTrackingHistory}
        closeRef={panelClose} onClose={closePanel} now={now} labels={labels} onSupplement={selectSupplement}
        onLocate={id => void locateMessage(id)} onOpenArtifact={(runId, artifactId) => void act("ai_artifact_open", { runId, artifactId, openMode: "default" })}
        onStop={runId => void act("ai_run_control", { request: { runId, action: "terminate" } })}
        onUnfollow={operationId => act("ai_personal_assistant_unfollow", { runtimeSessionId: sessionId, operationId })}/>}>
        <ConversationMessageList landing={!entries.length && !running && !error}
          after={historyPage || !followingLatest ? <ConversationLatestButton unseen={unseenOutput} onClick={returnToLatest}/> : null}
          listProps={{
            ref: transcript,
            tabIndex: before ? 0 : undefined,
            "aria-label": "个人助理消息",
            onScroll: event => {
              const panel = event.currentTarget;
              followScroll.current = !historyPage && panel.scrollHeight - panel.scrollTop - panel.clientHeight <= 48;
              setFollowingLatest(followScroll.current);
              if (followScroll.current) setUnseenOutput(false);
              const atTop = atTranscriptTop(panel);
              if (!atTop) historyBounce.current?.cancel();
              else if (!atHistoryTop && before && !historyRequest.current && historyBounce.current?.playState !== "running"
                && !window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
                historyBounce.current = messageFlow.current?.animate([
                  { transform: "translateY(0)", easing: "ease-out" },
                  { transform: "translateY(12px)", offset: 0.32, easing: "ease-in-out" },
                  { transform: "translateY(-2px)", offset: 0.78, easing: "ease-out" },
                  { transform: "translateY(0)" },
                ], { duration: 320 }) ?? null;
              }
              setAtHistoryTop(atTop);
            },
            onWheel: event => {
              if (!before || event.ctrlKey || !event.deltaY || Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
              const startsGesture = event.timeStamp - lastHistoryWheel.current >= historyWheelPauseMs;
              lastHistoryWheel.current = event.timeStamp;
              if (startsGesture && event.deltaY < 0 && atHistoryTop && atTranscriptTop(event.currentTarget)) void loadEarlier();
            },
            onTouchStart: event => {
              const touch = event.touches[0];
              historyTouchStart.current = before && atHistoryTop && atTranscriptTop(event.currentTarget) && event.touches.length === 1
                && historyBounce.current?.playState !== "running"
                ? { x: touch.clientX, y: touch.clientY } : null;
            },
            onTouchMove: event => {
              const start = historyTouchStart.current;
              if (!start || event.touches.length !== 1 || !atTranscriptTop(event.currentTarget)) {
                historyTouchStart.current = null;
                return;
              }
              const touch = event.touches[0];
              const distance = touch.clientY - start.y;
              if (distance > 24 && distance > Math.abs(touch.clientX - start.x)) {
                historyTouchStart.current = null;
                void loadEarlier();
              }
            },
            onKeyDown: event => {
              if (!before || !atHistoryTop || !atTranscriptTop(event.currentTarget) || event.target !== event.currentTarget
                || event.repeat || (event.key !== "ArrowUp" && event.key !== "PageUp")) return;
              event.preventDefault();
              void loadEarlier();
            },
          }}>
          <div className="pa-message-flow" ref={messageFlow}>
            {before ? <div className="pa-history-hint" data-visible={atHistoryTop || loadingOlder} data-loading={loadingOlder} role="status">
              {loadingOlder ? "正在加载消息…" : "上滑加载更多消息"}
            </div> : null}
            {error ? <div className="ai-list-empty" role="alert">{error}</div> : null}
            {current?.noticeError ? <div className="ai-list-empty" role="alert">这次汇报没能生成。你可以在个人事项里查看最新状态。</div> : null}
            {!entries.length && !running && !error ? <div className="ai-starter-content">
              <header className="ai-starter-heading"><h2>今天想推进什么？</h2></header>
            </div> : null}
            {entries.map(entry => <Fragment key={entry.messageId}><ConversationMessage role={entry.role} author={entry.role === "user" ? "你" : "个人助理"}
                messageId={entry.messageId} className={highlight?.id === entry.messageId ? "pa-message-highlight" : undefined}
                meta={<time dateTime={new Date(entry.createdAt).toISOString()} title={auditMoment(entry.createdAt)}>{conversationMessageMoment(entry.createdAt, now)}</time>}>
                {entry.attachments.length ? <MessageAttachments attachments={entry.attachments}/> : null}
                {entry.content ? <ConversationMessageBody role={entry.role} content={entry.content} markdown={{ diagramIdPrefix: `pa-${entry.messageId}` }}/> : null}
                {waitingWork.filter(item => item.runId === entry.workRunId).map(item =>
                  <button className="pa-link" type="button" key={item.runId} onClick={() => selectSupplement(item)}>补充信息</button>)}
              </ConversationMessage>
              {entry.retainedReply ? <article aria-label="未完成的 AI 消息" className="ai-message assistant ai-message-incomplete">
                <header><span>个人助理</span></header>
                <ConversationMessageBody role="assistant" content={entry.retainedReply} markdown={{ diagramIdPrefix: `pa-partial-${entry.messageId}` }}/>
              </article> : null}
            </Fragment>)}
            {running && !historyPage ? <ConversationMessage role="assistant" author="个人助理" className="ai-message-pending" pending>
              {current?.run?.partialOutput
                ? <div aria-label="正在输出的回复" aria-live="polite" className="ai-streaming-output"><MarkdownContent diagramIdPrefix="pa-stream">{current.run.partialOutput}</MarkdownContent></div>
                : <div className="ai-run-progress"><i aria-hidden="true" className="ai-running-mark"/>{labels[current?.run?.status ?? "running"]}</div>}
            </ConversationMessage> : null}
            {incompleteRun && !running && !historyPage ? <section aria-label="本轮回复状态" className="ai-conversation-runs">
              <span>{incompleteRun.status === "terminated" ? "回复已暂停" : incompleteRun.status === "interrupted" ? "回复已中断" : "回复未完成"}</span>
              {recovery ? <RunRecoveryStatus recovery={recovery}/> : null}
            </section> : null}
          </div>
        </ConversationMessageList>
        {actionError ? <div className="ai-recovered-send" data-failed="true" role="alert"><div><span>{actionError}</span></div></div> : null}
        <ModelCatalogNotice catalog={catalog} variant="conversation"/>
        <ConversationComposer onSubmit={event => void send(event)}
          leadingAction={<ConversationComposerAdd canAttachMedia mediaDescription="添加附件"
            disabled={!dispatchReady || uploading || Boolean(pending) || Boolean(supplement)} hasFile={attachments.length > 0}
            onOpenMedia={() => files.current?.click()}/>}
          actions={<>
            <ConversationModelPicker displayModel={selectedModel} onOpen={catalog.refreshOnOpen} historical={Boolean(entries.length)} loading={catalog.loading}
              models={catalog.models} selectedConnectionId={connectionId} selectedModelId={model} selectedReasoningEffort={effort}
              disabled={Boolean(pending) || running || busy} onSelectModel={(id, next, connection) => { setConnectionId(connection); setModel(id); setEffort(next); }}/>
            {running && current?.run?.foreground
              ? <PauseReplyButton disabled={!dispatchReady || busy || !activeStatuses.has(current.run.status) || current.run.status === "terminating"} onPause={() => void pauseReply()}/>
              : recovery && (continuing || canContinue) ? <ContinueReplyButton recovery={recovery} disabled={recovery.disabled}/>
              : <ConversationActionButton aria-label={busy ? "发送中" : pending ? "核对送达" : "发送"} title={pending ? "核对送达" : "发送"}
                type="submit" disabled={!dispatchReady || running || busy || uploading || (!pending && (supplement ? !supplementCurrent || !draft.trim() || Boolean(attachments.length)
                  : !selectedModel || (!draft.trim() && !attachments.length)))}/>}
          </>}>
          {attachments.length ? <DraftAttachments items={attachments} removable={!pending} onRemove={id => {
            void discardAiAttachment(id).then(() => setAttachmentGroups(value => ({ ...value, [draftKey]: (value[draftKey] ?? []).filter(item => item.attachment.attachmentId !== id) }))).catch(error => setActionError(assistantError(error)));
          }}/> : null}
          {supplement ? <div className="pa-actions"><span>补充给「{supplement.title}」</span>
            <button className="pa-link" type="button" disabled={Boolean(pending)} onClick={() => props.onSupplementChange?.(null)}>取消</button>
          </div> : null}
          <div className="ai-composer-input">
            <ConversationDraftInput aria-label="给个人助理发消息" placeholder="给个人助理发消息" value={draft}
              readOnly={busy || Boolean(pending)} onValueChange={value => setDrafts(values => ({ ...values, [draftKey]: value }))} rows={1} wrap="soft"
              onKeyDown={event => { if (isConversationSendKey(event)) { event.preventDefault(); void send(); } }}/>
          </div>
        </ConversationComposer>
        <input type="file" ref={files} className="ai-media-input" aria-hidden="true" multiple tabIndex={-1} onChange={event => void stageFiles(event.currentTarget.files)}/>
      </ConversationStage>
    </div>
    {historyTransfer.dialog}
    {active && records && records.session === sessionId && dispatchReady ? <PersonalRecordsDialog
      key={records.token ?? records.recordId ?? records.kind} kind={records.kind} runtimeSessionId={records.session}
      initialRecordId={records.recordId} updateKey={recordsUpdateKey}
      scopes={[
        ...(props.recordProjects ?? []).map(project => ({ kind: "project" as const, id: project.projectId, label: project.displayName })),
        ...work.map(item => ({ kind: "conversation" as const, id: item.id, label: item.title })),
      ]}
      onClose={closeRecords}
      onLocateSource={id => { setRecords(null); void locateMessage(id); }}/> : null}
  </section>;
}
