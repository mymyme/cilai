import { useId, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import { createPortal } from "react-dom";
import { noticeText } from "./text.js";
import "./stack.css";

type PageNoticeBase = {
  id: string;
  /** 唯一的一句正文：通知写结果，报错写原因和下一步；不设标题。 */
  message: string;
  paused?: boolean;
  /**
   * 持续状态的稳定键（例如“任务编号加失败原因”）。用户关闭后，本次页面会话内同一键不再弹出，
   * 键变化或应用重新加载后会再次提示；不传则每次展示都视为新的提醒。
   */
  stateKey?: string;
  /** 点击卡片正文时触发（例如定位到对应字段）；卡片内的按钮不触发。键盘使用者通过 actions 到达同一动作。 */
  onActivate?: () => void;
  actions?: readonly { label: string; onClick: () => void; disabled?: boolean }[];
};

/** 通知：操作已完成或已开始，定时收起。 */
export type PageInformation = PageNoticeBase & {
  kind: "notice";
  tone: "success" | "info";
  durationMs?: number;
  /** 悬停暂停后，移开指针时重新使用完整的自动关闭时长。 */
  resetOnHover?: boolean;
};

/** 报错：操作未成功或需要人工处理，不自动收起，由用户关闭或问题解除后撤下。 */
export type PageError = PageNoticeBase & {
  kind: "error";
  tone: "danger" | "warning";
  durationMs?: never;
  resetOnHover?: never;
};

export type PageNotice = PageInformation | PageError;
export type PageNoticeStackProps = {
  anchor: HTMLElement | null;
  content?: HTMLElement | null;
  active?: boolean;
  notices: readonly PageNotice[];
  onDismiss: (id: string, reason: "close" | "timeout") => void;
  className?: string;
  returnFocus?: HTMLElement | null;
};

type Placement = {
  anchor: HTMLElement;
  left: number;
  top: number;
  width: number;
  maxHeight: number;
};

function usePlacement(anchor: HTMLElement | null, content: HTMLElement | null, active: boolean) {
  const [position, setPosition] = useState<Placement | null>(null);
  useLayoutEffect(() => {
    const document = anchor?.ownerDocument;
    const view = document?.defaultView;
    if (!active || !anchor || !document || !view) {
      setPosition(null);
      return;
    }
    let frame = 0;
    const update = () => {
      frame = 0;
      const bounds = anchor.getBoundingClientRect();
      const contentBounds = content?.getBoundingClientRect() ?? bounds;
      const leftEdge = Math.max(0, contentBounds.left);
      const rightEdge = Math.min(view.innerWidth, contentBounds.right);
      const width = Math.min(360, rightEdge - leftEdge - 24);
      const top = bounds.bottom + 10;
      const maxHeight = view.innerHeight - top - 12;
      if (!anchor.isConnected || document.hidden || width <= 0 || bounds.bottom < 0 || maxHeight < 55) {
        setPosition(null);
        return;
      }
      const left = leftEdge + (rightEdge - leftEdge - width) / 2;
      setPosition((current) => current?.anchor === anchor && current.left === left && current.top === top
        && current.width === width && current.maxHeight === maxHeight ? current : { anchor, left, top, width, maxHeight });
    };
    const schedule = () => {
      if (!frame) frame = view.requestAnimationFrame(update);
    };
    const boundary = document.body;
    const observer = new ResizeObserver(schedule);
    const layout = new MutationObserver(() => {
      watchLayout();
      schedule();
    });
    function watchLayout() {
      observer.disconnect();
      layout.disconnect();
      if (content) observer.observe(content);
      for (let element: HTMLElement | null = anchor; element && element !== boundary; element = element.parentElement) {
        observer.observe(element);
        if (element.previousElementSibling) observer.observe(element.previousElementSibling);
        layout.observe(element, { childList: true });
      }
    }
    watchLayout();
    update();
    view.addEventListener("resize", schedule);
    view.addEventListener("scroll", schedule, true);
    document.addEventListener("visibilitychange", update);
    return () => {
      observer.disconnect();
      layout.disconnect();
      view.cancelAnimationFrame(frame);
      view.removeEventListener("resize", schedule);
      view.removeEventListener("scroll", schedule, true);
      document.removeEventListener("visibilitychange", update);
    };
  }, [anchor, content, active]);
  return active && position?.anchor === anchor ? position : null;
}

function useDismissTimer(
  notices: readonly PageNotice[],
  running: PageNotice | undefined,
  onDismiss: PageNoticeStackProps["onDismiss"],
  hovering: boolean,
) {
  const budgets = useRef(new Map<string, { duration: number; remaining: number; expired: boolean }>());
  const dismiss = useRef(onDismiss);
  useLayoutEffect(() => { dismiss.current = onDismiss; }, [onDismiss]);
  useLayoutEffect(() => {
    const live = new Set(notices.filter(({ durationMs }) => durationMs !== undefined).map(({ id }) => id));
    for (const id of budgets.current.keys()) {
      if (!live.has(id)) budgets.current.delete(id);
    }
    for (const notice of notices) {
      if (hovering && notice.resetOnHover && notice.durationMs !== undefined) {
        budgets.current.set(notice.id, { duration: notice.durationMs, remaining: notice.durationMs, expired: false });
      }
    }
  }, [notices, hovering]);
  const id = running?.id;
  const duration = running?.durationMs;
  useLayoutEffect(() => {
    if (id === undefined || duration === undefined) return;
    const previous = budgets.current.get(id);
    const budget = previous?.duration === duration ? previous : { duration, remaining: duration, expired: false };
    budgets.current.set(id, budget);
    if (budget.expired) return;
    const started = performance.now();
    const timeout = window.setTimeout(() => {
      budget.expired = true;
      dismiss.current(id, "timeout");
    }, budget.remaining);
    return () => {
      window.clearTimeout(timeout);
      budget.remaining = Math.max(0, budget.remaining - (performance.now() - started));
    };
  }, [id, duration]);
}

const marks: Record<PageNotice["tone"] | "arrow" | "close", ReactNode> = {
  info: <><circle cx="10" cy="10" r="7"/><path d="M10 6.5v4l2.5 1.5"/></>,
  success: <path d="m4.5 10 3.5 3.5 7.5-7.5"/>,
  warning: <><path d="M8.7 3.8a1.5 1.5 0 0 1 2.6 0l6.2 10.7a1.5 1.5 0 0 1-1.3 2.2H3.8a1.5 1.5 0 0 1-1.3-2.2z"/><path d="M10 7.4v3.3m0 2.7v.1"/></>,
  danger: <><circle cx="10" cy="10" r="7"/><path d="M10 6.6v4m0 2.8v.1"/></>,
  arrow: <path d="m6 8 4 4 4-4"/>,
  close: <path d="m6 6 8 8m0-8-8 8"/>,
};

function NoticeIcon({ name }: { name: keyof typeof marks }) {
  return <svg aria-hidden="true" focusable="false" viewBox="0 0 20 20" fill="none" stroke="currentColor"
    strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">{marks[name]}</svg>;
}

export function PageNoticeStack({ active = true, anchor, content = null, notices, onDismiss, className, returnFocus }: PageNoticeStackProps) {
  const panelId = useId();
  const root = useRef<HTMLElement>(null);
  const toggle = useRef<HTMLButtonElement>(null);
  const lastFocus = useRef<HTMLElement | null>(null);
  const focusedControl = useRef<{ element: HTMLElement; index: number } | null>(null);
  const retainedOrder = useRef<string[]>([]);
  const [expanded, setExpanded] = useState(false);
  const [hovering, setHovering] = useState(false);
  const [focused, setFocused] = useState(false);
  const position = usePlacement(anchor, content, active && notices.length > 0);
  const opened = expanded && notices.length > 1;
  const positions = new Map(retainedOrder.current.map((id, index) => [id, index]));
  const ordered = [...notices].sort((a, b) => opened || focused || hovering
    ? (positions.get(a.id) ?? positions.size) - (positions.get(b.id) ?? positions.size)
    : Number(a.durationMs !== undefined) - Number(b.durationMs !== undefined));
  const [first, ...rest] = ordered;

  useLayoutEffect(() => {
    retainedOrder.current = ordered.map(({ id }) => id);
  });
  useLayoutEffect(() => {
    if (!position) {
      setExpanded(false);
      setHovering(false);
      setFocused(false);
    } else if (notices.length < 2) {
      setExpanded(false);
    }
  }, [position, notices.length]);

  useDismissTimer(notices, position && !opened && !hovering && !focused && !first?.paused ? first : undefined, onDismiss, hovering);

  useLayoutEffect(() => {
    const request = focusedControl.current;
    if (!request || request.element.isConnected) return;
    focusedControl.current = null;
    const document = request.element.ownerDocument;
    if (!active || document.activeElement !== document.body) {
      setFocused(false);
      return;
    }
    const buttons = Array.from(root.current?.querySelectorAll<HTMLButtonElement>("[data-notice-close]") ?? []);
    const next = opened ? buttons[Math.min(request.index, buttons.length - 1)] : toggle.current ?? buttons[0];
    const target = next ?? returnFocus ?? lastFocus.current;
    if (target?.isConnected) target.focus({ preventScroll: true });
    else setFocused(false);
  });
  useLayoutEffect(() => {
    if (!opened || !position) return;
    const document = position.anchor.ownerDocument;
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !root.current?.contains(event.target)) setExpanded(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [opened, position]);

  if (!first || !position) return null;

  const card = (notice: PageNotice, index: number) => (
    <article className="cilai-page-notice" data-tone={notice.tone} data-notice-index={index} data-activatable={notice.onActivate ? "true" : undefined} key={notice.id}
      role={notice.onActivate ? "button" : undefined} tabIndex={notice.onActivate ? 0 : undefined}
      onClick={notice.onActivate ? (event) => { if (!(event.target as HTMLElement).closest("button")) notice.onActivate?.(); } : undefined}
      onKeyDown={notice.onActivate ? (event) => { if (event.target === event.currentTarget && (event.key === "Enter" || event.key === " ")) { event.preventDefault(); notice.onActivate?.(); } } : undefined}>
      <span className="cilai-page-notice__mark"><NoticeIcon name={notice.tone}/></span>
      <div className="cilai-page-notice__copy" aria-live={index === 0 ? "polite" : undefined}>
        <span>{noticeText(notice)}</span>
      </div>
      <div className="cilai-page-notice__actions">
        {index === 0 && rest.length > 0 && <button type="button" ref={toggle}
          className="cilai-page-notice__toggle" aria-expanded={opened} aria-controls={panelId}
          aria-label={opened ? "收起提示" : `展开 ${ordered.length} 条提示`} onClick={() => setExpanded(!opened)}>
          <NoticeIcon name="arrow"/>
        </button>}
        <button type="button" data-notice-close aria-label={`关闭提示：${noticeText(notice)}`}
          onClick={() => onDismiss(notice.id, "close")}><NoticeIcon name="close"/></button>
      </div>
      {notice.actions?.length ? <div className="cilai-page-notice__business-actions">
        {notice.actions.map(({ label, onClick, disabled }) => <button key={label} type="button" onClick={onClick} disabled={disabled}>{label}</button>)}
      </div> : null}
    </article>
  );

  return createPortal(
    <section aria-label="页面提示" ref={root}
      className={className ? `cilai-page-notice-stack ${className}` : "cilai-page-notice-stack"}
      data-expanded={opened} data-layers={Math.min(2, rest.length)}
      style={{ left: position.left, top: position.top, width: position.width, maxHeight: position.maxHeight,
        "--cilai-page-notice-space": `${position.maxHeight}px` } as CSSProperties}
      onPointerEnter={() => setHovering(true)} onPointerLeave={() => setHovering(false)}
      onFocus={(event) => {
        if (event.relatedTarget instanceof HTMLElement && !event.currentTarget.contains(event.relatedTarget)) {
          lastFocus.current = event.relatedTarget;
        }
        if (event.target instanceof HTMLElement) {
          const index = event.target.closest<HTMLElement>("[data-notice-index]")?.dataset.noticeIndex;
          if (index !== undefined) focusedControl.current = { element: event.target, index: Number(index) };
        }
        setFocused(true);
      }}
      onBlur={(event) => {
        if (event.currentTarget.contains(event.relatedTarget)) return;
        if (event.relatedTarget || event.target.isConnected) focusedControl.current = null;
        setFocused(false);
      }}
      onKeyDown={(event) => {
        if (event.key !== "Escape" || event.defaultPrevented || !opened) return;
        event.preventDefault();
        event.stopPropagation();
        toggle.current?.focus({ preventScroll: true });
        setExpanded(false);
      }}>
      <div className="cilai-page-notice-stack__front">{card(first, 0)}</div>
      <div className="cilai-page-notice-stack__fold" id={panelId} aria-hidden={!opened} inert={!opened}>
        <div className="cilai-page-notice-stack__scroll">{rest.map((notice, index) => card(notice, index + 1))}</div>
      </div>
    </section>,
    position.anchor.ownerDocument.body,
  );
}
