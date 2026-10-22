import { createPortal } from "./page-portal";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type RefObject } from "react";
import { CheckIcon, MoonIcon, SettingsIcon, SunIcon } from "./icons";
import type { AppTheme } from "./theme";
import "./user-menu.css";

export interface LocalUserProfile { displayName: string; profileId: string; }
export function profileInitials(displayName: string) {
  const parts = displayName.trim().split(/\s+/).filter(Boolean);
  if (parts.length > 1) return parts.slice(0, 2).map(part => [...part][0]).join("").toLocaleUpperCase();
  return [...(parts[0] ?? "")].slice(0, 2).join("").toLocaleUpperCase();
}

export function UserMenu(props: {
  expanded: boolean;
  profile: LocalUserProfile;
  onOpenSettings: () => void;
  onOpenChange?: (open: boolean) => void;
  onThemeChange: (theme: AppTheme) => void;
  theme: AppTheme;
}) {
  const [open, setOpen] = useState(false);
  const [placement, setPlacement] = useState<{ left: number; bottom: number; width: number } | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const initials = profileInitials(props.profile.displayName);

  useEffect(() => () => props.onOpenChange?.(false), [props.onOpenChange]);

  const changeOpen = (next: boolean) => {
    props.onOpenChange?.(next);
    setOpen(next);
  };

  useLayoutEffect(() => {
    if (!open || !triggerRef.current) return;
    const rect = triggerRef.current.getBoundingClientRect();
    setPlacement({
      left: Math.max(8, rect.left),
      bottom: window.innerHeight - rect.top + 7,
      width: Math.max(284, rect.width),
    });
  }, [open, props.expanded]);

  useEffect(() => {
    if (!open) return;
    const closeFromOutside = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!triggerRef.current?.contains(target) && !menuRef.current?.contains(target)) changeOpen(false);
    };
    const closeFromKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        changeOpen(false);
        triggerRef.current?.focus();
      }
    };
    const closeFromLayout = () => changeOpen(false);
    window.addEventListener("pointerdown", closeFromOutside);
    window.addEventListener("keydown", closeFromKey);
    window.addEventListener("resize", closeFromLayout);
    return () => {
      window.removeEventListener("pointerdown", closeFromOutside);
      window.removeEventListener("keydown", closeFromKey);
      window.removeEventListener("resize", closeFromLayout);
    };
  }, [open]);

  const toggleMenu = () => changeOpen(!open);

  return <div className="activity-user">
    <button
      aria-expanded={open}
      aria-haspopup="menu"
      aria-label={`用户：${props.profile.displayName}`}
      className="activity-user-trigger"
      onClick={() => void toggleMenu()}
      ref={triggerRef}
      title={props.expanded ? undefined : props.profile.displayName}
      type="button">
      <span className="activity-user-avatar">{initials}</span>
      <span className="activity-user-copy"><strong>{props.profile.displayName}</strong><small>{"本地资料"}</small></span>
      <svg aria-hidden="true" className="activity-user-chevron" viewBox="0 0 16 16"><path d="m4 6 4 4 4-4"/></svg>
    </button>
    {open && placement ? createPortal(<UserMenuPanel {...props} menuRef={menuRef} onClose={() => changeOpen(false)} style={{ left: placement.left, bottom: placement.bottom, width: placement.width }}/>, document.body) : null}
  </div>;
}

function UserMenuPanel(props: {
  profile: LocalUserProfile; onOpenSettings: () => void; onThemeChange: (theme: AppTheme) => void;
  onClose: () => void; theme: AppTheme; menuRef?: RefObject<HTMLDivElement | null>; style?: CSSProperties;
}) {
  const choose = (action: () => void) => { action(); props.onClose(); };
  return <div aria-label="用户菜单" className="activity-user-menu" ref={props.menuRef} role="menu" style={props.style}>
    <header className="activity-user-identity"><span className="activity-user-avatar">{profileInitials(props.profile.displayName)}</span><strong>{props.profile.displayName}</strong></header>
    <section aria-label="外观主题" className="activity-user-theme"><span className="activity-user-section-label">外观</span><div role="group">
      <button aria-pressed={props.theme === "light"} onClick={() => choose(() => props.onThemeChange("light"))} type="button"><SunIcon/><span>浅色</span>{props.theme === "light" ? <CheckIcon/> : null}</button>
      <button aria-pressed={props.theme === "dark"} onClick={() => choose(() => props.onThemeChange("dark"))} type="button"><MoonIcon/><span>深色</span>{props.theme === "dark" ? <CheckIcon/> : null}</button>
    </div></section>
    <button onClick={() => choose(props.onOpenSettings)} role="menuitem" type="button"><SettingsIcon/><span>设置</span></button>
  </div>;
}
