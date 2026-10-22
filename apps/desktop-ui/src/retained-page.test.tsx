import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { useEffect, useState } from "react";
import { RetainedPage } from "./retained-page";
import { ConfirmDialog } from "./confirm-dialog";

afterEach(cleanup);

test("只在访问后挂载，保留输入、DOM 和预览资源，账户退出才释放", () => {
  const mount = vi.fn(), release = vi.fn();
  function Content() {
    const [value,setValue] = useState("");
    useEffect(() => { mount(); return release; },[]);
    return <input aria-label="页面草稿" value={value} onChange={event=>setValue(event.target.value)}/>;
  }
  const page = (active: boolean, scope = "one") => <RetainedPage key={scope} active={active}><Content/></RetainedPage>;
  const view = render(page(false));
  expect(mount).not.toHaveBeenCalled();
  view.rerender(page(true));
  const input = screen.getByRole("textbox");
  fireEvent.change(input,{target:{value:"保留的草稿"}});
  view.rerender(page(false));
  expect(screen.queryByRole("textbox")).toBeNull();
  expect(release).not.toHaveBeenCalled();
  view.rerender(page(true));
  expect(screen.getByRole("textbox")).toBe(input);
  expect(input).toHaveProperty("value","保留的草稿");
  expect(mount).toHaveBeenCalledTimes(1);
  view.rerender(page(true,"two"));
  expect(release).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("textbox")).toHaveProperty("value","");
});

test("隐藏设置父页面时不预加载未访问的子页面", () => {
  const mount = vi.fn();
  function Content() { useEffect(()=>{mount();},[]); return <p>子页面</p>; }
  const page = (active: boolean) => <RetainedPage active={active}><RetainedPage active><Content/></RetainedPage></RetainedPage>;
  const view = render(page(false));
  expect(mount).not.toHaveBeenCalled();
  view.rerender(page(true));
  expect(mount).toHaveBeenCalledTimes(1);
  view.rerender(page(false));
  expect(screen.queryByText("子页面")?.closest("[hidden]")).not.toBeNull();
});

test("隐藏页的 body 弹窗及键盘焦点陷阱不影响当前页", () => {
  const cancel = vi.fn();
  const page = (active: boolean) => <><RetainedPage active={active}>
    <ConfirmDialog title="保留的确认" onCancel={cancel} onConfirm={()=>{}} confirmLabel="确定"/>
  </RetainedPage><input aria-label="当前页输入"/></>;
  const view = render(page(true));
  expect(screen.getByRole("dialog")).toBeTruthy();
  view.rerender(page(false));
  expect(screen.queryByRole("dialog")).toBeNull();
  const input = screen.getByRole("textbox"); input.focus();
  fireEvent.keyDown(window,{key:"Escape"});
  expect(cancel).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(input);
  view.rerender(page(true));
  expect(screen.getByRole("dialog")).toBeTruthy();
  fireEvent.keyDown(window,{key:"Escape"});
  expect(cancel).toHaveBeenCalledTimes(1);
});

test("切走后暂停页面中的媒体播放", () => {
  const view = render(<RetainedPage active><video data-testid="media"/></RetainedPage>);
  const pause = vi.fn();
  Object.defineProperty(screen.getByTestId("media"),"pause",{value:pause});
  view.rerender(<RetainedPage active={false}><video data-testid="media"/></RetainedPage>);
  expect(pause).toHaveBeenCalledTimes(1);
});
