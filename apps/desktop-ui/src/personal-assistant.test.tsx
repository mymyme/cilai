import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { PersonalAssistantPage } from "./personal-assistant";
const host=vi.hoisted(()=>({invoke:vi.fn(),seed:{} as Record<string,unknown>,work:[] as unknown[]}));
vi.mock("./host",()=>({invoke:host.invoke,listen:async()=>()=>{}}));
vi.mock("./conversation-session",async()=>{
  const {useState}=await import("react");
  return {useConversationSessionId:()=>"session",useConversationSessionReadScope:()=>"session",useConversationSessionDispatchReady:()=>true,useConversationSessionFlush:()=>async()=>{},useConversationSessionState:(key:string,initial:unknown)=>useState(()=>Object.hasOwn(host.seed,key)?structuredClone(host.seed[key]):typeof initial==="function"?initial():initial)};
});
vi.mock("./use-model-catalog",()=>({ModelCatalogNotice:()=>null,useModelCatalog:()=>({loading:false,loaded:true,models:[{modelId:"model",connectionId:"connection",label:"个人模型",inputModalities:["text"],supportedReasoningEfforts:[],defaultReasoningEffort:null}],refreshOnOpen:()=>{}})}));
beforeEach(()=>{
  host.seed={personalAssistantModel:"model",personalAssistantConnection:"connection"};host.work=[];host.invoke.mockReset();
  host.invoke.mockImplementation(async(command,args)=>{
    if(command==="ai_personal_assistant_view")return {runtimeSessionId:"session",messages:[],run:null,before:null,tracking:[],work:host.work};
    if(command==="model_execution_preflight")return {snapshot:{connectionId:"connection",modelId:"model"},snapshotDigest:"digest",historyTransfer:null};
    if(command==="ai_personal_assistant_send")return {runId:"run",status:"queued"};
    if(command==="ai_personal_assistant_receipt")return {status:"preparing"};
    throw new Error(`unexpected ${command} ${JSON.stringify(args)}`);
  });
});
afterEach(cleanup);
async function write(content:string){const input=await screen.findByRole("textbox",{name:"给个人助理发消息"});fireEvent.change(input,{target:{value:content}});return input as HTMLTextAreaElement;}
test("助理按明确连接发送一次并在受理后清空草稿，不把默认档位固化",async()=>{
  render(<PersonalAssistantPage/>);const input=await write("帮我整理两个项目");fireEvent.click(screen.getByRole("button",{name:"发送"}));
  await waitFor(()=>expect(host.invoke).toHaveBeenCalledWith("ai_personal_assistant_send",expect.objectContaining({runtimeSessionId:"session",connectionId:"connection",modelId:"model",reasoningEffort:null,content:"帮我整理两个项目",historyConsent:null})));
  await waitFor(()=>expect(input.value).toBe(""));
  expect(host.invoke.mock.calls.filter(([name])=>name==="ai_personal_assistant_send")).toHaveLength(1);
});
test("受理响应丢失保留原文，只核对回执，点击核对不会重发业务",async()=>{
  const implementation=host.invoke.getMockImplementation()!;host.invoke.mockImplementation(async(command,args)=>{if(command==="ai_personal_assistant_send")throw new Error("response_lost");return implementation(command,args);});
  render(<PersonalAssistantPage/>);const input=await write("保留原交办要求");fireEvent.click(screen.getByRole("button",{name:"发送"}));
  await screen.findByText("正在准备发送。");expect(input.value).toBe("保留原交办要求");
  fireEvent.click(screen.getByRole("button",{name:"核对送达"}));await act(async()=>{});
  expect(host.invoke.mock.calls.filter(([name])=>name==="ai_personal_assistant_send")).toHaveLength(1);
  expect(host.invoke.mock.calls.filter(([name])=>name==="ai_personal_assistant_receipt").length).toBeGreaterThanOrEqual(2);
});
test("待补信息绑定准确工作运行，原目标变化后不能误发到另一项",async()=>{
  host.work=[{id:"target",runId:"work-run",title:"准备方案",projectId:null,projectName:null,taskId:null,status:"waiting_user",waiting:{question:"预算多少",impact:"需要预算"},updatedAt:1}];
  render(<PersonalAssistantPage supplement={{id:"target",runId:"work-run",title:"准备方案"}}/>);
  await waitFor(()=>expect(host.invoke).toHaveBeenCalledWith("ai_personal_assistant_view",expect.anything()));const input=await write("预算一万元");
  await waitFor(()=>expect((screen.getByRole("button",{name:"发送"}) as HTMLButtonElement).disabled).toBe(false));fireEvent.click(screen.getByRole("button",{name:"发送"}));
  await waitFor(()=>expect(host.invoke).toHaveBeenCalledWith("ai_personal_assistant_send",expect.objectContaining({workRunId:"work-run",content:"预算一万元"})));
  await waitFor(()=>expect(input.value).toBe(""));expect(host.invoke.mock.calls.some(([name])=>name==="model_execution_preflight")).toBe(false);
});
