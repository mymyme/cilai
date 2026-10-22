import {cleanup,fireEvent,render,screen,waitFor,within} from "@testing-library/react";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import type {PersonalStorageInventory,PersonalStorageItem,PersonalStorageOperation} from "./generated/protocol";
import {PersonalStorageSettings,storageMessage} from "./personal-storage";
const invoke=vi.hoisted(()=>vi.fn());
vi.mock("./host",()=>({invoke}));
afterEach(cleanup);
const profileId="1".repeat(32);
const cache=():PersonalStorageItem=>({itemId:"cache-item",category:"cache",displayPath:"cache/preview",managedRelativePath:"cache/preview",logicalBytes:30,fileCount:1,snapshotDigest:"digest",cleanupAllowed:true,reasons:[],references:[]});
const inventory=():PersonalStorageInventory=>({profileId,profilePath:"/personal/profile",scannedAt:1,knownLogicalBytes:30,sizeComplete:false,activeWriterCount:0,coverage:[{category:"project_originals",scope:"/external/project",state:"external_reference",reason:null}],items:[cache()],issues:[]});
const operation=():PersonalStorageOperation=>({operationId:"storage-op",profileId,state:"prepared",previewDigest:"preview-digest",createdAt:1,updatedAt:1,selectedItems:[cache()],selectedLogicalBytes:30,requiresQuiescence:true,blockers:[],result:null});
let usage:PersonalStorageInventory,records:PersonalStorageOperation[];
beforeEach(()=>{usage=inventory();records=[];invoke.mockReset();invoke.mockImplementation(async(command:string)=>{
  if(command==="personal_storage_inventory")return structuredClone(usage);
  if(command==="personal_storage_list")return structuredClone(records);
  if(command==="personal_storage_prepare")return operation();
  throw new Error("unconfirmed");
});});
it("盘点标明已知占用与外部引用，不自动清理",async()=>{
  render(<PersonalStorageSettings profileId={profileId}/>);await screen.findByText(/已知占用/);
  expect(screen.getByText(/仅外部引用/)).toBeTruthy();
  expect(invoke.mock.calls.map(call=>call[0]).sort()).toEqual(["personal_namespace_storage_inventory","personal_storage_inventory","personal_storage_list"].sort());
});
it("当前版本和业务资料无法选择清理",async()=>{
  usage.items=[{...cache(),category:"application_data",cleanupAllowed:false,reasons:["business_data_protected"],references:[{kind:"current_application",objectId:"local"}]}];
  render(<PersonalStorageSettings profileId={profileId}/>);await screen.findByText("业务资料受保护。");
  expect((screen.getByRole("checkbox") as HTMLInputElement).disabled).toBe(true);
  expect((screen.getByRole("button",{name:"预览清理"}) as HTMLButtonElement).disabled).toBe(true);
});
it("清理预览列出冻结范围，取消不停止任务或删除内容",async()=>{
  render(<PersonalStorageSettings profileId={profileId}/>);fireEvent.click(await screen.findByRole("checkbox"));fireEvent.click(screen.getByRole("button",{name:"预览清理"}));
  const dialog=await screen.findByRole("dialog");expect(dialog.textContent).toContain("cache/preview");expect(dialog.textContent).toContain("30 字节");
  fireEvent.click(within(dialog).getByRole("button",{name:"取消"}));
  expect(invoke.mock.calls.some(call=>call[0]==="personal_storage_execute")).toBe(false);
});
it("未知清理回执不自动重试，刷新只盘点已有结果",async()=>{
  records=[operation()];render(<PersonalStorageSettings profileId={profileId}/>);
  fireEvent.click(await screen.findByRole("button",{name:"核对清理"}));fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"清理"}));
  await screen.findByRole("alert");expect(invoke.mock.calls.filter(call=>call[0]==="personal_storage_execute")).toHaveLength(1);
  expect((screen.getByRole("checkbox") as HTMLInputElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole("button",{name:"刷新占用"}));await waitFor(()=>expect((screen.getByRole("checkbox") as HTMLInputElement).disabled).toBe(false));
  expect(invoke.mock.calls.filter(call=>call[0]==="personal_storage_execute")).toHaveLength(1);
});
it("部分失败逐项保留，不显示完整清理成功",async()=>{
  records=[operation()];const original=invoke.getMockImplementation()!;
  invoke.mockImplementation(async(command,args)=>command==="personal_storage_execute"?{operation:{...operation(),state:"incomplete",result:{items:[{itemId:"cache-item",outcome:"retained",deletedLogicalBytes:0,deletedFileCount:0,retainedRelativePath:"cache/preview",reason:null}],deletedLogicalBytes:0,deletedFileCount:0,physicalSpaceReleased:null,issues:[]}},maintenanceError:null}:original(command,args));
  render(<PersonalStorageSettings profileId={profileId}/>);fireEvent.click(await screen.findByRole("button",{name:"核对清理"}));fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"清理"}));
  await screen.findByText("未完成");expect(screen.getByText(/cache\/preview：已保留/)).toBeTruthy();
  expect(screen.getByText("实际磁盘释放：未提供")).toBeTruthy();
  expect(screen.queryByText("清理已核对，实际磁盘释放量未提供。")).toBeNull();
  expect(invoke.mock.calls.find(call=>call[0]==="personal_storage_execute")![1].request).not.toHaveProperty("quiescence");
});
it("未知大小与真实零值分开显示",async()=>{
  usage.items=[{...cache(),logicalBytes:null},{...cache(),itemId:"empty",logicalBytes:0}];
  render(<PersonalStorageSettings profileId={profileId}/>);await screen.findByText("缓存 · 大小未知");expect(screen.getByText("缓存 · 0 字节")).toBeTruthy();
});
it("初次盘点失败不显示空占用或给出清理动作",async()=>{
  invoke.mockRejectedValue(new Error("private body text"));render(<PersonalStorageSettings profileId={profileId}/>);
  await screen.findByRole("alert");expect(screen.queryByText(/已知占用/)).toBeNull();
  expect(screen.queryByRole("button",{name:"预览清理"})).toBeNull();
  expect(screen.getByRole("alert").textContent).not.toContain("private body");
});
it("未知错误不回显任意原文或凭据",()=>{
  expect(storageMessage("sensitive key and request body")).toBe("状态尚未确认，请刷新核对。");
});

it("预览发现保护项时不能确认清理",async()=>{
  records=[{...operation(),blockers:["personal_storage_selected_item_protected"]}];
  render(<PersonalStorageSettings profileId={profileId}/>);
  fireEvent.click(await screen.findByRole("button",{name:"核对清理"}));
  const dialog=screen.getByRole("dialog");
  expect((within(dialog).getByRole("button",{name:"清理"}) as HTMLButtonElement).disabled).toBe(true);
  expect(invoke.mock.calls.some(call=>call[0]==="personal_storage_execute")).toBe(false);
});

it("中断未确认的删除量不被显示为零或完整成功",async()=>{
  records=[{...operation(),state:"incomplete",result:{items:[],deletedLogicalBytes:null,deletedFileCount:null,physicalSpaceReleased:null,issues:[]}}];
  render(<PersonalStorageSettings profileId={profileId}/>);await screen.findByText("已删除文件数：未确认 · 逻辑大小未确认");
  expect(screen.queryByText("清理已核对，实际磁盘释放量未提供。")).toBeNull();
});
