import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { useState } from "react";
import { WindowsSandboxProvider, useWindowsSandbox } from "./use-windows-sandbox";
const invoke = vi.hoisted(()=>vi.fn());
vi.mock("./host",()=>({invoke}));
const ready = {supported:true,status:"ready",errorCode:null,errorMessage:null};
function Probe(props:{name:string;platform?:string}) {
  const value = useWindowsSandbox(props.platform ?? "windows");
  return <div><span>{props.name}:{value.loading ? "loading" : value.status?.status ?? "none"}</span>
    <button onClick={()=>void value.refresh()}>{props.name}检查</button>
    <button onClick={()=>void value.setup("C:\\project")}>{props.name}初始化</button></div>;
}
beforeEach(()=>{invoke.mockReset().mockResolvedValue(ready);});
afterEach(cleanup);

test("对话与设置并发只读取一次，卸载消费者再切回复用结果", async () => {
  const shell = (settings: boolean) => <WindowsSandboxProvider scopeKey="profile/server">
    {settings ? <Probe name="设置"/> : <><Probe name="对话"/><Probe name="第二页"/></>}
  </WindowsSandboxProvider>;
  const view = render(shell(false));
  await screen.findByText("对话:ready");
  expect(invoke).toHaveBeenCalledTimes(1);
  view.rerender(shell(true));
  await screen.findByText("设置:ready");
  expect(invoke).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole("button",{name:"设置检查"}));
  await waitFor(()=>expect(invoke).toHaveBeenCalledTimes(2));
});

test("启动检查发现未初始化时自动初始化一次，结果同步给所有页面", async () => {
  invoke.mockResolvedValueOnce({...ready,status:"notConfigured"});
  render(<WindowsSandboxProvider scopeKey="one"><Probe name="对话"/><Probe name="设置"/></WindowsSandboxProvider>);
  await screen.findByText("对话:ready");
  expect(screen.getByText("设置:ready")).toBeTruthy();
  expect(invoke.mock.calls.map(([command])=>command)).toEqual(["ai_windows_sandbox_status","ai_windows_sandbox_setup"]);
  expect(invoke.mock.calls[1][1]).toEqual({workspaceRoot:null});
});

test("凭据失效自动修复被取消后不再自动请求，手动初始化仍可用", async () => {
  const stale = {...ready,status:"updateRequired",errorCode:"windows_sandbox_credentials_invalid",errorMessage:"凭据已失效"};
  invoke.mockResolvedValueOnce(stale).mockRejectedValueOnce(new Error("windows_sandbox_setup_failed:UAC was cancelled")).mockResolvedValueOnce(stale);
  render(<WindowsSandboxProvider scopeKey="one"><Probe name="设置"/></WindowsSandboxProvider>);
  await waitFor(()=>expect(invoke).toHaveBeenCalledTimes(2));
  await screen.findByText("设置:updateRequired");
  fireEvent.click(screen.getByRole("button",{name:"设置检查"}));
  await waitFor(()=>expect(invoke).toHaveBeenCalledTimes(3));
  await act(async()=>{await Promise.resolve();});
  expect(invoke.mock.calls.map(([command])=>command)).toEqual(["ai_windows_sandbox_status","ai_windows_sandbox_setup","ai_windows_sandbox_status"]);
  fireEvent.click(screen.getByRole("button",{name:"设置初始化"}));
  await screen.findByText("设置:ready");
  expect(invoke.mock.calls[3]).toEqual(["ai_windows_sandbox_setup",{workspaceRoot:"C:\\project"}]);
});

test("账户或服务切换清空状态，旧请求的迟到结果不能写入新范围", async () => {
  let resolveOld!: (value:typeof ready)=>void;
  invoke.mockImplementationOnce(()=>new Promise(resolve=>{resolveOld=resolve;}));
  const shell = (scope:string) => <WindowsSandboxProvider scopeKey={scope}><Probe name="状态"/></WindowsSandboxProvider>;
  const view = render(shell("account-one/server-one"));
  await waitFor(()=>expect(invoke).toHaveBeenCalledTimes(1));
  view.rerender(shell("account-two/server-two"));
  await screen.findByText("状态:ready");
  await act(async()=>{resolveOld({...ready,status:"error"});});
  expect(screen.getByText("状态:ready")).toBeTruthy();
  expect(invoke).toHaveBeenCalledTimes(2);
});

test("失败保留明确状态，切页不自动重试，手动检查可恢复", async () => {
  invoke.mockRejectedValueOnce(new Error("windows_sandbox_setup_failed"));
  const shell = (name:string) => <WindowsSandboxProvider scopeKey="one"><Probe key={name} name={name}/></WindowsSandboxProvider>;
  const view=render(shell("对话"));
  await screen.findByText("对话:error");
  view.rerender(shell("设置"));
  await screen.findByText("设置:error");
  expect(invoke).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole("button",{name:"设置检查"}));
  await screen.findByText("设置:ready");
});

test("非 Windows 页面不发起沙箱请求，禁用阶段等待服务恢复", async () => {
  const view=render(<WindowsSandboxProvider scopeKey="one" enabled={false}><Probe name="状态"/></WindowsSandboxProvider>);
  await act(async()=>{await Promise.resolve();});
  expect(invoke).not.toHaveBeenCalled();
  view.rerender(<WindowsSandboxProvider scopeKey="one" enabled><Probe name="状态" platform="macos"/></WindowsSandboxProvider>);
  await act(async()=>{await Promise.resolve();});
  expect(invoke).not.toHaveBeenCalled();
  view.rerender(<WindowsSandboxProvider scopeKey="one" enabled><Probe name="状态"/></WindowsSandboxProvider>);
  await screen.findByText("状态:ready");
  expect(invoke).toHaveBeenCalledTimes(1);
});
test("重启服务只使沙箱状态失效，保留当前页面的输入", async () => {
  function Draft() { const [value,setValue] = useState(""); return <input aria-label="草稿" value={value} onChange={event=>setValue(event.target.value)}/>; }
  const shell = (scope:string) => <WindowsSandboxProvider scopeKey={scope}><Draft/><Probe name="状态"/></WindowsSandboxProvider>;
  const view=render(shell("account/server-one"));
  await screen.findByText("状态:ready");
  const input=screen.getByRole("textbox");
  fireEvent.change(input,{target:{value:"服务重启前的草稿"}});
  view.rerender(shell("account/server-two"));
  await screen.findByText("状态:ready");
  expect(screen.getByRole("textbox")).toBe(input);
  expect(input).toHaveProperty("value","服务重启前的草稿");
  expect(invoke).toHaveBeenCalledTimes(2);
});
