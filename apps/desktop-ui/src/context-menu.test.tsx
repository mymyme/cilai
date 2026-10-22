import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { ContextMenu, openContextMenu, type ContextMenuState } from "./context-menu";

afterEach(cleanup);

function Harness({ onPick }: { onPick: (label: string) => void }) {
  const [state, setState] = useState<ContextMenuState>(null);
  return <>
    <div
      data-testid="target"
      onContextMenu={(event) => setState(openContextMenu(event, [
        { label: "打开", onSelect: () => onPick("打开") },
        { label: "删除", danger: true, onSelect: () => onPick("删除") },
        { separator: true },
        { label: "不可用", disabled: true, onSelect: () => onPick("不可用") },
      ]))}>行</div>
    <ContextMenu state={state} onClose={() => setState(null)}/>
  </>;
}

describe("应用自有右键菜单", () => {
  it("右键前不存在菜单", () => {
    render(<Harness onPick={() => {}}/>);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("右键后按给定顺序列出菜单项", () => {
    render(<Harness onPick={() => {}}/>);
    fireEvent.contextMenu(screen.getByTestId("target"));
    expect(screen.getAllByRole("menuitem").map((item) => item.textContent)).toEqual(["打开", "删除", "不可用"]);
  });

  it("阻止系统菜单弹出", () => {
    render(<Harness onPick={() => {}}/>);
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    screen.getByTestId("target").dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });

  it("选中菜单项后触发对应动作并关闭菜单", () => {
    const onPick = vi.fn();
    render(<Harness onPick={onPick}/>);
    fireEvent.contextMenu(screen.getByTestId("target"));
    fireEvent.click(screen.getByRole("menuitem", { name: "删除" }));
    expect(onPick).toHaveBeenCalledWith("删除");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  // 真实的鼠标一次点击会先发 pointerdown 再发 click。曾经"点击外部关闭"的监听器
  // 不区分内外，菜单项自己的 pointerdown 就把菜单卸载了，click 派发不到按钮上，
  // 于是菜单里每一项点了都只是把菜单关掉。只发 click 的测试看不出这个问题。
  it("按真实事件顺序点击菜单项仍然触发动作", () => {
    const onPick = vi.fn();
    render(<Harness onPick={onPick}/>);
    fireEvent.contextMenu(screen.getByTestId("target"));
    const item = screen.getByRole("menuitem", { name: "打开" });
    fireEvent.pointerDown(item);
    expect(screen.queryByRole("menu")).not.toBeNull();
    fireEvent.click(item);
    expect(onPick).toHaveBeenCalledWith("打开");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("禁用项不可触发", () => {
    const onPick = vi.fn();
    render(<Harness onPick={onPick}/>);
    fireEvent.contextMenu(screen.getByTestId("target"));
    fireEvent.click(screen.getByRole("menuitem", { name: "不可用" }));
    expect(onPick).not.toHaveBeenCalled();
    expect(screen.queryByRole("menu")).not.toBeNull();
  });

  it("Esc 关闭菜单", () => {
    render(<Harness onPick={() => {}}/>);
    fireEvent.contextMenu(screen.getByTestId("target"));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("点击外部关闭菜单", () => {
    render(<Harness onPick={() => {}}/>);
    fireEvent.contextMenu(screen.getByTestId("target"));
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
  });
  it("分隔线只做分组，不参与菜单项计数", () => {
    render(<Harness onPick={() => {}}/>);
    fireEvent.contextMenu(screen.getByTestId("target"));
    expect(screen.getAllByRole("menuitem")).toHaveLength(3);
    expect(document.querySelectorAll(".context-menu hr")).toHaveLength(1);
  });

  it("分隔线出现在它声明的位置上", () => {
    render(<Harness onPick={() => {}}/>);
    fireEvent.contextMenu(screen.getByTestId("target"));
    const nodes = [...(document.querySelector(".context-menu")?.children ?? [])].map((node) => node.tagName.toLowerCase());
    expect(nodes).toEqual(["button", "button", "hr", "button"]);
  });
});
