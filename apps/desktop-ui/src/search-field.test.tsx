import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SearchField } from "./search-field";

afterEach(cleanup);

describe("SearchField", () => {
  it("由外层承接唯一边框并向外传递输入", () => {
    const onChange = vi.fn();
    const { container } = render(<SearchField ariaLabel="搜索任务" onChange={onChange} placeholder="搜索" value=""/>);
    expect(container.querySelector(".search-field")?.getAttribute("data-focus-frame")).toBe("wrapper");
    fireEvent.change(screen.getByRole("searchbox", { name: "搜索任务" }), { target: { value: "日报" } });
    expect(onChange).toHaveBeenCalledWith("日报");
  });

  it("可清除搜索词", () => {
    const onChange = vi.fn();
    render(<SearchField ariaLabel="搜索任务" clearable onChange={onChange} placeholder="搜索" value="日报"/>);
    fireEvent.click(screen.getByRole("button", { name: "清除搜索任务" }));
    expect(onChange).toHaveBeenCalledWith("");
  });

  it("没有搜索词时保留尾部状态", () => {
    render(<SearchField ariaLabel="全局搜索" clearable onChange={() => {}} placeholder="搜索" trailing={<kbd>⌘ K</kbd>} value=""/>);
    expect(screen.getByText("⌘ K")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "清除全局搜索" })).toBeNull();
  });
});
