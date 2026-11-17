import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "./page-portal";
import { usePageActive } from "./page-visibility";
import "./hover-tip.css";

/** 停留多久才弹。太短会在指针划过时闪出来，太长则要等。 */
const hoverDelay = 1000;

/**
 * 悬停气泡。
 *
 * 气泡挂到文档根部，避免被侧边栏或文件预览区裁剪。位置按触发元素实测，指针移开或按下即收。
 */
export function HoverTip(props: {
  label: string;
  side?: "right" | "bottom";
  /** 为假时不弹，例如侧边栏展开后名称已经直接可见。 */
  enabled?: boolean;
  children: (handlers: {
    onPointerEnter: () => void;
    onPointerLeave: () => void;
    onPointerDown: () => void;
    ref: (node: HTMLElement | null) => void;
  }) => React.ReactNode;
}) {
  const enabled = usePageActive() && props.enabled !== false;
  const [placement, setPlacement] = useState<{ left: number; top: number; maxWidth?: number } | null>(null);
  const anchorRef = useRef<HTMLElement | null>(null);
  const timerRef = useRef<number | null>(null);

  const clear = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  // 卸载时必须清掉计时器，否则组件已经不在了还会尝试落位
  useEffect(() => clear, [clear]);
  // 关闭条件从启用态消失那一刻起立即生效，不等指针移开
  useEffect(() => {
    if (!enabled) {
      clear();
      setPlacement(null);
    }
  }, [enabled, clear]);

  const show = () => {
    const anchor = anchorRef.current;
    if (!anchor) return;
    const rect = anchor.getBoundingClientRect();
    if (props.side === "bottom") {
      const margin = 8;
      const left = Math.max(margin, Math.min(Math.round(rect.left), window.innerWidth - 240));
      setPlacement({ left, top: Math.round(rect.bottom + margin), maxWidth: Math.min(480, window.innerWidth - left - margin) });
      return;
    }
    setPlacement({ left: Math.round(rect.right + 8), top: Math.round(rect.top + rect.height / 2) });
  };

  const handlers = {
    ref: (node: HTMLElement | null) => { anchorRef.current = node; },
    onPointerEnter: () => {
      if (!enabled) return;
      clear();
      timerRef.current = window.setTimeout(show, hoverDelay);
    },
    onPointerLeave: () => { clear(); setPlacement(null); },
    // 点下去就是已经知道要去哪了，气泡继续留着只会挡住刚切过去的页面
    onPointerDown: () => { clear(); setPlacement(null); },
  };

  return <>
    {props.children(handlers)}
    {enabled && placement ? createPortal(
      <div className="hover-tip" data-side={props.side} role="tooltip" style={placement}>{props.label}</div>,
      document.body,
    ) : null}
  </>;
}
