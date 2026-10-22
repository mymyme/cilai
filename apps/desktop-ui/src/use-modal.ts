import { useEffect, useLayoutEffect, useRef } from "react";
import { usePageActive } from "./page-visibility";

/**
 * 模态弹窗共用的键盘与焦点行为。此前九个弹窗各自手写 Escape，其中三个漏掉了；
 * 焦点陷阱一个都没有，Tab 会径直走到弹窗背后的页面上，键盘用户能在看不见的地方
 * 点到被遮住的按钮；关闭之后焦点也没人送回去，得从页首重新 Tab 一遍。
 *
 * 初始焦点仍由各弹窗自己的 autoFocus 决定——哪个字段该先亮起来是内容问题，
 * 这里不该替它们判断。
 *
 * 用法：把返回的 ref 挂到弹窗最外层元素上。
 */
export function useModal<T extends HTMLElement>(options: {
  onClose: () => void;
  /** 忙碌时（例如正在保存）不接受 Escape，避免半途中断已经发出的请求。 */
  busy?: boolean;
  active?: boolean;
}): React.RefObject<T | null> {
  const pageActive = usePageActive();
  const pageActiveRef = useRef(pageActive);
  useLayoutEffect(() => { pageActiveRef.current = pageActive; },[pageActive]);
  const containerRef = useRef<T>(null);
  const optionsRef = useRef(options);
  const triggerRef = useRef<HTMLElement | null>(
    document.activeElement instanceof HTMLElement ? document.activeElement : null,
  );
  useLayoutEffect(() => { optionsRef.current = options; });

  useEffect(() => {
    if (!pageActive) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (optionsRef.current.active === false) return;
      if (event.key === "Escape") {
        if (!optionsRef.current.busy) optionsRef.current.onClose();
        return;
      }
      if (event.key !== "Tab") return;
      const container = containerRef.current;
      if (container === null) return;
      const stops = focusableWithin(container);
      if (stops.length === 0) {
        // 弹窗里没有任何可聚焦元素时，焦点不该溜到背后的页面上。
        event.preventDefault();
        return;
      }
      const first = stops[0];
      const last = stops[stops.length - 1];
      const active = document.activeElement;
      // 只在两端接管，中间的 Tab 交给浏览器自己走，顺序才和视觉顺序一致。
      if (event.shiftKey && (active === first || !container.contains(active))) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (active === last || !container.contains(active))) {
        event.preventDefault();
        first.focus();
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      if (!pageActiveRef.current) return;
      // StrictMode 会在元素仍挂载时校验一次副作用，这不是真正关闭弹窗。
      if (containerRef.current?.isConnected) return;
      // 弹窗关掉后把焦点送回触发它的那颗按钮。元素可能已经随列表一起卸载，
      // 那时 isConnected 为假，还给它反而会把焦点丢到 document.body。
      if (triggerRef.current !== null && triggerRef.current.isConnected) triggerRef.current.focus();
    };
  }, [pageActive]);

  return containerRef;
}

const FOCUSABLE = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

/** 按文档顺序取出能停留焦点的元素；隐藏起来的不算。 */
function focusableWithin(container: HTMLElement): HTMLElement[] {
  return [...container.querySelectorAll<HTMLElement>(FOCUSABLE)]
    .filter((element) => element.offsetParent !== null || element === document.activeElement);
}
