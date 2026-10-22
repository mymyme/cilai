import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "./page-portal";
import { usePageActive } from "./page-visibility";
import "./select.css";

export type SelectOption = { value: string; label: string; disabled?: boolean };

/**
 * Replaces the native <select>. The browser draws the option list with the operating system's
 * own menu, which no stylesheet can reach, so every dropdown in the app looked foreign.
 *
 * The list is rendered through a portal rather than next to the trigger, so the trigger stays a
 * direct child of whatever laid it out. Page rules that size or place a select keep working.
 */
export function Select(props: {
  value: string;
  options: readonly SelectOption[];
  onChange: (value: string) => void;
  onOpen?: () => void;
  ariaLabel?: string;
  autoFocus?: boolean;
  className?: string;
  disabled?: boolean;
  id?: string;
  placeholder?: string;
}) {
  const pageActive = usePageActive();
  const [open, setOpen] = useState(false);
  useEffect(() => { if (!pageActive) setOpen(false); },[pageActive]);
  useEffect(() => { if (props.disabled) setOpen(false); },[props.disabled]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [placement, setPlacement] = useState<{ left: number; top: number; width: number; maxHeight: number; above: boolean } | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const selectedIndex = props.options.findIndex((option) => option.value === props.value);
  const selected = selectedIndex >= 0 ? props.options[selectedIndex] : null;

  const close = useCallback(() => {
    setOpen(false);
    triggerRef.current?.focus();
  }, []);

  const measure = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const gap = 4;
    const below = window.innerHeight - rect.bottom - gap;
    const above = rect.top - gap;
    const useAbove = below < 180 && above > below;
    setPlacement({
      left: rect.left,
      top: useAbove ? rect.top - gap : rect.bottom + gap,
      width: rect.width,
      maxHeight: Math.max(120, Math.min(288, useAbove ? above : below)),
      above: useAbove,
    });
  }, []);

  useLayoutEffect(() => {
    if (!open || !pageActive) return;
    measure();
  }, [open, measure, pageActive]);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!triggerRef.current?.contains(target) && !listRef.current?.contains(target)) setOpen(false);
    };
    const dismiss = () => setOpen(false);
    const onScroll = (event: Event) => {
      if (event.target instanceof Node && listRef.current?.contains(event.target)) return;
      dismiss();
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", dismiss);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", dismiss);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    listRef.current?.querySelector<HTMLElement>('[data-active="true"]')?.scrollIntoView({ block: "nearest" });
  }, [open, activeIndex]);

  const openList = () => {
    if (props.disabled) return;
    props.onOpen?.();
    setActiveIndex(selectedIndex >= 0 ? selectedIndex : firstEnabled(props.options));
    setOpen(true);
  };

  const commit = (index: number) => {
    if (props.disabled) return;
    const option = props.options[index];
    if (!option || option.disabled) return;
    props.onChange(option.value);
    close();
  };

  const step = (from: number, delta: number) => {
    const total = props.options.length;
    for (let offset = 1; offset <= total; offset += 1) {
      const next = (from + delta * offset + total * offset) % total;
      if (!props.options[next]?.disabled) return next;
    }
    return from;
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (props.disabled) return;
    if (!open) {
      if (["Enter", " ", "ArrowDown", "ArrowUp"].includes(event.key)) {
        event.preventDefault();
        openList();
      }
      return;
    }
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); close(); return; }
    if (event.key === "Tab") { setOpen(false); return; }
    if (event.key === "Enter" || event.key === " ") { event.preventDefault(); commit(activeIndex); return; }
    if (event.key === "ArrowDown") { event.preventDefault(); setActiveIndex((current) => step(current, 1)); return; }
    if (event.key === "ArrowUp") { event.preventDefault(); setActiveIndex((current) => step(current, -1)); return; }
    if (event.key === "Home") { event.preventDefault(); setActiveIndex(firstEnabled(props.options)); return; }
    if (event.key === "End") { event.preventDefault(); setActiveIndex(step(0, -1)); }
  };

  return <>
    <button
      aria-expanded={open}
      aria-haspopup="listbox"
      aria-label={props.ariaLabel}
      autoFocus={props.autoFocus}
      className={props.className ? `ch-select ${props.className}` : "ch-select"}
      disabled={props.disabled}
      id={props.id}
      ref={triggerRef}
      role="combobox"
      type="button"
      onClick={() => (open ? setOpen(false) : openList())}
      onKeyDown={onKeyDown}>
      <span className="ch-select-value" data-placeholder={selected ? undefined : "true"}>{selected ? selected.label : props.placeholder ?? ""}</span>
    </button>
    {open && placement ? createPortal(
      <div
        className="ch-select-list"
        ref={listRef}
        role="listbox"
        style={{
          left: placement.left,
          width: placement.width,
          maxHeight: placement.maxHeight,
          ...(placement.above ? { bottom: window.innerHeight - placement.top } : { top: placement.top }),
        }}
        onKeyDown={onKeyDown}>
        {props.options.map((option, index) => <button
          aria-selected={option.value === props.value}
          data-active={index === activeIndex}
          disabled={option.disabled}
          key={option.value}
          role="option"
          type="button"
          onClick={() => commit(index)}
          onPointerEnter={() => setActiveIndex(index)}>{option.label}</button>)}
      </div>,
      triggerRef.current?.closest('[aria-modal="true"]') ?? document.body,
    ) : null}
  </>;
}

function firstEnabled(options: readonly SelectOption[]) {
  const index = options.findIndex((option) => !option.disabled);
  return index >= 0 ? index : 0;
}
