import {cleanup,fireEvent,render,screen,waitFor,within} from "@testing-library/react";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import type {CapabilityCreationProject,PersonalApplicationDataOperation} from "./generated/protocol";
import {PersonalApplicationData} from "./personal-application-data";
const invoke=vi.hoisted(()=>vi.fn());vi.mock("./host",()=>({invoke}));afterEach(cleanup);
const project:CapabilityCreationProject={creationProjectId:"app",title:"个人应用",businessGoal:"管理本地记录",targetType:"local_mcp",status:"installed",conversationId:null,managedRelativePath:"managed",requestedPermissions:[],acceptanceExamples:[],sourceDigest:"source",sourceFileCount:1,stateRevision:3,packageDigest:"new-package",errorReason:null,createdAt:1,updatedAt:1};
const upgrade=():PersonalApplicationDataOperation=>({readOnlyAfterRestore:false,sourceProfileId:null,operationId:"upgrade",profileId:"a".repeat(32),creationProjectId:"app",clientOperationId:"once",expectedRevision:2,packageDigest:"new-package",sourcePackageDigest:"old-package",sourceSlotId:"old-slot",targetSlotId:"new-slot",action:"upgrade",state:"completed",previewDigest:"digest",snapshotDigest:"snapshot",reason:null,activatedRevision:3,createdAt:1,updatedAt:1});
const rollback=():PersonalApplicationDataOperation=>({...upgrade(),operationId:"rollback",packageDigest:"old-package",sourcePackageDigest:"new-package",sourceSlotId:"new-slot",targetSlotId:"old-slot",expectedRevision:3,action:"rollback",state:"prepared",activatedRevision:null});
const props=()=>({project,disabled:false,onBusy:vi.fn(),onChanged:vi.fn(async()=>{})});
let records:PersonalApplicationDataOperation[];
beforeEach(()=>{records=[upgrade()];invoke.mockReset();invoke.mockImplementation(async(command:string)=>{
  if(command==="personal_application_data_list")return structuredClone(records);
  if(command==="personal_application_data_prepare")return rollback();
  if(command==="personal_application_data_rollback")return {...rollback(),state:"completed",activatedRevision:4};
  throw new Error("unknown");
});});
it("读取版本历史不自动复制数据或回退",async()=>{
  render(<PersonalApplicationData {...props()}/>);await screen.findByRole("button",{name:"准备回退"});
  expect(invoke).toHaveBeenCalledExactlyOnceWith("personal_application_data_list",{request:{}});
});
it("准备冻结旧版本与当前修订，取消不执行切换",async()=>{
  render(<PersonalApplicationData {...props()}/>);fireEvent.click(await screen.findByRole("button",{name:"准备回退"}));
  const dialog=await screen.findByRole("dialog");expect(dialog.textContent).toContain("old-package");expect(dialog.textContent).toContain("当前版本和数据保留");
  expect(invoke).toHaveBeenCalledWith("personal_application_data_prepare",{request:{creationProjectId:"app",expectedRevision:3,packageDigest:"old-package",clientOperationId:expect.any(String),action:"rollback",targetOperationId:"upgrade"}});
  fireEvent.click(within(dialog).getByRole("button",{name:"取消"}));expect(invoke.mock.calls.some(([command])=>command==="personal_application_data_rollback")).toBe(false);
});
it("确认后只提交预览标识，完成后刷新实际应用状态",async()=>{
  const callbacks=props();render(<PersonalApplicationData {...callbacks}/>);fireEvent.click(await screen.findByRole("button",{name:"准备回退"}));
  fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button",{name:"回退"}));await screen.findByText("已切换到保留的版本与数据。");
  expect(invoke).toHaveBeenCalledWith("personal_application_data_rollback",{request:{operationId:"rollback",previewDigest:"digest"}});expect(callbacks.onChanged).toHaveBeenCalledTimes(1);
});
it("回退未知不再提交，刷新只读取回执",async()=>{
  const original=invoke.getMockImplementation()!;invoke.mockImplementation((command,args)=>command==="personal_application_data_rollback"?Promise.reject(new Error("private detail")):original(command,args));
  render(<PersonalApplicationData {...props()}/>);fireEvent.click(await screen.findByRole("button",{name:"准备回退"}));fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button",{name:"回退"}));
  await screen.findByText("版本切换结果尚未确认，请刷新核对原操作。");expect((screen.getByRole("button",{name:"准备回退"}) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole("button",{name:"刷新记录"}));await waitFor(()=>expect((screen.getByRole("button",{name:"准备回退"}) as HTMLButtonElement).disabled).toBe(false));
  expect(invoke.mock.calls.filter(([command])=>command==="personal_application_data_rollback")).toHaveLength(1);expect(screen.queryByText(/private detail/)).toBeNull();
});
it("恢复来源历史保持原完成状态且只读",async()=>{
  records=[{...upgrade(),readOnlyAfterRestore:true,sourceProfileId:"b".repeat(32)}];render(<PersonalApplicationData {...props()}/>);await screen.findByText("恢复来源历史，仅供查看。");
  expect(screen.getByText("安装或升级 · 已完成")).toBeTruthy();expect(screen.queryByRole("button",{name:"准备回退"})).toBeNull();
});
it("保留数据变化时不称切换成功",async()=>{
  const original=invoke.getMockImplementation()!;invoke.mockImplementation((command,args)=>command==="personal_application_data_rollback"?Promise.reject(new Error("personal_application_data_retained_changed")):original(command,args));
  const callbacks=props();render(<PersonalApplicationData {...callbacks}/>);fireEvent.click(await screen.findByRole("button",{name:"准备回退"}));fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button",{name:"回退"}));
  await screen.findByText("保留的旧版数据已变化，未执行回退。");expect(callbacks.onChanged).not.toHaveBeenCalled();expect(screen.queryByText("已切换到保留的版本与数据。")).toBeNull();
});
it("中断核对不再次安装或切换",async()=>{
  records=[{...rollback(),state:"applying"}];render(<PersonalApplicationData {...props()}/>);fireEvent.click(await screen.findByRole("button",{name:"核对中断操作"}));fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"核对"}));
  await screen.findByText("版本切换结果尚未确认，请刷新核对原操作。");expect(invoke.mock.calls.filter(([command])=>command==="personal_application_data_resolve_interrupted")).toHaveLength(1);expect(invoke.mock.calls.some(([command])=>command==="personal_application_data_rollback")).toBe(false);
});
