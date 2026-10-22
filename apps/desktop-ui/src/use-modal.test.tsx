import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { StrictMode } from "react";
import { useModal } from "./use-modal";

afterEach(cleanup);

function Dialog({ onClose, busy }: { onClose: () => void; busy?: boolean }) {
  const ref = useModal<HTMLDivElement>({ onClose, busy });
  return <div ref={ref} role="dialog">
    <button type="button">首个</button>
    <input aria-label="项目名称" autoFocus />
    <textarea aria-label="项目目标" />
    <button type="button">末个</button>
  </div>;
}

/** jsdom 不做布局，offsetParent 恒为 null，焦点判定需要它表现得像可见元素。 */
function makeVisible(container: HTMLElement) {
  for (const element of container.querySelectorAll("button, input, textarea")) {
    Object.defineProperty(element, "offsetParent", { get: () => container, configurable: true });
  }
}

describe("模态弹窗的键盘与焦点行为", () => {
  it("Escape 关闭弹窗", () => {
    const onClose = vi.fn();
    render(<Dialog onClose={onClose}/>);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("忙碌时不接受 Escape：请求已经发出去，半途关掉会让人以为没提交", () => {
    const onClose = vi.fn();
    render(<Dialog busy onClose={onClose}/>);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("父页刷新且回调引用变化时，不会把项目目标的焦点抢走", () => {
    const view = render(<Dialog onClose={() => {}}/>);
    const goal = screen.getByRole("textbox", { name: "项目目标" });
    goal.focus();

    view.rerender(<Dialog onClose={() => {}}/>);

    expect(document.activeElement).toBe(goal);
  });

  it("严格模式校验副作时仍保留弹窗的初始焦点", () => {
    render(<StrictMode><Dialog onClose={() => {}}/></StrictMode>);

    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "项目名称" }));
  });

  it("不重建焦点监听也会使用最新的忙碌状态和关闭回调", () => {
    const firstClose = vi.fn();
    const latestClose = vi.fn();
    const view = render(<Dialog busy onClose={firstClose}/>);
    view.rerender(<Dialog busy={false} onClose={latestClose}/>);

    fireEvent.keyDown(window, { key: "Escape" });

    expect(firstClose).not.toHaveBeenCalled();
    expect(latestClose).toHaveBeenCalledOnce();
  });

  it("焦点走到末尾再按 Tab 回到开头，不会溜到弹窗背后的页面上", () => {
    const { container } = render(<Dialog onClose={() => {}}/>);
    makeVisible(container);
    const last = screen.getByRole("button", { name: "末个" });
    last.focus();
    fireEvent.keyDown(window, { key: "Tab" });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "首个" }));
  });

  it("在开头按 Shift+Tab 回到末尾", () => {
    const { container } = render(<Dialog onClose={() => {}}/>);
    makeVisible(container);
    screen.getByRole("button", { name: "首个" }).focus();
    fireEvent.keyDown(window, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "末个" }));
  });

  it("关闭后焦点回到打开它的那颗按钮，而不是丢在 body 上", () => {
    const trigger = document.createElement("button");
    document.body.appendChild(trigger);
    trigger.focus();
    expect(document.activeElement).toBe(trigger);

    const view = render(<Dialog onClose={() => {}}/>);
    makeVisible(view.container);
    screen.getByRole("button", { name: "首个" }).focus();

    view.unmount();
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });
});
