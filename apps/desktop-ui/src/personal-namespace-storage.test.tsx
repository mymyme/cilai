import {act,cleanup,fireEvent,render,screen} from "@testing-library/react";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {PersonalNamespaceStorage,type PersonalNamespaceInventory} from "./personal-namespace-storage";
const invoke=vi.hoisted(()=>vi.fn());
vi.mock("./host",()=>({invoke}));
afterEach(cleanup);
const profileId="1".repeat(32);
function inventory():PersonalNamespaceInventory {
  const scopes=["namespace/logs","namespace/test-diagnostics","browser/http-cache"];
  return {profileId,scannedAt:1,observation:"live_metadata",sizeComplete:false,knownLogicalBytes:8,
    items:scopes.map((scope,index)=>({itemId:`shell:${scope}`,category:index===2?"cache":"logs",displayPath:scope,managedRelativePath:null,logicalBytes:index===2?null:index===0?8:0,fileCount:index===2?null:1,snapshotDigest:null,cleanupAllowed:false,reasons:[],references:[]})),
    coverage:scopes.map((scope,index)=>({category:index===2?"cache":"logs",scope,state:index===2?"not_scanned":"covered",reason:index===2?"personal_storage_browser_not_initialized":null})),issues:["personal_storage_browser_not_initialized"]};
}
beforeEach(()=>{invoke.mockReset();invoke.mockImplementation(async command=>command==="personal_namespace_storage_list"?[]:inventory());});
it("未初始化缓存显示未知并独立记录本组已知占用",async()=>{
  render(<PersonalNamespaceStorage profileId={profileId}/>);
  await screen.findByText("本组已知占用：8 字节");
  expect(screen.getByText("浏览器 HTTP 缓存：大小未知 · 未统计")).toBeTruthy();
  expect(invoke).toHaveBeenCalledWith("personal_namespace_storage_inventory",{});
  expect(invoke.mock.calls.every(([command])=>["personal_namespace_storage_inventory","personal_namespace_storage_list"].includes(command))).toBe(true);
  expect(screen.getByRole("checkbox",{name:"浏览器 HTTP 缓存"}).hasAttribute("disabled")).toBe(true);
});
it("真实零缓存与未统计分开显示",async()=>{
  const value=inventory();value.items[2].logicalBytes=0;value.coverage[2]={...value.coverage[2],state:"covered",reason:null};value.sizeComplete=true;value.issues=[];invoke.mockResolvedValue(value);
  render(<PersonalNamespaceStorage profileId={profileId}/>);await screen.findByText("本组占用：8 字节");
  expect(screen.getByText("浏览器 HTTP 缓存：0 字节")).toBeTruthy();
});
it("统计失败不显示零或错误原文，刷新只再读元数据",async()=>{
  invoke.mockRejectedValue(new Error("secret body"));render(<PersonalNamespaceStorage profileId={profileId}/>);
  await screen.findByText("统计未完成，请刷新核对。");expect(screen.queryByText(/本组.*占用/)).toBeNull();expect(screen.queryByText(/secret body/)).toBeNull();
  invoke.mockImplementation(async command=>command==="personal_namespace_storage_list"?[]:inventory());fireEvent.click(screen.getByRole("button",{name:"刷新宿主占用"}));await screen.findByText("本组已知占用：8 字节");
  expect(invoke.mock.calls.every(([command,args])=>["personal_namespace_storage_inventory","personal_namespace_storage_list"].includes(command)&&Object.keys(args).length<2)).toBe(true);
});
it("其他资料返回值不能混入当前占用",async()=>{
  invoke.mockResolvedValue({...inventory(),profileId:"2".repeat(32)});render(<PersonalNamespaceStorage profileId={profileId}/>);
  await screen.findByText("统计未完成，请刷新核对。");expect(screen.queryByText(/本组.*占用/)).toBeNull();
});
it("在途切换资料不展示旧统计",async()=>{
  let resolve!:(value:PersonalNamespaceInventory)=>void;
  invoke.mockImplementationOnce(()=>new Promise<PersonalNamespaceInventory>(done=>{resolve=done;}));
  const view=render(<PersonalNamespaceStorage profileId={profileId}/>);
  const other={...inventory(),profileId:"2".repeat(32),knownLogicalBytes:16};invoke.mockResolvedValue(other);
  view.rerender(<PersonalNamespaceStorage profileId={other.profileId}/>);await screen.findByText("本组已知占用：16 字节");
  await act(async()=>resolve(inventory()));expect(screen.queryByText("本组已知占用：8 字节")).toBeNull();
});
