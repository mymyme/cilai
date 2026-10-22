import {act,cleanup,fireEvent,render,screen,waitFor} from "@testing-library/react";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {PersonalNotificationSettingsPanel} from "./personal-notification-settings";
import type {PersonalNotificationSettings} from "./personal-notifications";
const invoke=vi.hoisted(()=>vi.fn());
vi.mock("./host",()=>({invoke}));
afterEach(cleanup);
const state=():PersonalNotificationSettings=>({profileId:"a".repeat(32),preferences:{enabled:false,completed:true,failed:true,actionRequired:true,sound:false,revision:0},system:{supported:true,active:false,reasonCode:null,lastOutcome:null,profileId:"a".repeat(32)}});
let current:PersonalNotificationSettings;
beforeEach(()=>{current=state();invoke.mockReset();invoke.mockImplementation(async(command,args)=>{
  if(command==="personal_notification_preferences_get")return structuredClone(current);
  if(command==="personal_notification_preferences_save"){
    const {profileId,expectedRevision,...preferences}=args.request;
    expect(profileId).toBe(current.profileId);expect(expectedRevision).toBe(current.preferences.revision);
    current.preferences={...preferences,revision:expectedRevision+1};return structuredClone(current);
  }
  throw new Error("unexpected");
});});
async function ready(){await waitFor(()=>expect((screen.getByRole("checkbox",{name:"允许通知"}) as HTMLInputElement).disabled).toBe(false));}
it("默认关闭，只读进入不会授权系统通知",async()=>{
  render(<PersonalNotificationSettingsPanel/>);await ready();
  expect((screen.getByRole("checkbox",{name:"允许通知"}) as HTMLInputElement).checked).toBe(false);
  expect((screen.getByRole("checkbox",{name:"完成"}) as HTMLInputElement).disabled).toBe(true);
  expect(invoke).toHaveBeenCalledTimes(1);expect(invoke).toHaveBeenCalledWith("personal_notification_preferences_get",{});
});
it("明确保存当前资料及修订，普通界面没有claim或open命令",async()=>{
  render(<PersonalNotificationSettingsPanel/>);await ready();fireEvent.click(screen.getByRole("checkbox",{name:"允许通知"}));fireEvent.click(screen.getByRole("checkbox",{name:"声音"}));
  fireEvent.click(screen.getByRole("button",{name:"保存"}));await screen.findByText("已保存");
  expect(invoke).toHaveBeenLastCalledWith("personal_notification_preferences_save",{request:{profileId:"a".repeat(32),expectedRevision:0,enabled:true,completed:true,failed:true,actionRequired:true,sound:true}});
  expect(invoke.mock.calls.every(([command])=>!String(command).endsWith("_internal"))).toBe(true);
});
it("保存未知保持输入并锁住再次写入，刷新只读取原结果",async()=>{
  const original=invoke.getMockImplementation()!;invoke.mockImplementation((command,args)=>command==="personal_notification_preferences_save"?Promise.reject(new Error("private error")):original(command,args));
  render(<PersonalNotificationSettingsPanel/>);await ready();fireEvent.click(screen.getByRole("checkbox",{name:"允许通知"}));fireEvent.click(screen.getByRole("button",{name:"保存"}));
  await screen.findByText("结果尚未确认，请刷新核对。");
  expect((screen.getByRole("checkbox",{name:"允许通知"}) as HTMLInputElement).checked).toBe(true);
  expect((screen.getByRole("button",{name:"保存"}) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.queryByText("private error")).toBeNull();fireEvent.click(screen.getByRole("button",{name:"刷新"}));await ready();
  expect(invoke.mock.calls.filter(([command])=>command==="personal_notification_preferences_save")).toHaveLength(1);
  expect((screen.getByRole("checkbox",{name:"允许通知"}) as HTMLInputElement).checked).toBe(false);
});
it("不支持系统通知明确呈现，不以保存成功冒充投递成功",async()=>{
  current.system.supported=false;render(<PersonalNotificationSettingsPanel/>);await screen.findByText("当前环境不支持系统通知。");
  expect(screen.queryByText("已保存")).toBeNull();expect(invoke).toHaveBeenCalledTimes(1);
});
it("资料切换后不接受旧资料的迟到回执",async()=>{
  let finish!:(value:PersonalNotificationSettings)=>void;
  invoke.mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}));
  const view=render(<PersonalNotificationSettingsPanel profileId={"a".repeat(32)}/>);
  current={...state(),profileId:"b".repeat(32),system:{...state().system,profileId:"b".repeat(32)}};
  view.rerender(<PersonalNotificationSettingsPanel profileId={"b".repeat(32)}/>);await ready();
  await act(async()=>finish({...state(),preferences:{...state().preferences,enabled:true}}));
  expect((screen.getByRole("checkbox",{name:"允许通知"}) as HTMLInputElement).checked).toBe(false);
});
it("维护暂停和修订冲突只给核对提示，不自动保存",async()=>{
  current.system.reasonCode="personal_notification_paused";render(<PersonalNotificationSettingsPanel/>);await screen.findByText("资料维护中，通知已暂停。");
  invoke.mockRejectedValueOnce(new Error("personal_notification_preferences_changed"));
  fireEvent.click(screen.getByRole("checkbox",{name:"允许通知"}));fireEvent.click(screen.getByRole("button",{name:"保存"}));
  await screen.findByText("设置已变化，请刷新核对。");
  expect(invoke.mock.calls.filter(([command])=>command==="personal_notification_preferences_save")).toHaveLength(1);
});
it("投递记录容量受限时明确提示停止新通知",async()=>{
  current.system.reasonCode="personal_notification_delivery_limit";render(<PersonalNotificationSettingsPanel/>);
  await screen.findByText("投递记录已达上限，已停止新通知。");
  expect(invoke.mock.calls.every(([command])=>command==="personal_notification_preferences_get")).toBe(true);
});
