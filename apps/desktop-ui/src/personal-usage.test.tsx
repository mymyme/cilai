import {cleanup,fireEvent,render,screen,waitFor,within} from "@testing-library/react";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {PersonalUsageSettings} from "./personal-usage";
const invoke=vi.hoisted(()=>vi.fn());
vi.mock("./host",()=>({invoke}));
afterEach(cleanup);
beforeEach(()=>invoke.mockReset());
const row=(overrides:Record<string,unknown>={})=>({runId:"run-one",purpose:"foreground",phase:"reply",connectionId:"connection-a",modelId:"shared-name",status:"completed",startedAt:1,completedAt:2,tokens:null,cost:null,...overrides});
const view=(rows:unknown[]=[])=>({stateRevision:1,rows,coverage:[{purpose:"foreground",source:"persisted_runtime_records"},
  {purpose:"review",source:"not_provided"},{purpose:"memory",source:"not_provided"},{purpose:"page_ai",source:"current_runtime_records_usage_not_provided"}],costSource:null,priceCheckedAt:null});
const tokens=(input:number,output:number)=>({inputTokens:input,cachedInputTokens:0,outputTokens:output,source:"official_runtime_usage"});

it("missing and actual zero remain distinct while partial coverage is explicit",async()=>{
  invoke.mockResolvedValue(view([row(),row({runId:"run-two",tokens:tokens(0,0)})]));
  render(<PersonalUsageSettings/>);
  const table=await screen.findByRole("table");
  await waitFor(()=>expect(within(table).getByText("1 / 2")).toBeTruthy());
  expect(within(table).getAllByText("0")).toHaveLength(3);
  expect(screen.getByText(/缺失值不计为零/)).toBeTruthy();
  expect(screen.getAllByText("独立用量未提供")).toHaveLength(2);
  expect(invoke).toHaveBeenCalledWith("ai_usage_view");
});
it("same model across connections and automation remain separate",async()=>{
  invoke.mockResolvedValue(view([row({tokens:tokens(12,3)}),row({connectionId:"connection-b",tokens:tokens(7,2)}),
    row({purpose:"automation",tokens:tokens(9,4)})]));
  render(<PersonalUsageSettings/>);
  await screen.findByText("connection-b");
  expect(screen.getAllByRole("row")).toHaveLength(4);
  fireEvent.click(screen.getByRole("combobox",{name:"用途"}));fireEvent.click(screen.getByRole("option",{name:"自动化"}));
  expect(screen.getAllByRole("row")).toHaveLength(2);
  expect(screen.getByText("9")).toBeTruthy();
  expect(screen.queryByText("connection-b")).toBeNull();
});
it("unrecorded historical connection is not filled from another record and classification is separate from review",async()=>{
  invoke.mockResolvedValue(view([row({purpose:"unknown",connectionId:null,modelId:null}),
    row({purpose:"classification",phase:"classification",connectionId:null,modelId:null,tokens:tokens(4,1)})]));
  render(<PersonalUsageSettings/>);
  await screen.findByText("用途未提供");
  expect(screen.getAllByText("模型未提供")).toHaveLength(2);
  expect(within(screen.getByRole("table")).getByText("会话整理")).toBeTruthy();
  fireEvent.click(screen.getByRole("combobox",{name:"用途"}));fireEvent.click(screen.getByRole("option",{name:"自动审核"}));
  expect(screen.getByText("此范围没有已记录的运行。")).toBeTruthy();
  expect(screen.getByText("独立用量未提供")).toBeTruthy();
});
it("failed refresh removes stale results and does not call models or alter preferences",async()=>{
  invoke.mockResolvedValueOnce(view([row({tokens:tokens(123,11)})])).mockRejectedValueOnce(new Error("读取用量失败"));
  render(<PersonalUsageSettings/>);
  await screen.findByText("123");
  fireEvent.click(screen.getByRole("button",{name:"刷新"}));
  expect((await screen.findByRole("alert")).textContent).toContain("读取用量失败");
  expect(screen.queryByRole("table")).toBeNull();
  expect(invoke.mock.calls).toEqual([["ai_usage_view"],["ai_usage_view"]]);
});
it("invalid counts cannot be displayed as precise statistics",async()=>{
  invoke.mockResolvedValue(view([row({tokens:tokens(-1,1)})]));
  render(<PersonalUsageSettings/>);
  expect((await screen.findByRole("alert")).textContent).toContain("用量记录无效");
  expect(screen.queryByRole("table")).toBeNull();
});
it("sum outside safe integer range is marked unrepresentable instead of rounded",async()=>{
  invoke.mockResolvedValue(view([row({tokens:tokens(Number.MAX_SAFE_INTEGER,1)}),row({tokens:tokens(1,2)})]));
  render(<PersonalUsageSettings/>);
  await screen.findAllByText("超出精确统计范围");
  expect(screen.getAllByText("超出精确统计范围")).toHaveLength(1);
  expect(screen.getByRole("cell",{name:"输出"}).textContent).toContain("3");
});

const request=(overrides:Record<string,unknown>={})=>row({runId:"request-one",purpose:"review",phase:"request",recordSource:"persisted_request_metadata",
  sourceProfileId:"original-profile",configRevision:2,authGeneration:1,readOnlyAfterRestore:false,...overrides});
const responseTokens=(overrides:Record<string,unknown>={})=>({inputTokens:null,cachedInputTokens:null,cacheWriteInputTokens:null,
  outputTokens:null,reasoningOutputTokens:null,totalTokens:null,source:"upstream_responses_usage",...overrides});
const cost=(amount:string,currency="USD")=>({amount,currency,source:"upstream.responses.usage.cost"});

it("request counters show independent coverage without filling omitted fields from total or cache",async()=>{
  invoke.mockResolvedValue(view([request({tokens:responseTokens({inputTokens:8,outputTokens:0,totalTokens:8})}),
    request({runId:"request-two",tokens:responseTokens({cachedInputTokens:0,reasoningOutputTokens:3})})]));
  render(<PersonalUsageSettings/>);
  await screen.findByRole("table");
  for(const label of ["输入","缓存输入","输出","推理输出","总计"]){
    const cell=screen.getByRole("cell",{name:label});
    expect(within(cell).getByText("1 / 2 条提供")).toBeTruthy();
  }
  const cache=screen.getByRole("cell",{name:"缓存写入"});
  expect(within(cache).getByText("未提供")).toBeTruthy();
  expect(within(cache).getByText("0 / 2 条提供")).toBeTruthy();
  expect(within(screen.getByRole("cell",{name:"输出"})).getByText("0")).toBeTruthy();
});
it("upstream expenses are summed exactly by explicit currency including tiny values and actual zero",async()=>{
  invoke.mockResolvedValue(view([
    request({cost:cost("9007199254740993.000000000000000001")}),
    request({runId:"request-two",cost:cost("0.000000000000000009")}),
    request({runId:"request-three",cost:cost("0","CNY")}),request({runId:"request-four"}),
  ]));
  render(<PersonalUsageSettings/>);
  const expense=await screen.findByRole("cell",{name:"费用"});
  expect(within(expense).getByText("USD 9007199254740993.00000000000000001")).toBeTruthy();
  expect(within(expense).getByText("CNY 0")).toBeTruthy();
  expect(within(expense).getByText("2 / 4 条提供")).toBeTruthy();
  expect(within(expense).getByText("1 / 4 条提供")).toBeTruthy();
  expect(within(expense).getByText("1 条未提供")).toBeTruthy();
  expect(invoke.mock.calls).toEqual([["ai_usage_view"]]);
});
it("restored and unconfirmed request records remain visible without claiming zero expense or retrying",async()=>{
  const result=view([request({readOnlyAfterRestore:true,status:"unconfirmed",completedAt:null})]);
  result.coverage=[{purpose:"review",source:"persisted_request_receipts"}];
  invoke.mockResolvedValue(result);
  render(<PersonalUsageSettings/>);
  expect(await screen.findByText("恢复历史：1 条，只读")).toBeTruthy();
  expect(screen.getByText("结果未确认：1 条")).toBeTruthy();
  expect(screen.getByText("按已保存的独立请求回执统计")).toBeTruthy();
  expect(screen.queryByText("USD 0")).toBeNull();
  expect(invoke.mock.calls).toEqual([["ai_usage_view"]]);
});
it("request receipts and run aggregates are separate and changing model choices cannot rewrite history",async()=>{
  invoke.mockResolvedValue(view([row({purpose:"review",tokens:tokens(10,3)}),request({tokens:responseTokens({inputTokens:10})}),
    request({runId:"restored-request",sourceProfileId:"another-profile",readOnlyAfterRestore:true,tokens:responseTokens({inputTokens:10})})]));
  render(<PersonalUsageSettings/>);
  await screen.findByRole("table");
  expect(screen.getAllByRole("row")).toHaveLength(4);
  expect(screen.getAllByRole("cell",{name:"输入"})).toHaveLength(3);
  expect(screen.queryByText("30")).toBeNull();
});
it.each([
  {amount:"0.1",currency:"",source:"upstream.responses.usage.cost"},
  cost("1e-8"),cost("+1"),cost("01"),cost("-1"),cost("1."),cost("1".repeat(81)),
  {amount:"1",currency:"USD",source:"PRIVATE_SECRET"},
])("malformed expense metadata is rejected without displaying its contents: %#",async invalid=>{
  invoke.mockResolvedValue(view([request({cost:invalid})]));
  render(<PersonalUsageSettings/>);
  expect((await screen.findByRole("alert")).textContent).toContain("用量记录无效");
  expect(screen.queryByRole("table")).toBeNull();
  expect(document.body.textContent).not.toContain("PRIVATE_SECRET");
});
it("a claimed upstream counter without its independent request identity is rejected",async()=>{
  invoke.mockResolvedValue(view([row({tokens:responseTokens({inputTokens:1})})]));
  render(<PersonalUsageSettings/>);
  expect((await screen.findByRole("alert")).textContent).toContain("用量记录无效");
});

it("confirmed duplicate receipts remain visible and are excluded once from counter and currency totals",async()=>{
  invoke.mockResolvedValue(view([request({tokens:responseTokens({inputTokens:7,totalTokens:9}),cost:cost("0.000001")}),
    request({runId:"request-two",tokens:responseTokens({inputTokens:7,totalTokens:9}),cost:cost("0.000001"),contributesToTotals:false,duplicateOfRequestId:"request-one"})]));
  render(<PersonalUsageSettings/>);
  expect(await screen.findByText("重复回执：1 条，未重复计数")).toBeTruthy();
  expect(screen.getByText("计入 1 / 2 条")).toBeTruthy();
  const input=screen.getByRole("cell",{name:"输入"});
  expect(within(input).getByText("7")).toBeTruthy();
  expect(within(input).getByText("1 / 1 条提供")).toBeTruthy();
  expect(within(screen.getByRole("cell",{name:"费用"})).getByText("USD 0.000001")).toBeTruthy();
  expect(screen.queryByText("USD 0.000002")).toBeNull();
});
it("an unbound duplicate marker cannot silently exclude a record from totals",async()=>{
  invoke.mockResolvedValue(view([request({contributesToTotals:false,tokens:responseTokens({inputTokens:7})})]));
  render(<PersonalUsageSettings/>);
  expect((await screen.findByRole("alert")).textContent).toContain("用量记录无效");
});

it.each([
  [request({runId:"request-two",duplicateOfRequestId:"missing-request",contributesToTotals:false})],
  [request({tokens:responseTokens({inputTokens:7})}),request({runId:"request-two",tokens:responseTokens({inputTokens:8}),duplicateOfRequestId:"request-one",contributesToTotals:false})],
  [request(),request({runId:"request-two",configRevision:3,duplicateOfRequestId:"request-one",contributesToTotals:false})],
])("duplicate receipt attribution must match an included request and its exact counters: %#",async(...rows)=>{
  invoke.mockResolvedValue(view(rows));
  render(<PersonalUsageSettings/>);
  expect((await screen.findByRole("alert")).textContent).toContain("用量记录无效");
});
