import {act,cleanup,fireEvent,render,screen,waitFor,within} from "@testing-library/react";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import type {PersonalMemoryResetOperation} from "./generated/protocol";
import {PersonalMemoryReset} from "./personal-memory-reset";
const invoke=vi.hoisted(()=>vi.fn());
vi.mock("./host",()=>({invoke}));
afterEach(cleanup);
const operation=():PersonalMemoryResetOperation=>({operationId:"memory-reset",profileId:"a".repeat(32),state:"prepared",previewDigest:"digest",createdAt:1,updatedAt:1,blockers:[],scopeResults:[],scopes:["one","two"].map((scopeId,index)=>({scopeId,nativeHomeRelativePath:`managed/${scopeId}`,scopeKind:index===0?"connection_workbench":"connection_application",visibleMemoryFileCount:1,visibleMemoryLogicalBytes:4,inspectionIssue:null}))});
const completed=():PersonalMemoryResetOperation=>({...operation(),state:"completed",scopeResults:operation().scopes.map(scope=>({scopeId:scope.scopeId,outcome:"reset",writerLeaseId:"lease",officialResponseReceived:true,processExitConfirmed:true,reason:null}))});
let records:PersonalMemoryResetOperation[];
beforeEach(()=>{records=[];invoke.mockReset();invoke.mockImplementation(async(command:string)=>{
  if(command==="personal_memory_reset_list")return structuredClone(records);
  if(command==="personal_memory_reset_prepare")return operation();
  if(command==="personal_memory_reset_confirm")return completed();
  throw new Error("unknown");
});});
const props=()=>({disabled:false,onChanged:vi.fn(),onBusy:vi.fn(),onUnconfirmed:vi.fn()});
async function prepare(){fireEvent.click(screen.getByRole("button",{name:"删除"}));return screen.findByRole("dialog");}
it("删除先展示全部作用域，取消不停止任务或调用官方清理",async()=>{
  render(<PersonalMemoryReset {...props()}/>);const dialog=await prepare();
  expect(dialog.textContent).toContain("2 个原生范围");expect(dialog.textContent).toContain("后台记忆");
  fireEvent.click(within(dialog).getByRole("button",{name:"取消"}));
  expect(invoke.mock.calls.some(([command])=>command==="personal_memory_reset_confirm")).toBe(false);
});
it("全部作用域取得回执与退出后才显示完成，确认请求不带proof或目录",async()=>{
  const callbacks=props();render(<PersonalMemoryReset {...callbacks}/>);const dialog=await prepare();fireEvent.click(within(dialog).getByRole("button",{name:"删除"}));
  await screen.findByText("当前运行时记忆已清理，后台记忆已关闭。");
  expect(invoke).toHaveBeenCalledWith("personal_memory_reset_confirm",{request:{operationId:"memory-reset",previewDigest:"digest"}});
  expect(callbacks.onChanged).toHaveBeenCalledTimes(1);
});
it("维护仍在执行时不提前显示完成",async()=>{
  let done!:(value:PersonalMemoryResetOperation)=>void;
  const original=invoke.getMockImplementation()!;invoke.mockImplementation((command,args)=>command==="personal_memory_reset_confirm"?new Promise(resolve=>{done=resolve;}):original(command,args));
  render(<PersonalMemoryReset {...props()}/>);fireEvent.click(within(await prepare()).getByRole("button",{name:"删除"}));
  await waitFor(()=>expect(done).toBeTypeOf("function"));expect(screen.queryByText("当前运行时记忆已清理，后台记忆已关闭。")).toBeNull();
  expect(within(screen.getByRole("dialog")).getByRole("button",{name:"处理中…"})).toBeTruthy();
  await act(async()=>done(completed()));await screen.findByText("当前运行时记忆已清理，后台记忆已关闭。");
});
it("未知回执锁住新删除，刷新只读取已有记录",async()=>{
  const original=invoke.getMockImplementation()!;invoke.mockImplementation((command,args)=>command==="personal_memory_reset_confirm"?Promise.reject(new Error("private response")):original(command,args));
  const callbacks=props();render(<PersonalMemoryReset {...callbacks}/>);fireEvent.click(within(await prepare()).getByRole("button",{name:"删除"}));
  await screen.findByText("删除结果尚未确认，请刷新核对原操作。");expect((screen.getByRole("button",{name:"删除"}) as HTMLButtonElement).disabled).toBe(true);
  expect(callbacks.onUnconfirmed).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole("button",{name:"刷新删除记录"}));await waitFor(()=>expect((screen.getByRole("button",{name:"删除"}) as HTMLButtonElement).disabled).toBe(false));
  expect(invoke.mock.calls.filter(([command])=>command==="personal_memory_reset_confirm")).toHaveLength(1);
  expect(screen.queryByText(/private response/)).toBeNull();
});
it("部分失败保留逐范围结果，不称全部删除",async()=>{
  const result=completed();result.state="incomplete";result.scopeResults[1]={...result.scopeResults[1],outcome:"failed",officialResponseReceived:false,reason:"personal_memory_reset_response_unconfirmed"};
  const original=invoke.getMockImplementation()!;invoke.mockImplementation((command,args)=>command==="personal_memory_reset_confirm"?Promise.resolve(result):original(command,args));
  render(<PersonalMemoryReset {...props()}/>);fireEvent.click(within(await prepare()).getByRole("button",{name:"删除"}));
  await screen.findByText("删除未完成，请核对各项结果。");expect(screen.getByText(/页面 AI：失败/)).toBeTruthy();
  expect(screen.queryByText("当前运行时记忆已清理，后台记忆已关闭。")).toBeNull();
});
it("缺少范围或退出证据的完成状态仍视为未知",async()=>{
  const result=completed();result.scopeResults.pop();const original=invoke.getMockImplementation()!;
  invoke.mockImplementation((command,args)=>command==="personal_memory_reset_confirm"?Promise.resolve(result):original(command,args));
  render(<PersonalMemoryReset {...props()}/>);fireEvent.click(within(await prepare()).getByRole("button",{name:"删除"}));
  await screen.findByText("官方清理回执尚未确认，请核对原操作。");expect(screen.queryByText("当前运行时记忆已清理，后台记忆已关闭。")).toBeNull();
});
it("中断记录只能明确核对，不重发清理",async()=>{
  records=[{...operation(),state:"applying"}];render(<PersonalMemoryReset {...props()}/>);
  fireEvent.click(await screen.findByRole("button",{name:"核对中断删除"}));fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"核对"}));
  await screen.findByText("删除结果尚未确认，请刷新核对原操作。");
  expect(invoke.mock.calls.filter(([command])=>command==="personal_memory_reset_resolve_interrupted")).toHaveLength(1);
  expect(invoke.mock.calls.some(([command])=>command==="personal_memory_reset_confirm")).toBe(false);
});
