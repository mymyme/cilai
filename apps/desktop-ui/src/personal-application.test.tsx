import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { PersonalApplication } from "./personal-application";

const host = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("./host", () => host);
beforeEach(() => { host.invoke.mockReset(); host.invoke.mockImplementation(async (command) => command === "personal_document_create" ? "app://personal-capability/document-1" : { tools: [{ name: "save" }] }); });
afterEach(cleanup);
const props = { projectId: "project1", packageDigest: "sha256:immutable", html: "<h1>我的应用</h1>", title: "我的应用", active: true };
async function open() {
  render(<PersonalApplication {...props}/>);
  const frame = await screen.findByTitle("我的应用") as HTMLIFrameElement;
  const channel = host.invoke.mock.calls.find(([command]) => command === "personal_document_create")![1].html.match(/const channel="([^"]+)"/)![1];
  return { frame, channel };
}
function request(frame: HTMLIFrameElement, channel: string, source: Window | null = frame.contentWindow) {
  act(() => window.dispatchEvent(new MessageEvent("message", { source, data: { type: "cilai-personal-request", channel, id: "1", method: "tools/call", params: { tool: "save", arguments: { text: "确认内容" } } } })));
}
test("只接收自身页面请求，确认后调用精确安装版本", async () => {
  const { frame, channel } = await open();
  expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
  expect(frame.srcdoc).toBe("");
  expect(frame.src).toBe("app://personal-capability/document-1");
  request(frame, channel, window);
  request(frame, "other-channel");
  expect(screen.queryByRole("dialog")).toBeNull();
  request(frame, channel);
  expect(await screen.findByRole("dialog", { name: "运行个人工具" })).toBeTruthy();
  expect(host.invoke).toHaveBeenCalledTimes(2);
  fireEvent.click(screen.getByRole("button", { name: "运行" }));
  await waitFor(() => expect(host.invoke).toHaveBeenCalledWith("personal_capability_call", { creationProjectId: "project1", expectedDigest: "sha256:immutable", tool: "save", arguments: { text: "确认内容" } }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
});
test("取消工具请求不执行，安装校验失败不加载页面", async () => {
  const { frame, channel } = await open();
  request(frame, channel);
  fireEvent.click(await screen.findByRole("button", { name: "取消" }));
  expect(host.invoke).toHaveBeenCalledTimes(2);
  cleanup(); host.invoke.mockRejectedValue(new Error("个人能力版本已变化"));
  render(<PersonalApplication {...props}/>);
  expect((await screen.findByRole("alert")).textContent).toContain("个人能力版本已变化");
  expect(screen.queryByTitle("我的应用")).toBeNull();
});
test("页面只能使用自己选定的文件，调用和写入保留版本绑定",async()=>{
  const {frame,channel}=await open();
  const send=(id:string,method:string,params:unknown)=>act(()=>window.dispatchEvent(new MessageEvent("message",{source:frame.contentWindow,data:{type:"cilai-personal-request",channel,id,method,params}})));
  host.invoke.mockResolvedValue({grantId:"selected-1",name:"record.txt",packageDigest:props.packageDigest});
  send("2","files/select",{});await waitFor(()=>expect(host.invoke).toHaveBeenLastCalledWith("personal_capability_select_file",{creationProjectId:props.projectId}));
  await act(async()=>{await Promise.resolve();});
  send("3","tools/call",{tool:"inspect",arguments:{},fileGrantId:"forged"});expect(screen.queryByRole("dialog")).toBeNull();
  send("4","tools/call",{tool:"inspect",arguments:{},fileGrantId:"selected-1"});fireEvent.click(await screen.findByRole("button",{name:"运行"}));
  await waitFor(()=>expect(host.invoke).toHaveBeenLastCalledWith("personal_capability_call",expect.objectContaining({expectedDigest:props.packageDigest,fileGrantId:"selected-1"})));
  await waitFor(()=>expect(screen.queryByRole("dialog")).toBeNull());
  send("5","files/write",{grantId:"selected-1",content:"整理结果"});await waitFor(()=>expect(host.invoke).toHaveBeenLastCalledWith("personal_capability_confirm_file_write",{creationProjectId:props.projectId,packageDigest:props.packageDigest,grantId:"selected-1",content:"整理结果"}));
});


test("关闭页面释放文档，未完成的加载不能留下页面", async () => {
  await open();
  cleanup();
  expect(host.invoke).toHaveBeenCalledWith("personal_document_release", { url: "app://personal-capability/document-1" });
  let resolveDocument!: (url: string) => void;
  host.invoke.mockImplementation(async (command) => command === "personal_document_create"
    ? new Promise<string>(resolve => { resolveDocument = resolve; }) : { tools: [] });
  const view = render(<PersonalApplication {...props}/>);
  await waitFor(() => expect(resolveDocument).toBeTypeOf("function"));
  view.unmount();
  await act(async () => resolveDocument("app://personal-capability/late-document"));
  expect(host.invoke).toHaveBeenCalledWith("personal_document_release", { url: "app://personal-capability/late-document" });
});

test("页面注册失败明确报错，不改用内联页面", async () => {
  host.invoke.mockImplementation(async command => {
    if (command === "personal_document_create") throw new Error("应用页面超过加载限制");
    return { tools: [] };
  });
  render(<PersonalApplication {...props}/>);
  expect((await screen.findByRole("alert")).textContent).toContain("应用页面超过加载限制");
  expect(screen.queryByTitle("我的应用")).toBeNull();
});
