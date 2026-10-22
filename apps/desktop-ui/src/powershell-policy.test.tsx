import {cleanup,fireEvent,render,screen,waitFor} from "@testing-library/react";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {PowerShellPolicySettings} from "./powershell-policy";
const invoke=vi.hoisted(()=>vi.fn());
vi.mock("./host",()=>({invoke}));
afterEach(cleanup);
beforeEach(()=>{invoke.mockReset();});
const restricted={status:"restricted",planId:"enable-once",restorePlanId:null,changes:[{engine:"windowsPowerShell",before:"Undefined",after:"RemoteSigned"}],restoreChanges:[],engines:[
  {engine:"windowsPowerShell",path:"C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",scopes:{machinePolicy:"Undefined",userPolicy:"Undefined",currentUser:"Undefined",localMachine:"Undefined",effective:"Restricted"}}
]};
it("checking is read-only; explicit confirmation shows machine scope and exact change before UAC",async()=>{
 invoke.mockResolvedValue(restricted);
 render(<PowerShellPolicySettings/>);
 fireEvent.click(await screen.findByRole("button",{name:"允许本地脚本"}));
 const dialog=await screen.findByRole("dialog");
 expect(dialog.textContent).toContain("本机所有 Windows 用户");
 expect(dialog.textContent).toContain("Undefined → RemoteSigned");
 expect(invoke).toHaveBeenCalledTimes(2);
 fireEvent.click(screen.getByRole("button",{name:"取消"}));
 expect(invoke).toHaveBeenCalledTimes(2);
 fireEvent.click(screen.getByRole("button",{name:"允许本地脚本"}));
 await screen.findByRole("dialog");
 invoke.mockResolvedValueOnce({...restricted,status:"configured",planId:null,result:"updated",changes:[]});
 fireEvent.click(screen.getByRole("button",{name:"请求管理员授权"}));
 await screen.findByText(/系统默认策略已更新/);
 expect(invoke).toHaveBeenLastCalledWith("powershell_policy_apply",{planId:"enable-once"});
});
it("already elevated host still requires explicit confirmation without promising another UAC prompt",async()=>{
 invoke.mockResolvedValue({...restricted,processElevated:true});
 render(<PowerShellPolicySettings/>);
 fireEvent.click(await screen.findByRole("button",{name:"允许本地脚本"}));
 await screen.findByRole("dialog");
 expect(invoke).toHaveBeenCalledTimes(2);
 expect(screen.queryByRole("button",{name:"请求管理员授权"})).toBeNull();
 fireEvent.click(screen.getByRole("button",{name:"确认修改"}));
 await waitFor(()=>expect(invoke).toHaveBeenLastCalledWith("powershell_policy_apply",{planId:"enable-once"}));
});
it("managed policy has no change action",async()=>{
 invoke.mockResolvedValue({...restricted,status:"managed",planId:null,changes:[]});
 render(<PowerShellPolicySettings/>);
 await screen.findByText("企业管理");
 expect(screen.queryByRole("button",{name:"允许本地脚本"})).toBeNull();
 expect(invoke).toHaveBeenCalledTimes(1);
});
it("cancellation invalidates the confirmation and permits a fresh check",async()=>{
 invoke.mockResolvedValueOnce(restricted).mockResolvedValueOnce(restricted).mockRejectedValueOnce(new Error("powershell_policy_failed: 已取消管理员授权。"));
 render(<PowerShellPolicySettings/>);
 fireEvent.click(await screen.findByRole("button",{name:"允许本地脚本"}));
 fireEvent.click(await screen.findByRole("button",{name:"请求管理员授权"}));
 const alert = await screen.findByRole("alert");
 expect(alert.textContent).toContain("取消");
 expect(alert.textContent).not.toContain("powershell_policy_failed");
 expect(screen.queryByRole("button",{name:"允许本地脚本"})).toBeNull();
 invoke.mockResolvedValueOnce(restricted);
 fireEvent.click(screen.getByRole("button",{name:"重新检查策略"}));
 await screen.findByRole("button",{name:"允许本地脚本"});
});
it("restoration displays the backed-up target and uses only its own confirmation",async()=>{
 invoke.mockResolvedValue({...restricted,status:"configured",planId:null,restorePlanId:"restore-once",changes:[],restoreChanges:[{engine:"windowsPowerShell",before:"RemoteSigned",after:"Undefined"}]});
 render(<PowerShellPolicySettings/>);
 fireEvent.click(await screen.findByRole("button",{name:"恢复原策略"}));
 expect((await screen.findByRole("dialog")).textContent).toContain("RemoteSigned → Undefined");
 invoke.mockResolvedValueOnce({...restricted,result:"restored"});
 fireEvent.click(screen.getByRole("button",{name:"请求管理员授权"}));
 await screen.findByText("已恢复修改前的系统策略。");
 expect(invoke).toHaveBeenLastCalledWith("powershell_policy_apply",{planId:"restore-once"});
});
it("操作前重新检查并使用新计划，不重复使用缓存的过期确认",async()=>{
 let checks=0;
 invoke.mockImplementation(async(command)=>command==="powershell_policy_check"
   ? {...restricted,planId:++checks===1 ? "expired" : "fresh"}
   : {...restricted,status:"configured",planId:null,result:"updated",changes:[]});
 render(<PowerShellPolicySettings/>);
 fireEvent.click(await screen.findByRole("button",{name:"允许本地脚本"}));
 fireEvent.click(await screen.findByRole("button",{name:"请求管理员授权"}));
 await screen.findByText(/系统默认策略已更新/);
 expect(invoke).toHaveBeenLastCalledWith("powershell_policy_apply",{planId:"fresh"});
 expect(checks).toBe(2);
});
