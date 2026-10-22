import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "./page-portal";
import "./context-menu.css";

export type ContextMenuItem =
  /** A rule between groups of related actions, so a long menu stays scannable. */
  | { separator: true }
  | {
    separator?: false;
    label: string;
    icon?: ReactNode;
    danger?: boolean;
    disabled?: boolean;
    onSelect: () => void;
  };

export type ContextMenuState = { x: number; y: number; items: ContextMenuItem[] } | null;

/**
 * Right-clicking anything that is not editable text used to raise the operating system's own
 * menu — "Look Up", "Translate", "Search with Baidu" — which knows nothing about the object under
 * the cursor. This renders the app's own menu at the cursor instead.
 */
export function ContextMenu(props: { state: ContextMenuState; onClose: () => void }) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  const { state, onClose } = props;

  useLayoutEffect(() => {
    if (!state) { setPosition(null); return; }
    const menu = menuRef.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    const margin = 8;
    setPosition({
      left: Math.max(margin, Math.min(state.x, window.innerWidth - rect.width - margin)),
      top: Math.max(margin, Math.min(state.y, window.innerHeight - rect.height - margin)),
    });
  }, [state]);

  useEffect(() => {
    if (!state) return;
    const dismiss = () => onClose();
    // pointerdown 先于 click 触发。菜单项自己的 pointerdown 一样会冒泡到 window，
    // 如果不放行，菜单会在 mouseup 之前就卸载，click 永远派发不到那颗按钮上——
    // 表现就是菜单里每一项点了都没反应，只是把菜单关掉。
    const dismissOnOutsidePointer = (event: PointerEvent) => {
      if (menuRef.current?.contains(event.target as Node)) return;
      onClose();
    };
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    window.addEventListener("pointerdown", dismissOnOutsidePointer);
    window.addEventListener("scroll", dismiss, true);
    window.addEventListener("resize", dismiss);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", dismissOnOutsidePointer);
      window.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("resize", dismiss);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [state, onClose]);

  if (!state) return null;

  return createPortal(
    <div
      className="context-menu"
      ref={menuRef}
      role="menu"
      // Measured before it is placed, so it is rendered off-screen for one frame rather than
      // flashing at the top-left corner.
      style={position ? { left: position.left, top: position.top } : { left: -9999, top: -9999 }}>
      {state.items.map((item, index) => item.separator
        ? <hr key={`separator-${index}`}/>
        : <button
          className={item.danger ? "danger" : undefined}
          disabled={item.disabled}
          key={item.label}
          role="menuitem"
          type="button"
          onClick={() => { onClose(); item.onSelect(); }}>{item.icon}{item.label}</button>)}
    </div>,
    document.body,
  );
}

/** Opens a menu at the cursor and keeps the browser's own menu from appearing alongside it. */
export function openContextMenu(event: React.MouseEvent, items: ContextMenuItem[]): ContextMenuState {
  event.preventDefault();
  event.stopPropagation();
  return { x: event.clientX, y: event.clientY, items };
}
