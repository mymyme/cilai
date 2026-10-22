import {cleanup, fireEvent, render, screen, waitFor, within} from "@testing-library/react";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import type {PersonalDataOperation, PersonalMaintenanceFence} from "./generated/protocol";
import {PersonalDataSettings, PersonalMaintenanceRecovery, PersonalRestoreStatus, personalDataError} from "./personal-data";
const invoke=vi.hoisted(()=>vi.fn());
vi.mock("./host",()=>({invoke}));
afterEach(cleanup);
const profileId="1".repeat(32);
const operation=():PersonalDataOperation=>({operationId:"backup-one",profileId,kind:"backup",state:"prepared",previewDigest:"sha256:one",createdAt:1,updatedAt:1,selection:{includeWorkspaceFiles:[]},backupManifest:null,restorePreview:null,blockers:[],result:null});
let records:PersonalDataOperation[],fences:PersonalMaintenanceFence[];
beforeEach(()=>{records=[];fences=[];invoke.mockReset();invoke.mockImplementation(async(command:string)=>{
  if(command==="personal_data_list")return structuredClone(records);
  if(command==="personal_maintenance_status")return structuredClone(fences);
  if(command==="personal_storage_list")return [];
  if(command==="personal_memory_reset_list"||command==="personal_application_data_list"||command==="personal_namespace_storage_list")return [];
  if(command==="personal_storage_inventory")return {profileId,profilePath:"/personal",scannedAt:1,knownLogicalBytes:0,sizeComplete:false,activeWriterCount:0,coverage:[],items:[],issues:[]};
  if(command==="app_server_request")return {workspaces:[]};
  throw new Error("unconfirmed");
});});
it("打开数据页只读取，不启动备份恢复或释放维护",async()=>{
  render(<PersonalDataSettings profileId={profileId}/>);await screen.findByText("尚无备份或恢复记录。");
  expect(invoke.mock.calls.every(call=>["personal_data_list","personal_maintenance_status","personal_storage_inventory","personal_storage_list","personal_namespace_storage_inventory","personal_memory_reset_list","personal_application_data_list","personal_namespace_storage_list","app_server_request"].includes(call[0]))).toBe(true);
});
it("备份先取得预览再确认，取消不停止任务或生成归档",async()=>{
  const original=invoke.getMockImplementation()!;
  invoke.mockImplementation(async(command,args)=>command==="personal_data_backup_prepare"?operation():original(command,args));
  render(<PersonalDataSettings profileId={profileId}/>);fireEvent.click(await screen.findByRole("button",{name:"准备备份"}));
  await screen.findByRole("dialog");fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"取消"}));
  expect(invoke.mock.calls.some(call=>call[0]==="personal_data_backup_capture")).toBe(false);
});
it("静止失败显示未完成，不能导出，不自动重试",async()=>{
  records=[operation()];const original=invoke.getMockImplementation()!;
  invoke.mockImplementation(async(command,args)=>command==="personal_data_backup_capture"?{
    operation:{...operation(),state:"incomplete",blockers:[{code:"personal_data_writer_active",category:null,objectId:null,relativePath:null}]},maintenanceError:"personal_maintenance_drain_unconfirmed",restartRequired:false}:original(command,args));
  render(<PersonalDataSettings profileId={profileId}/>);fireEvent.click(await screen.findByRole("button",{name:"核对并继续"}));
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"备份"}));
  await screen.findByText("未完成");expect(screen.queryByRole("button",{name:"导出"})).toBeNull();
  expect(invoke.mock.calls.filter(call=>call[0]==="personal_data_backup_capture")).toHaveLength(1);
  const request=invoke.mock.calls.find(call=>call[0]==="personal_data_backup_capture")![1].request;
  expect(request).not.toHaveProperty("quiescence");
  expect(request.previewDigest).toBe("sha256:one");
});
it("未知回执锁定新操作，只读刷新后核对已有记录",async()=>{
  records=[operation()];render(<PersonalDataSettings profileId={profileId}/>);
  fireEvent.click(await screen.findByRole("button",{name:"核对并继续"}));fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"备份"}));
  await screen.findByRole("alert");expect((screen.getByRole("button",{name:"准备备份"}) as HTMLButtonElement).disabled).toBe(true);
  expect(invoke.mock.calls.filter(call=>call[0]==="personal_data_backup_capture")).toHaveLength(1);
  fireEvent.click(screen.getByRole("button",{name:"刷新"}));await waitFor(()=>expect((screen.getByRole("button",{name:"准备备份"}) as HTMLButtonElement).disabled).toBe(false));
});
it("恢复预览列出新旧资料和保留位置，失败不会显示恢复成功",async()=>{
  const restore:PersonalDataOperation={...operation(),kind:"restore",restorePreview:{sourceProfile:{profileId:"2".repeat(32),displayName:"来源",schemaVersion:2},currentProfileId:profileId,targetProfileId:"3".repeat(32),archiveDigest:"sha256:archive",manifestDigest:"sha256:manifest",replacesCurrentProfileDirectory:false,preservesCurrentCopy:true,fileCount:12,totalBytes:200,workspaces:[],warnings:[]}};
  records=[restore];render(<PersonalDataSettings profileId={profileId}/>);
  fireEvent.click(await screen.findByRole("button",{name:"核对并继续"}));
  const dialog=screen.getByRole("dialog");expect(dialog.textContent).toContain("3".repeat(32));expect(dialog.textContent).toContain(`profiles/${profileId}`);
  fireEvent.click(within(dialog).getByRole("button",{name:"恢复"}));await screen.findByRole("alert");
  expect(invoke.mock.calls.find(call=>call[0]==="personal_data_restore_apply")![1].request.expectedCurrentProfileId).toBe(profileId);
  expect(screen.queryByText("恢复已处理，请重启后核对。")).toBeNull();
});
it("初次读取失败不显示空记录成功，阻止准备操作",async()=>{
  invoke.mockRejectedValue(new Error("storage_unknown"));render(<PersonalDataSettings profileId={profileId}/>);
  await waitFor(()=>expect(screen.getAllByRole("alert").length).toBeGreaterThan(0));
  expect(screen.queryByText("尚无备份或恢复记录。")).toBeNull();
  expect((screen.getByRole("button",{name:"准备备份"}) as HTMLButtonElement).disabled).toBe(true);
});
it("解除维护需要显式确认，取消不会解除旧进程占用",async()=>{
  fences=[{fenceId:"maintenance_"+"a".repeat(32),operationId:"backup-one",profileId,instanceFence:"instance",owner:{hostSessionId:"session",pid:1,processStart:"start"} as unknown as PersonalMaintenanceFence["owner"],state:"draining",createdAt:1,updatedAt:1}];
  render(<PersonalMaintenanceRecovery/>);fireEvent.click(await screen.findByRole("button",{name:"解除已退出进程的占用"}));
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"取消"}));
  expect(invoke.mock.calls.some(call=>call[0]==="personal_maintenance_recover")).toBe(false);
});
it("恢复应用中不给重启按钮，结束后重启失败不假装退出",async()=>{
  const view=render(<PersonalRestoreStatus applying/>);expect(screen.queryByRole("button",{name:"重启"})).toBeNull();
  view.rerender(<PersonalRestoreStatus applying={false}/>);fireEvent.click(screen.getByRole("button",{name:"重启"}));await screen.findByRole("alert");
  expect(invoke.mock.calls.filter(call=>call[0]==="personal_data_restart")).toHaveLength(1);
});
it("未知错误不将任意凭据或正文回显到界面",()=>{
  expect(personalDataError("private-secret body sentence")).not.toContain("private-secret");
  expect(personalDataError("error personal_data_writer_active")).toBe("仍有资料写入，维护未完成。");
});

function workspaceRestore():PersonalDataOperation {
  return {...operation(),kind:"restore",restorePreview:{
    sourceProfile:{profileId:"2".repeat(32),displayName:"来源",schemaVersion:2},currentProfileId:profileId,targetProfileId:"3".repeat(32),
    archiveDigest:"sha256:archive",manifestDigest:"sha256:manifest",replacesCurrentProfileDirectory:false,preservesCurrentCopy:true,
    fileCount:12,totalBytes:200,warnings:[],workspaces:[{workspaceId:"workspace-one",displayName:"项目",originalPath:"/old-project",
      originalFilesIncluded:true,archivePrefix:"workspaces/workspace-one"}],
  }};
}

it("包含原文的恢复先选新位置，只用更新后的预览确认且不向宿主提交路径",async()=>{
  const restore=workspaceRestore();records=[restore];const original=invoke.getMockImplementation()!;
  invoke.mockImplementation(async(command,args)=>{
    if(command==="personal_data_restore_workspace_plan")return {...restore,previewDigest:"sha256:chosen",restorePreview:{
      ...restore.restorePreview!,workspaceDestination:{parentPath:"/chosen",parentIdentity:"identity",targetPath:"/chosen/Cilai-Restore-new"},
    }};
    return original(command,args);
  });
  render(<PersonalDataSettings profileId={profileId}/>);fireEvent.click(await screen.findByRole("button",{name:"核对并继续"}));
  let dialog=screen.getByRole("dialog");
  expect((within(dialog).getByRole("button",{name:"恢复"}) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(within(dialog).getByRole("button",{name:"选择位置"}));
  await screen.findByText("/chosen/Cilai-Restore-new");
  dialog=screen.getByRole("dialog");
  expect((within(dialog).getByRole("button",{name:"恢复"}) as HTMLButtonElement).disabled).toBe(false);
  const plan=invoke.mock.calls.find(call=>call[0]==="personal_data_restore_workspace_plan")![1].request;
  expect(Object.keys(plan).sort()).toEqual(["clientOperationId","operationId","previewDigest"]);
  expect(plan.previewDigest).toBe("sha256:one");
  fireEvent.click(within(dialog).getByRole("button",{name:"恢复"}));await screen.findByRole("alert");
  expect(invoke.mock.calls.find(call=>call[0]==="personal_data_restore_apply")![1].request.previewDigest).toBe("sha256:chosen");
});

it("取消原文位置选择保留预览，未知回执锁定确认并且不重试",async()=>{
  records=[workspaceRestore()];const original=invoke.getMockImplementation()!;
  let result:"cancel"|"unknown"="cancel";
  invoke.mockImplementation(async(command,args)=>command==="personal_data_restore_workspace_plan"
    ?result==="cancel"?null:Promise.reject(Error("unknown")):original(command,args));
  render(<PersonalDataSettings profileId={profileId}/>);fireEvent.click(await screen.findByRole("button",{name:"核对并继续"}));
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"选择位置"}));
  await waitFor(()=>expect((within(screen.getByRole("dialog")).getByRole("button",{name:"选择位置"}) as HTMLButtonElement).disabled).toBe(false));
  expect((within(screen.getByRole("dialog")).getByRole("button",{name:"恢复"}) as HTMLButtonElement).disabled).toBe(true);
  result="unknown";
  fireEvent.click(within(screen.getByRole("dialog")).getByRole("button",{name:"选择位置"}));await screen.findByRole("alert");
  expect((within(screen.getByRole("dialog")).getByRole("button",{name:"恢复"}) as HTMLButtonElement).disabled).toBe(true);
  expect(invoke.mock.calls.filter(call=>call[0]==="personal_data_restore_workspace_plan")).toHaveLength(2);
  expect(invoke.mock.calls.some(call=>call[0]==="personal_data_restore_apply")).toBe(false);
});
