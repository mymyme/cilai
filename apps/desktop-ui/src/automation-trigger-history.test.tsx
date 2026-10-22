import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AutomationTriggerHistory } from "./automation-trigger-history";
import type { PersonalAutomationTriggerRecord, PersonalAutomationTriggerPage } from "./generated/protocol";
const invoke=vi.hoisted(()=>vi.fn());
vi.mock("./host",()=>({invoke}));
afterEach(cleanup);beforeEach(()=>invoke.mockReset());
const record=(patch:Partial<PersonalAutomationTriggerRecord>={}):PersonalAutomationTriggerRecord=>({triggerId:"trigger-one",claimId:null,profileId:"a".repeat(32),ruleId:"rule-one",ruleVersion:2,scheduledDueAt:1,createdAt:2,finishedAt:3,runId:null,outcome:"skipped",reasonCode:"automation_previous_run_active",readOnlyAfterRestore:false,...patch});
const page=(r:PersonalAutomationTriggerRecord=record(),sequence=20,nextBeforeSequence:number|null=null,claimReleased:boolean|null=r.claimId?false:null):PersonalAutomationTriggerPage=>({items:[{sequence,record:r,claimReleased}],nextBeforeSequence});
const props=()=>({ruleId:"rule-one",refreshToken:"one",availableRunIds:[] as string[],onOpenRun:vi.fn()});

it("skipped triggers are read only metadata and do not create or rerun a task",async()=>{
  invoke.mockResolvedValue(page());render(<AutomationTriggerHistory {...props()}/>);
  expect(await screen.findByText("上次任务尚未结束，本次已跳过。")).toBeTruthy();
  expect(screen.queryByRole("button",{name:"核对结束状态"})).toBeNull();
  expect(invoke.mock.calls).toEqual([["ai_automation_trigger_history",{request:{ruleId:"rule-one",limit:20,beforeSequence:null}}]]);
});
it("restored history stays read only and only an available existing run can be opened",async()=>{
  const p=props();p.availableRunIds=["run-one"];
  invoke.mockResolvedValue(page(record({claimId:"old-claim",outcome:"outcome_unconfirmed",runId:"run-one",readOnlyAfterRestore:true})));
  render(<AutomationTriggerHistory {...p}/>);
  expect(await screen.findByText("恢复历史，只读")).toBeTruthy();
  expect(screen.queryByRole("button",{name:"核对结束状态"})).toBeNull();
  fireEvent.click(screen.getByRole("button",{name:"运行详情"}));expect(p.onOpenRun).toHaveBeenCalledWith("run-one");
  expect(invoke).toHaveBeenCalledTimes(1);
});
it("explicit reconciliation requires actual released receipt and only refreshes metadata",async()=>{
  const r=record({claimId:"claim-one",outcome:"admitted",finishedAt:null,reasonCode:null});
  const done={...r,outcome:"outcome_unconfirmed" as const,finishedAt:5,reasonCode:"automation_interrupted_outcome_unconfirmed"};
  invoke.mockResolvedValueOnce(page(r)).mockResolvedValueOnce({claim:{claimId:r.claimId,profileId:r.profileId,released:true},trigger:done,ruleRecord:null}).mockResolvedValueOnce(page(done,20,null,true));
  render(<AutomationTriggerHistory {...props()}/>);
  fireEvent.click(await screen.findByRole("button",{name:"核对结束状态"}));
  expect(await screen.findByText("已核对退出，原结果仍未确认。")).toBeTruthy();
  expect(screen.getByText("占用已解除")).toBeTruthy();
  expect(screen.queryByRole("button",{name:"核对结束状态"})).toBeNull();
  expect(invoke.mock.calls).toEqual([
    ["ai_automation_trigger_history",{request:{ruleId:"rule-one",limit:20,beforeSequence:null}}],
    ["ai_automation_claim_reconcile",{request:{claimId:"claim-one"}}],
    ["ai_automation_trigger_history",{request:{ruleId:"rule-one",limit:20,beforeSequence:null}}],
  ]);
});
it("unknown reconciliation cannot display completed or automatically retry",async()=>{
  const r=record({claimId:"claim-one",outcome:"admitted",reasonCode:null});
  invoke.mockResolvedValueOnce(page(r)).mockResolvedValueOnce({claim:{claimId:r.claimId,profileId:r.profileId,released:false},trigger:r});
  render(<AutomationTriggerHistory {...props()}/>);fireEvent.click(await screen.findByRole("button",{name:"核对结束状态"}));
  expect((await screen.findByRole("alert")).textContent).toContain("结束状态未确认，占用保持");
  expect(screen.queryByText("已完成")).toBeNull();expect(invoke).toHaveBeenCalledTimes(2);
});
it("failed history read does not invent an empty history; refresh is explicit",async()=>{
  invoke.mockRejectedValueOnce(new Error("读取失败")).mockResolvedValueOnce(page());render(<AutomationTriggerHistory {...props()}/>);
  expect((await screen.findByRole("alert")).textContent).toContain("读取失败");expect(screen.queryByText("暂无触发记录。")).toBeNull();
  fireEvent.click(screen.getByRole("button",{name:"刷新触发记录"}));expect(await screen.findByText("已跳过")).toBeTruthy();expect(invoke).toHaveBeenCalledTimes(2);
});
it("a late page from a previously selected rule cannot replace the current rule",async()=>{
  let resolve!:(value:PersonalAutomationTriggerPage)=>void;
  invoke.mockImplementationOnce(()=>new Promise(r=>{resolve=r;})).mockResolvedValueOnce(page(record({ruleId:"rule-two",triggerId:"trigger-two",outcome:"completed",reasonCode:null})));
  const p=props();const rendered=render(<AutomationTriggerHistory {...p}/>);rendered.rerender(<AutomationTriggerHistory {...p} ruleId="rule-two"/>);
  expect(await screen.findByText("已完成")).toBeTruthy();await act(async()=>resolve(page()));
  expect(screen.queryByText("已跳过")).toBeNull();
});
it("pagination is bounded to the actual rule and does not start missing run details",async()=>{
  invoke.mockResolvedValueOnce(page(record(),20,20)).mockResolvedValueOnce(page(record({triggerId:"trigger-two",runId:"not-present",reasonCode:null,outcome:"failed"}),19));
  render(<AutomationTriggerHistory {...props()}/>);fireEvent.click(await screen.findByRole("button",{name:"更多"}));
  expect(await screen.findByText("失败")).toBeTruthy();expect(screen.queryByRole("button",{name:"运行详情"})).toBeNull();
  expect(invoke.mock.calls[1]).toEqual(["ai_automation_trigger_history",{request:{ruleId:"rule-one",limit:20,beforeSequence:20}}]);
});
it("duplicate page entries are rejected and do not silently replace earlier trigger facts",async()=>{
  invoke.mockResolvedValueOnce(page(record(),20,20)).mockResolvedValueOnce(page(record({outcome:"completed",reasonCode:null}),19));
  render(<AutomationTriggerHistory {...props()}/>);fireEvent.click(await screen.findByRole("button",{name:"更多"}));
  expect((await screen.findByRole("alert")).textContent).toContain("触发记录已变化");expect(screen.getByText("已跳过")).toBeTruthy();expect(screen.queryByText("已完成")).toBeNull();
});
it("unrecognized reason text is not echoed as a supplier error or secret",async()=>{
  invoke.mockResolvedValue(page(record({reasonCode:"PRIVATE_SECRET_RESPONSE"})));render(<AutomationTriggerHistory {...props()}/>);
  expect(await screen.findByText("原因无法识别，请查看诊断。")).toBeTruthy();expect(document.body.textContent).not.toContain("PRIVATE_SECRET_RESPONSE");
});
it("a metadata refresh during reconciliation clears busy state when the original call settles",async()=>{
  const r=record({claimId:"claim-one",outcome:"admitted",reasonCode:null});let resolve!:(value:unknown)=>void;
  invoke.mockResolvedValueOnce(page(r)).mockImplementationOnce(()=>new Promise(done=>{resolve=done;})).mockResolvedValueOnce(page(r));
  const p=props();const rendered=render(<AutomationTriggerHistory {...p}/>);fireEvent.click(await screen.findByRole("button",{name:"核对结束状态"}));
  rendered.rerender(<AutomationTriggerHistory {...p} refreshToken="two"/>);
  await waitFor(()=>expect(invoke).toHaveBeenCalledTimes(3));await act(async()=>resolve({claim:{released:false}}));
  await waitFor(()=>expect(screen.getByRole("button",{name:"核对结束状态"})).toHaveProperty("disabled",false));
});
it.each([page(record({ruleId:"wrong-rule"})),page(record(),Number.MAX_SAFE_INTEGER+1),{items:[],nextBeforeSequence:10}])("invalid page cannot be presented as verified history: %#",async invalid=>{
  invoke.mockResolvedValue(invalid);render(<AutomationTriggerHistory {...props()}/>);
  expect((await screen.findByRole("alert")).textContent).toContain("触发记录无法核对");expect(screen.queryByText("暂无触发记录。")).toBeNull();
});

it("unknown result with an already released claim is not offered as an active task to reconcile",async()=>{
  invoke.mockResolvedValue(page(record({claimId:"claim-one",outcome:"outcome_unconfirmed",reasonCode:null}),20,null,true));
  render(<AutomationTriggerHistory {...props()}/>);
  expect(await screen.findByText("结果未确认")).toBeTruthy();expect(screen.getByText("占用已解除")).toBeTruthy();
  expect(screen.queryByRole("button",{name:"核对结束状态"})).toBeNull();expect(invoke).toHaveBeenCalledTimes(1);
});
it("a claim without a verifiable release flag cannot be displayed as a free or active resource",async()=>{
  invoke.mockResolvedValue(page(record({claimId:"claim-one"}),20,null,null));render(<AutomationTriggerHistory {...props()}/>);
  expect((await screen.findByRole("alert")).textContent).toContain("触发记录无法核对");
});

it("reconciled interruption refreshes the actual paused rule without resubmitting it",async()=>{
  const r=record({claimId:"claim-one",outcome:"admitted",reasonCode:null});
  const done={...r,outcome:"outcome_unconfirmed" as const,reasonCode:null};const onReconciled=vi.fn();
  invoke.mockResolvedValueOnce(page(r)).mockResolvedValueOnce({claim:{claimId:r.claimId,profileId:r.profileId,released:true},trigger:done,ruleRecord:{id:"automationRules:rule-one",value:{value:{automationRuleId:"rule-one",enabled:false,nextRunAt:null}}}}).mockResolvedValueOnce(page(done,20,null,true));
  render(<AutomationTriggerHistory {...props()} onReconciled={onReconciled}/>);fireEvent.click(await screen.findByRole("button",{name:"核对结束状态"}));
  expect(await screen.findByText("已核对退出，规则已暂停。原结果仍未确认。")).toBeTruthy();
  await waitFor(()=>expect(onReconciled).toHaveBeenCalledTimes(1));expect(invoke.mock.calls.map(call=>call[0])).toEqual(["ai_automation_trigger_history","ai_automation_claim_reconcile","ai_automation_trigger_history"]);
});

it("a released receipt replay never claims that a subsequently enabled rule is still paused",async()=>{
  const r=record({claimId:"claim-one",outcome:"outcome_unconfirmed",reasonCode:null});
  invoke.mockResolvedValueOnce(page(r)).mockResolvedValueOnce({claim:{claimId:r.claimId,profileId:r.profileId,released:true},trigger:r,ruleRecord:{id:"automationRules:rule-one",value:{value:{automationRuleId:"rule-one",enabled:true,nextRunAt:500}}}}).mockResolvedValueOnce(page(r,20,null,true));
  render(<AutomationTriggerHistory {...props()}/>);fireEvent.click(await screen.findByRole("button",{name:"核对结束状态"}));
  expect(await screen.findByText("占用已解除，原结果仍未确认。")).toBeTruthy();expect(screen.queryByText("已核对退出，规则已暂停。原结果仍未确认。")).toBeNull();expect(invoke).toHaveBeenCalledTimes(3);
});
