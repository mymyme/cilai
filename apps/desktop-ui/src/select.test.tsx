import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Select } from "./select";

afterEach(cleanup);

const options = [
  { value: "low", label: "低" },
  { value: "medium", label: "中" },
  { value: "high", label: "高", disabled: true },
  { value: "critical", label: "紧急" },
];

const setup = (over: Partial<Parameters<typeof Select>[0]> = {}) => {
  const onChange = vi.fn();
  render(<Select ariaLabel="优先级" options={options} value="low" onChange={onChange} {...over}/>);
  return { onChange, trigger: () => screen.getByRole("combobox", { name: "优先级" }) };
};

describe("Select", () => {
  it("收起时显示当前值的文案，且不渲染选项列表", () => {
    const { trigger } = setup();
    expect(trigger().textContent).toBe("低");
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(trigger().getAttribute("aria-expanded")).toBe("false");
  });

  it("点击展开后列出全部选项并标记当前项", () => {
    const { trigger } = setup();
    fireEvent.click(trigger());
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["低", "中", "高", "紧急"]);
    expect(screen.getByRole("option", { name: "低" }).getAttribute("aria-selected")).toBe("true");
  });

  it("选中某项后回调该项的值并收起列表", () => {
    const { onChange, trigger } = setup();
    fireEvent.click(trigger());
    fireEvent.click(screen.getByRole("option", { name: "紧急" }));
    expect(onChange).toHaveBeenCalledWith("critical");
    expect(screen.queryAllByRole("option")).toHaveLength(0);
  });

  it("禁用的选项点击无效，不会产生回调", () => {
    const { onChange, trigger } = setup();
    fireEvent.click(trigger());
    fireEvent.click(screen.getByRole("option", { name: "高" }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it("方向键跳过禁用项", () => {
    const { onChange, trigger } = setup({ value: "medium" });
    fireEvent.click(trigger());
    fireEvent.keyDown(trigger(), { key: "ArrowDown" });
    fireEvent.keyDown(trigger(), { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith("critical");
  });

  it("收起状态按方向键即展开", () => {
    const { trigger } = setup();
    fireEvent.keyDown(trigger(), { key: "ArrowDown" });
    expect(trigger().getAttribute("aria-expanded")).toBe("true");
  });

  it("Esc 收起且不改变取值", () => {
    const { onChange, trigger } = setup();
    fireEvent.click(trigger());
    fireEvent.keyDown(trigger(), { key: "Escape" });
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("点击外部收起", () => {
    const { trigger } = setup();
    fireEvent.click(trigger());
    fireEvent.pointerDown(document.body);
    expect(screen.queryAllByRole("option")).toHaveLength(0);
  });

  it("列表内滚动后仍能点击选项，页面滚动则收起", () => {
    const { onChange, trigger } = setup();
    fireEvent.click(trigger());
    fireEvent.scroll(screen.getByRole("listbox"));
    fireEvent.pointerEnter(screen.getByRole("option", { name: "紧急" }));
    fireEvent.scroll(screen.getByRole("listbox"));
    fireEvent.click(screen.getByRole("option", { name: "紧急" }));
    expect(onChange).toHaveBeenCalledWith("critical");
    fireEvent.click(trigger());
    fireEvent.scroll(window);
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("下拉菜单的 Escape 不传给外层弹窗", () => {
    const onOuterKeyDown = vi.fn();
    render(<div onKeyDown={onOuterKeyDown}><Select ariaLabel="弹窗选择" options={options} value="low" onChange={() => {}}/></div>);
    const trigger = screen.getByRole("combobox", { name: "弹窗选择" });
    fireEvent.click(trigger);
    fireEvent.keyDown(trigger, { key: "Escape" });
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onOuterKeyDown).not.toHaveBeenCalled();
  });

  it("弹窗内的选项属于同一个模态范围，刷新选项时保持展开并允许选择", () => {
    const onChange = vi.fn();
    const view = render(<div aria-modal="true" role="dialog"><Select ariaLabel="模型" options={options} value="low" onChange={onChange}/></div>);
    fireEvent.click(screen.getByRole("combobox", { name: "模型" }));
    expect(screen.getByRole("dialog").contains(screen.getByRole("listbox"))).toBe(true);
    view.rerender(<div aria-modal="true" role="dialog"><Select ariaLabel="模型" options={[...options, { value: "new", label: "新模型" }]} value="low" onChange={onChange}/></div>);
    fireEvent.click(screen.getByRole("option", { name: "新模型" }));
    expect(onChange).toHaveBeenCalledWith("new");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole("combobox", { name: "模型" }));
  });

  it("禁用时点击不展开", () => {
    const { trigger } = setup({ disabled: true });
    fireEvent.click(trigger());
    expect(screen.queryAllByRole("option")).toHaveLength(0);
  });
  it("提交冻结后关闭已展开选项，不能继续改变选择",()=>{
    const onChange=vi.fn();const view=render(<Select ariaLabel="网络" options={options} value="low" onChange={onChange}/>);
    fireEvent.click(screen.getByRole("combobox",{name:"网络"}));expect(screen.getByRole("listbox")).toBeTruthy();
    view.rerender(<Select ariaLabel="网络" options={options} value="low" onChange={onChange} disabled/>);
    expect(screen.queryByRole("listbox")).toBeNull();fireEvent.keyDown(screen.getByRole("combobox",{name:"网络"}),{key:"ArrowDown"});
    expect(onChange).not.toHaveBeenCalled();expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("取值不在选项中时显示占位文案", () => {
    const { trigger } = setup({ value: "", placeholder: "请选择" });
    expect(trigger().textContent).toBe("请选择");
  });
});


it("鼠标和键盘打开选择器都通知读取，关闭及浏览选项不重复通知", () => {
  const onOpen = vi.fn();
  const { trigger } = setup({ onOpen });
  fireEvent.click(trigger());
  expect(onOpen).toHaveBeenCalledTimes(1);
  fireEvent.keyDown(trigger(), { key: "ArrowDown" });
  fireEvent.click(trigger());
  expect(onOpen).toHaveBeenCalledTimes(1);
  fireEvent.keyDown(trigger(), { key: "ArrowDown" });
  expect(onOpen).toHaveBeenCalledTimes(2);
});
