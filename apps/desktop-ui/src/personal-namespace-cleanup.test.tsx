import {cleanup,fireEvent,render,screen,waitFor} from "@testing-library/react";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {PersonalNamespaceCleanup} from "./personal-namespace-cleanup";
import type {PersonalNamespaceInventory} from "./personal-namespace-storage";
import type {PersonalNamespaceStorageOperation} from "./generated/protocol";
const invoke=vi.hoisted(()=>vi.fn());
vi.mock("./host",()=>({invoke}));
afterEach(cleanup);
const profileId="a".repeat(32);
const inventory:PersonalNamespaceInventory={profileId,scannedAt:1,observation:"live_metadata",sizeComplete:true,knownLogicalBytes:10,items:[],issues:[],coverage:[{category:"cache",scope:"browser/http-cache",state:"covered",reason:null}],retainedLogSegments:[]};
const operation=():PersonalNamespaceStorageOperation=>({operationId:"namespace_test",profileId,namespacePath:"/personal",namespaceIdentity:"identity",shellOwner:{instanceId:"shell",pid:1,startFingerprint:"fingerprint"},state:"prepared",previewDigest:"sha256:preview",createdAt:1,updatedAt:1,logSegmentId:null,logFiles:[],browserSession:{partition:`persist:ai-browser-${profileId}`,storagePath:"/personal/browser",cacheBytes:10},actions:[{actionId:"action",kind:"browser_http_cache",state:"planned",outcome:null,reasonCode:null,startedAt:null,acknowledgedAt:null,afterCacheBytes:null}],blockers:[]});
beforeEach(()=>{invoke.mockReset();invoke.mockImplementation(async command=>command==="personal_namespace_storage_list"?[]:operation());});
async function preview(){fireEvent.click(screen.getByRole("checkbox",{name:"浏览器 HTTP 缓存"}));fireEvent.click(screen.getByRole("button",{name:"预览清理"}));await screen.findByRole("button",{name:"确认清理"});}
it("初始只读历史，不自动准备或清理；未扫描缓存不能选择",async()=>{
  render(<PersonalNamespaceCleanup profileId={profileId} inventory={{...inventory,coverage:[{category:"cache",scope:"browser/http-cache",state:"not_scanned",reason:"not_initialized"}]}}/>);
  await waitFor(()=>expect(invoke).toHaveBeenCalledExactlyOnceWith("personal_namespace_storage_list",{request:{}}));
  expect(screen.getByRole("checkbox",{name:"浏览器 HTTP 缓存"}).hasAttribute("disabled")).toBe(true);
  expect(screen.getByRole("button",{name:"预览清理"}).hasAttribute("disabled")).toBe(true);
});
it("用户先选范围再预览，普通界面请求不含目录、PID或静止证明",async()=>{
  render(<PersonalNamespaceCleanup profileId={profileId} inventory={inventory}/>);await preview();
  const sent=invoke.mock.calls.find(([command])=>command==="personal_namespace_storage_prepare")![1];
  expect(Object.keys(sent.request).sort()).toEqual(["clientOperationId","itemKinds","logSegmentId"]);
  expect(sent.request.itemKinds).toEqual(["browser_http_cache"]);
  expect(sent.request.logSegmentId).toBeNull();
  expect(invoke.mock.calls.some(([command])=>command==="personal_namespace_storage_execute")).toBe(false);
});
it("取消只发取消请求，冻结段保持后端真实取消回执",async()=>{
  invoke.mockImplementation(async command=>command==="personal_namespace_storage_list"?[]:command==="personal_namespace_storage_cancel"?{...operation(),state:"cancelled"}:operation());
  render(<PersonalNamespaceCleanup profileId={profileId} inventory={inventory}/>);await preview();
  fireEvent.click(screen.getByRole("button",{name:"取消清理"}));await screen.findByText("已取消");
  expect(invoke).toHaveBeenCalledWith("personal_namespace_storage_cancel",{request:{operationId:"namespace_test",previewDigest:"sha256:preview"}});
  expect(invoke.mock.calls.some(([command])=>command==="personal_namespace_storage_execute")).toBe(false);
});
it("执行结果未知时冻结新操作，核对历史不重发实际清理",async()=>{
  invoke.mockImplementation(async command=>{if(command==="personal_namespace_storage_execute")throw Error("private token");return command==="personal_namespace_storage_list"?[{...operation(),state:"applying"}]:operation();});
  render(<PersonalNamespaceCleanup profileId={profileId} inventory={inventory}/>);await preview();
  fireEvent.click(screen.getByRole("button",{name:"确认清理"}));await screen.findByText("结果未确认，未自动重做。请核对历史或中断状态。");
  expect(screen.getByRole("button",{name:"确认清理"}).hasAttribute("disabled")).toBe(true);
  fireEvent.click(screen.getByRole("button",{name:"核对清理历史"}));await screen.findByText("处理中");
  expect(invoke.mock.calls.filter(([command])=>command==="personal_namespace_storage_execute")).toHaveLength(1);
  expect(screen.queryByText(/private token/)).toBeNull();
});
it("真实零缓存与实际磁盘释放未提供分别显示",async()=>{
  const done={...operation(),state:"completed",actions:operation().actions.map(item=>({...item,state:"acknowledged",outcome:"succeeded",afterCacheBytes:0}))};
  invoke.mockImplementation(async command=>command==="personal_namespace_storage_list"?[]:command==="personal_namespace_storage_execute"?{operation:done,maintenanceError:null}:operation());
  render(<PersonalNamespaceCleanup profileId={profileId} inventory={inventory}/>);await preview();
  fireEvent.click(screen.getByRole("button",{name:"确认清理"}));await screen.findByText("已完成");
  expect(screen.getByText("浏览器 HTTP 缓存：已确认清理 · 当前缓存 0 字节")).toBeTruthy();
  expect(screen.getByText("实际磁盘空间释放：未提供。")).toBeTruthy();
});
