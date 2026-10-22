import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "./page-portal";
import "./hover-tip.css";

/** 停留多久才弹。太短会在指针划过时闪出来，太长则要等。 */
const hoverDelay = 1000;

/**
 * 悬停气泡。
 *
 * 侧边栏收起后只剩图标，名称无处可看。浏览器自带的 title 提示在这里不合用：外观由系统
 * 绘制，样式表够不到，深色主题下会突兀地弹出一块系统外观，延迟也无法约定。
 *
 * 气泡经传送门挂到文档根部而不是触发元素旁边，侧边栏本身有 overflow 裁剪，挂在里面会
 * 被裁掉。位置按触发元素实测，指针移开或按下即收。
 */
export function HoverTip(props: {
  label: string;
  /** 为假时不弹，例如侧边栏展开后名称已经直接可见。 */
  enabled?: boolean;
  children: (handlers: {
    onPointerEnter: () => void;
    onPointerLeave: () => void;
    onPointerDown: () => void;
    ref: (node: HTMLElement | null) => void;
  }) => React.ReactNode;
}) {
  const [placement, setPlacement] = useState<{ left: number; top: number } | null>(null);
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
    if (props.enabled === false) {
      clear();
      setPlacement(null);
    }
  }, [props.enabled, clear]);

  const show = () => {
    const anchor = anchorRef.current;
    if (!anchor) return;
    const rect = anchor.getBoundingClientRect();
    setPlacement({ left: Math.round(rect.right + 8), top: Math.round(rect.top + rect.height / 2) });
  };

  const handlers = {
    ref: (node: HTMLElement | null) => { anchorRef.current = node; },
    onPointerEnter: () => {
      if (props.enabled === false) return;
      clear();
      timerRef.current = window.setTimeout(show, hoverDelay);
    },
    onPointerLeave: () => { clear(); setPlacement(null); },
    // 点下去就是已经知道要去哪了，气泡继续留着只会挡住刚切过去的页面
    onPointerDown: () => { clear(); setPlacement(null); },
  };

  return <>
    {props.children(handlers)}
    {placement ? createPortal(
      <div className="hover-tip" role="tooltip" style={{ left: placement.left, top: placement.top }}>{props.label}</div>,
      document.body,
    ) : null}
  </>;
}
