import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { RichMenu, RichMenuItem, RichMenuSection, RichMenuSeparator } from "./rich-menu";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function Fixture(props: { onSecond?: () => void }) {
  const [value, setValue] = useState("first");
  return <RichMenu
    ariaLabel="公共富菜单"
    shortcutActions={{ "2": () => { setValue("second"); props.onSecond?.(); } }}
    trigger={({ open, toggle, triggerRef }) => <button aria-expanded={open} onClick={toggle} ref={triggerRef} type="button">打开菜单</button>}>
    <RichMenuSection label="选项">
      <RichMenuItem label="第一项" onSelect={() => setValue("first")} selected={value === "first"}/>
      <RichMenuItem description="统一描述" label="第二项" onSelect={() => { setValue("second"); props.onSecond?.(); }} selected={value === "second"}/>
    </RichMenuSection>
    <RichMenuSeparator/>
    <RichMenuItem action label="新增" onSelect={() => undefined}/>
  </RichMenu>;
}

describe("RichMenu", () => {
  test("操作菜单键盘跳过禁用项并在退出或选择后恢复焦点", () => {
    const selected = vi.fn();
    render(<RichMenu ariaLabel="操作" keyboardNavigation trigger={({ toggle, triggerRef }) => <button ref={triggerRef} onClick={toggle}>更多</button>}>
      <RichMenuItem label="不可用" disabled onSelect={() => undefined}/>
      <RichMenuItem label="管理" onSelect={selected}/>
      <RichMenuItem label="导出" onSelect={() => undefined}/>
    </RichMenu>);
    const trigger = screen.getByRole("button", { name: "更多" });
    fireEvent.click(trigger);
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "管理" }));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "导出" }));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "管理" }));
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(document.activeElement).toBe(trigger);
    expect(screen.queryByRole("menu")).toBeNull();
    fireEvent.click(trigger);
    fireEvent.click(screen.getByRole("menuitem", { name: "管理" }));
    expect(selected).toHaveBeenCalledOnce();
    expect(document.activeElement).toBe(trigger);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  test("统一选中态、条目语义并在选择后关闭", () => {
    const onSecond = vi.fn();
    render(<Fixture onSecond={onSecond}/>);
    fireEvent.click(screen.getByRole("button", { name: "打开菜单" }));
    expect(screen.getByRole("menu", { name: "公共富菜单" })).toBeTruthy();
    expect(screen.getByRole("menuitemradio", { name: "第一项" }).getAttribute("data-selected")).toBe("true");
    fireEvent.click(screen.getByRole("menuitemradio", { name: /第二项/ }));
    expect(onSecond).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menu", { name: "公共富菜单" })).toBeNull();
  });

  test("支持快捷键和 Escape 关闭", () => {
    const onSecond = vi.fn();
    render(<Fixture onSecond={onSecond}/>);
    fireEvent.click(screen.getByRole("button", { name: "打开菜单" }));
    fireEvent.keyDown(window, { key: "2" });
    expect(onSecond).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menu", { name: "公共富菜单" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "打开菜单" }));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("menu", { name: "公共富菜单" })).toBeNull();
  });

  test("内容未超过可用高度时将边框计入菜单高度", () => {
    render(<Fixture/>);
    fireEvent.click(screen.getByRole("button", { name: "打开菜单" }));
    const menu = screen.getByRole("menu", { name: "公共富菜单" });
    Object.defineProperty(menu, "scrollHeight", { configurable: true, value: 120 });
    vi.spyOn(window, "getComputedStyle").mockReturnValue({
      borderTopWidth: "1px",
      borderBottomWidth: "1px",
    } as CSSStyleDeclaration);

    fireEvent(window, new Event("resize"));

    expect(menu.style.maxHeight).toBe("122px");
  });
});
