import { createContext, type ReactNode, type RefObject, useCallback, useContext, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "./page-portal";
import { usePageActive } from "./page-visibility";
import "./rich-menu.css";

type MenuPlacement = {
  left: number;
  top: number;
  width: number;
  maxHeight: number;
};

const RichMenuContext = createContext<{ close: () => void } | null>(null);

export function RichMenu(props: {
  ariaLabel: string;
  align?: "start" | "end";
  width?: number;
  maxHeight?: number;
  className?: string;
  keyboardNavigation?: boolean;
  shortcutActions?: Readonly<Record<string, () => void>>;
  onOpenChange?: (open: boolean) => void;
  trigger: (state: { open: boolean; toggle: () => void; triggerRef: RefObject<HTMLButtonElement | null> }) => ReactNode;
  children: ReactNode | ((state: { close: () => void }) => ReactNode);
}) {
  const pageActive = usePageActive();
  const [open, setOpen] = useState(false);
  useEffect(() => { if (!pageActive) setOpen(false); },[pageActive]);
  const [placement, setPlacement] = useState<MenuPlacement | null>(null);
  /*
   * 定位落定后再隔一帧标记，供样式表决定要不要对尺寸变化做过渡。
   *
   * 菜单首帧摆在屏幕外（-9999）等待测量，测出真实位置是同一批更新里改的。若此时已经
   * 允许过渡，菜单会从屏幕外一路滑进来。隔一帧之后再放行，首次定位就落在过渡之外。
   */
  const [settled, setSettled] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const close = useCallback(() => {
    if (props.keyboardNavigation) triggerRef.current?.focus();
    setOpen(false);
    setPlacement(null);
  }, [props.keyboardNavigation]);
  const toggle = useCallback(() => setOpen((current) => !current), []);
  const positioned = placement !== null;

  useEffect(() => {
    if (!positioned) {
      setSettled(false);
      return;
    }
    const frame = requestAnimationFrame(() => setSettled(true));
    return () => cancelAnimationFrame(frame);
  }, [positioned]);

  useEffect(() => props.onOpenChange?.(open), [open, props.onOpenChange]);

  useEffect(() => {
    if (positioned && props.keyboardNavigation) menuRef.current?.querySelector<HTMLButtonElement>('[role^="menuitem"]:not(:disabled)')?.focus();
  }, [positioned, props.keyboardNavigation]);

  const measure = useCallback(() => {
    const trigger = triggerRef.current;
    const menu = menuRef.current;
    if (!trigger || !menu) return;
    const margin = 12;
    const gap = 8;
    const triggerRect = trigger.getBoundingClientRect();
    const width = Math.min(props.width ?? 320, Math.max(0, window.innerWidth - margin * 2));
    const menuStyle = window.getComputedStyle(menu);
    const borderHeight = (Number.parseFloat(menuStyle.borderTopWidth) || 0)
      + (Number.parseFloat(menuStyle.borderBottomWidth) || 0);
    const naturalHeight = menu.scrollHeight + borderHeight;
    const availableAbove = Math.max(0, triggerRect.top - margin - gap);
    const availableBelow = Math.max(0, window.innerHeight - triggerRect.bottom - margin - gap);
    const desiredHeight = Math.min(naturalHeight, props.maxHeight ?? 420);
    const opensAbove = availableBelow < desiredHeight && availableAbove > availableBelow;
    const availableHeight = opensAbove ? availableAbove : availableBelow;
    const maxHeight = Math.max(0, Math.min(desiredHeight, availableHeight));
    const rawLeft = props.align === "end" ? triggerRect.right - width : triggerRect.left;
    const left = Math.max(margin, Math.min(rawLeft, window.innerWidth - width - margin));
    const top = opensAbove
      ? Math.max(margin, triggerRect.top - gap - maxHeight)
      : Math.min(window.innerHeight - margin - maxHeight, triggerRect.bottom + gap);
    setPlacement({ left, top, width, maxHeight });
  }, [props.align, props.maxHeight, props.width]);

  useLayoutEffect(() => {
    if (!open || !pageActive) return;
    measure();
  }, [open, measure, pageActive]);

  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!open || !menu) return;
    const observer = new MutationObserver(() => measure());
    observer.observe(menu, { childList: true, subtree: true, characterData: true });
    return () => observer.disconnect();
  }, [measure, open]);

  useEffect(() => {
    if (!open || !pageActive) return;
    const dismissOutside = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!triggerRef.current?.contains(target) && !menuRef.current?.contains(target)) close();
    };
    const handleKey = (event: KeyboardEvent) => {
      if (props.keyboardNavigation && menuRef.current?.contains(document.activeElement)) {
        const items = Array.from(menuRef.current.querySelectorAll<HTMLButtonElement>('[role^="menuitem"]:not(:disabled)'));
        const index = items.indexOf(document.activeElement as HTMLButtonElement);
        const next = event.key === "ArrowDown" ? (index + 1) % items.length
          : event.key === "ArrowUp" ? (index - 1 + items.length) % items.length
          : event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : null;
        if (next !== null && items.length) { event.preventDefault(); items[next]?.focus(); return; }
        if (event.key === "Tab") { close(); return; }
      }
      if (event.key === "Escape") {
        event.preventDefault();
        close();
        triggerRef.current?.focus();
        return;
      }
      const action = props.shortcutActions?.[event.key];
      if (!action) return;
      event.preventDefault();
      action();
      close();
      triggerRef.current?.focus();
    };
    const reposition = () => measure();
    window.addEventListener("pointerdown", dismissOutside);
    window.addEventListener("keydown", handleKey);
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("pointerdown", dismissOutside);
      window.removeEventListener("keydown", handleKey);
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [close, measure, open, props.shortcutActions, props.keyboardNavigation, pageActive]);

  return <>
    {props.trigger({ open, toggle, triggerRef })}
    {open ? createPortal(
      <RichMenuContext.Provider value={{ close }}>
        <div
          aria-label={props.ariaLabel}
          className={props.className ? `ch-rich-menu ${props.className}` : "ch-rich-menu"}
          data-positioned={placement ? "true" : "false"}
          data-settled={settled ? "true" : "false"}
          ref={menuRef}
          role="menu"
          style={placement ?? { left: -9999, top: -9999, width: props.width ?? 320, maxHeight: props.maxHeight ?? 420 }}>
          {typeof props.children === "function" ? props.children({ close }) : props.children}
        </div>
      </RichMenuContext.Provider>,
      document.body,
    ) : null}
  </>;
}

export function RichMenuSection(props: { label: string; children: ReactNode }) {
  return <section aria-label={props.label} className="ch-rich-menu-section" role="group">
    <p>{props.label}</p>
    {props.children}
  </section>;
}

export function RichMenuItem(props: {
  label: string;
  description?: string;
  leading?: ReactNode;
  shortcut?: string;
  selected?: boolean;
  disabled?: boolean;
  action?: boolean;
  closeOnSelect?: boolean;
  title?: string;
  onSelect: () => void;
}) {
  const context = useContext(RichMenuContext);
  return <button
    aria-checked={props.selected === undefined ? undefined : props.selected}
    className="ch-rich-menu-item"
    data-action={props.action ? "true" : undefined}
    data-leading={props.leading ? "true" : undefined}
    data-selected={props.selected ? "true" : undefined}
    disabled={props.disabled}
    onClick={() => { if (props.closeOnSelect !== false) context?.close(); props.onSelect(); }}
    role={props.selected === undefined ? "menuitem" : "menuitemradio"}
    title={props.title}
    type="button">
    {props.leading ? <span className="ch-rich-menu-leading">{props.leading}</span> : null}
    <span className="ch-rich-menu-copy"><strong>{props.label}</strong>{props.description ? <small>{props.description}</small> : null}</span>
    {props.shortcut ? <kbd>{props.shortcut}</kbd> : null}
    {props.selected === undefined ? null : <span className="ch-rich-menu-check">{props.selected ? <CheckIcon/> : null}</span>}
  </button>;
}

export function RichMenuSeparator() {
  return <div aria-hidden="true" className="ch-rich-menu-separator"/>;
}

export function RichMenuEmpty(props: { children: ReactNode }) {
  return <p className="ch-rich-menu-empty">{props.children}</p>;
}

function CheckIcon() {
  return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m6 12 4 4 8-8"/></svg>;
}
