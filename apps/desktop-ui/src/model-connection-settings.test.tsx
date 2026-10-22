import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ModelConnectionSettings } from "./model-connection-settings";
import type { ModelConnection, ModelDescriptor, ModelValidationRecord, ModelConnectionChange, ModelValidationPrepareParams } from "./generated/protocol";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
const connection: ModelConnection = {
  profileId: "11111111111111111111111111111111", connectionId: "22222222222222222222222222222222",
  configRevision: 1, authGeneration: 1, authScopes: [], config: { name: "本地服务", kind: "local", baseUrl: "http://127.0.0.1:11434/v1",
    localRuntime: "ollama", authMode: "none", enabled: true, network: { mode: "direct", proxyUrl: null, proxySecretRef: null, proxyAuth: "none", serviceAuth: "none", headerNames: [] } },
  secretRef: null, deleted: false, createdAt: 1, updatedAt: 1,
};
const model: ModelDescriptor = { descriptorId: "model-one", connectionId: connection.connectionId, modelId: "local-model", displayName: "本地模型", revision: 1,
  configRevision: 1, authGeneration: 1, source: "user_declared", capabilities: {}, contextWindowTokens: null, inputModalities: [], defaultReasoningEffort: null, supportedReasoningEfforts: [], runtimeVersion: null, serviceVersion: null, updatedAt: 1 };
let validationSequence=0;
function validationReceipt(request:ModelValidationPrepareParams):ModelValidationRecord {
  return {validationId:(++validationSequence).toString(16).padStart(32,"0"),runtimeKind:request.runtimeKind,connectionId:request.connectionId,modelId:request.modelId,layer:request.layer,configRevision:request.expectedConfigRevision,authGeneration:request.expectedAuthGeneration,outcome:"passed",completionRevision:1,reason:null,runtimeVersion:"0.163.0",serviceVersion:null,updatedAt:1};
}
let connections: ModelConnection[];
beforeEach(() => {
  connections = []; validationSequence=0;
  invoke.mockReset();
  invoke.mockImplementation(async (command: string) => {
    if (command === "model_connection_list") return structuredClone(connections);
    if (["model_connection_change_list", "model_purpose_list", "model_validation_list", "model_credential_cleanup_list"].includes(command)) return [];
    if (command === "model_descriptor_list") return [model];
    throw new Error(`unexpected command ${command}`);
  });
});
afterEach(cleanup);
async function chooseSelect(control: HTMLElement, label: string) {
  fireEvent.click(control);
  fireEvent.click(await screen.findByRole("option", { name: label }));
}

describe("个人连接设置", () => {
  it("高级证书随当前连接保存，切换网络方式不丢失且不自动发验证请求",async()=>{
    const pem="-----BEGIN CERTIFICATE-----\nPUBLICCERTIFICATETEXTONLY\n-----END CERTIFICATE-----";
    connections=[{...connection,config:{...connection.config,network:{...connection.config.network,caCertificatePem:pem}}}];
    const base=invoke.getMockImplementation()!;
    invoke.mockImplementation(async(command,args)=>{
      if(command==="model_connection_save")throw Error("model_ca_certificate_invalid");
      return base(command,args);
    });
    render(<ModelConnectionSettings/>);
    fireEvent.click(await screen.findByRole("button",{name:"本地服务 本地模型 · 启用"}));
    const form=within(screen.getByRole("form",{name:"编辑连接"}));
    fireEvent.click(form.getByRole("button",{name:"连接设置"}));
    const drawer=within(screen.getByRole("dialog",{name:"连接设置"}));
    expect(drawer.getByLabelText("CA 证书（PEM）")).toHaveProperty("value",pem);
    await chooseSelect(drawer.getByRole("combobox",{name:"网络"}),"系统代理");
    expect(drawer.getByLabelText("CA 证书（PEM）")).toHaveProperty("value",pem);
    fireEvent.click(drawer.getByRole("button",{name:"保存"}));
    await screen.findByRole("alert");
    const saves=invoke.mock.calls.filter(call=>call[0]==="model_connection_save");
    expect(saves).toHaveLength(1);
    expect(saves[0][1].request.prepare.config.network).toMatchObject({mode:"system_proxy",caCertificatePem:pem});
    expect(invoke.mock.calls.some(call=>call[0]==="model_connection_validate")).toBe(false);
    expect(screen.queryByText("已连接")).toBeNull();
  });

  it("进入页面只读取本地元数据，不自动验证或建立外部连接", async () => {
    render(<ModelConnectionSettings/>);
    await screen.findByText("尚未添加连接");
    expect(invoke.mock.calls.map(([command]) => command).sort()).toEqual(["model_connection_list", "model_connection_change_list", "model_purpose_list"].sort());
    fireEvent.click(screen.getByRole("button",{name:"凭据"}));
    await waitFor(()=>expect(invoke.mock.calls.map(([command])=>command).sort()).toEqual(["model_connection_list", "model_connection_change_list", "model_purpose_list", "model_credential_cleanup_list"].sort()));
    expect(screen.queryByText("企业模型网关")).toBeNull();
  });

  it("密钥只交给一次保存命令，失败后清空输入且不自动重做", async () => {
    const base = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, args: unknown) => {
      if (command === "model_connection_save") throw new Error("model_credential_write_failed;operationId=33333333333333333333333333333333");
      return base(command, args);
    });
    render(<ModelConnectionSettings/>);
    await screen.findByText("尚未添加连接");
    fireEvent.click(within(screen.getByRole("complementary",{name:"连接列表"})).getByRole("button",{name:"添加连接"}));
    const form = screen.getByRole("form", { name: "添加连接" });
    fireEvent.change(within(form).getByLabelText("名称"), { target: { value: "我的 API" } });
    fireEvent.change(within(form).getByLabelText("API 密钥"), { target: { value: "synthetic-ui-component-secret" } });
    fireEvent.click(within(form).getByRole("button", { name: "保存" }));
    await screen.findByRole("alert");
    expect(within(form).getByLabelText("API 密钥")).toHaveProperty("value", "");
    const saves = invoke.mock.calls.filter(([command]) => command === "model_connection_save");
    expect(saves).toHaveLength(1);
    expect(saves[0][1].request.secret).toBe("synthetic-ui-component-secret");
    expect(JSON.stringify(invoke.mock.calls.filter(([command]) => command !== "model_connection_save"))).not.toContain("synthetic-ui-component-secret");
    expect(screen.queryByText("已连接")).toBeNull();
  });

  it("在途任务阻止删除，但用户仍可取消预检弹窗", async () => {
    connections = [connection];
    const base = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, args: unknown) => command === "model_connection_preflight"
      ? { connectionId: connection.connectionId, action: "delete", configRevision: 1, impactDigest: "impact", references: [], activeLeaseIds: ["active-run"] }
      : base(command, args));
    render(<ModelConnectionSettings/>);
    fireEvent.click(await screen.findByRole("button", { name: "本地服务 本地模型 · 启用" }));
    fireEvent.click(within(screen.getByRole("form", { name: "编辑连接" })).getByRole("button", { name: "连接设置" }));
    fireEvent.click(within(screen.getByRole("dialog",{name:"连接设置"})).getByRole("button",{name:"删除连接"}));
    const dialog = await screen.findByRole("dialog", { name: "删除连接？" });
    expect(within(dialog).getByRole("button", { name: "删除" })).toHaveProperty("disabled", true);
    expect(within(dialog).getByRole("button", { name: "取消" })).toHaveProperty("disabled", false);
    fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "model_connection_delete")).toBe(false);
  });

  it("后台用途须选择连接模型并确认发送范围与费用才可保存", async () => {
    connections = [connection];
    const base = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, args: unknown) => command === "model_purpose_save" ? { result: (args as { request: { binding: unknown } }).request.binding, catalogRefreshError: null } : base(command, args));
    render(<ModelConnectionSettings/>);
    await screen.findByRole("button", { name: "本地服务 本地模型 · 启用" });
    fireEvent.click(screen.getByRole("button",{name:"用途"}));
    const form = screen.getByRole("form", { name: "后台记忆" });
    expect(within(form).getByLabelText("启用")).toHaveProperty("checked", false);
    await chooseSelect(within(form).getByRole("combobox", { name: "连接" }), "本地服务");
    await chooseSelect(within(form).getByRole("combobox", { name: "模型" }), "本地模型");
    fireEvent.click(within(form).getByLabelText("启用"));
    expect(within(form).getByRole("button", { name: "保存用途" })).toHaveProperty("disabled", true);
    fireEvent.click(within(form).getByRole("checkbox", { name: /会发送当前资料中.*用于记忆的历史对话/ }));
    fireEvent.click(within(form).getByRole("button", { name: "保存用途" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "model_purpose_save")).toBe(true));
    const saved = invoke.mock.calls.find(([command]) => command === "model_purpose_save")![1].request.binding;
    expect(saved.selection).toMatchObject({ connectionId: connection.connectionId, modelId: "local-model" });
    expect(saved.consent).toMatchObject({ serviceName: "本地服务", mayIncurCost: true });
    expect(saved.policy).toBe("explicit");
  });
});

it("完整验证按八层分别使用元数据代次和实际认证代次，回执只证明对应层",async()=>{
  connections=[connection];const base=invoke.getMockImplementation()!;
  invoke.mockImplementation(async(command,args)=>command==="model_connection_validate"?validationReceipt(args.request):base(command,args));
  render(<ModelConnectionSettings/>);
  fireEvent.click(await screen.findByRole("button",{name:"本地服务 本地模型 · 启用"}));
  fireEvent.click(await screen.findByRole("button",{name:"验证连接"}));
  const form=within(screen.getByRole("form",{name:"连接验证"}));
  expect(form.queryByRole("combobox",{name:"验证层级"})).toBeNull();
  await chooseSelect(form.getByRole("combobox",{name:"验证模型"}),"本地模型");
  fireEvent.click(form.getByRole("button",{name:"全部验证"}));
  fireEvent.click(within(await screen.findByRole("dialog",{name:"验证模型调用？"})).getByRole("button",{name:"全部验证"}));
  await waitFor(()=>expect(invoke.mock.calls.filter(([command])=>command==="model_connection_validate")).toHaveLength(8));
  await waitFor(()=>expect(form.getByRole("button",{name:"全部验证"})).toHaveProperty("disabled",false));
  const requests=invoke.mock.calls.filter(([command])=>command==="model_connection_validate").map(([,args])=>args.request);
  expect(requests.map(request=>request.layer)).toEqual(["configuration","network","authentication","protocol","catalog","inference","streaming","tools"]);
  for(const request of requests)expect(request).toEqual({runtimeKind:"workbench",connectionId:connection.connectionId,modelId:model.modelId,layer:request.layer,expectedConfigRevision:1,expectedAuthGeneration:["configuration","network","catalog"].includes(request.layer)?0:1});
  expect(document.querySelectorAll('.model-library-validation [data-outcome="passed"]')).toHaveLength(8);
});

it("ChatGPT完整验证不借另一用途或旧代次，未授权用途保留元数据检查",async()=>{
  connections=[{...connection,authGeneration:9,authScopes:[{runtimeKind:"workbench",authGeneration:3,state:"active",operationId:"prior-login"}],config:{...connection.config,kind:"chatgpt",baseUrl:null,localRuntime:null,authMode:"runtime_managed_chatgpt"}}];
  const records:ModelValidationRecord[]=[
    {validationId:"other-kind",connectionId:connection.connectionId,runtimeKind:"application",modelId:model.modelId,layer:"tools",configRevision:1,authGeneration:3,outcome:"passed",completionRevision:1,reason:null,runtimeVersion:"0.163.0",serviceVersion:null,updatedAt:1},
    {validationId:"old-scope",connectionId:connection.connectionId,runtimeKind:"workbench",modelId:model.modelId,layer:"inference",configRevision:1,authGeneration:2,outcome:"passed",completionRevision:1,reason:null,runtimeVersion:"0.163.0",serviceVersion:null,updatedAt:2},
  ];
  const base=invoke.getMockImplementation()!;
  invoke.mockImplementation(async(command,args)=>{
    if(command==="model_validation_list")return records;
    if(command==="model_connection_auth_changes")return [];
    if(command==="model_connection_auth_read")return{runtimeKind:args.request?.runtimeKind,account:null};
    if(command==="model_connection_validate")return validationReceipt(args.request);
    return base(command,args);
  });
  render(<ModelConnectionSettings/>);
  fireEvent.click(await screen.findByRole("button",{name:"本地服务 ChatGPT · 启用"}));
  fireEvent.click(await screen.findByRole("button",{name:"验证连接"}));
  const form=within(screen.getByRole("form",{name:"连接验证"}));
  await chooseSelect(form.getByRole("combobox",{name:"验证模型"}),"本地模型");
  expect(document.querySelectorAll('.model-library-validation [data-outcome="passed"]')).toHaveLength(0);
  await chooseSelect(form.getByRole("combobox",{name:"验证用途"}),"页面 AI");
  expect(form.getByText("此用途未授权，仅检查配置、网络和目录。")).toBeTruthy();
  fireEvent.click(form.getByRole("button",{name:"全部验证"}));
  fireEvent.click(within(await screen.findByRole("dialog",{name:"验证配置？"})).getByRole("button",{name:"全部验证"}));
  await waitFor(()=>expect(invoke.mock.calls.filter(([command])=>command==="model_connection_validate")).toHaveLength(3));
  await waitFor(()=>expect(form.getByRole("button",{name:"全部验证"})).toHaveProperty("disabled",false));
  const metadata=invoke.mock.calls.filter(([command])=>command==="model_connection_validate").map(([,args])=>args.request);
  expect(metadata.map(request=>request.layer)).toEqual(["configuration","network","catalog"]);
  for(const request of metadata)expect(request).toMatchObject({runtimeKind:"application",modelId:model.modelId,expectedConfigRevision:1,expectedAuthGeneration:0});
  expect(form.getAllByText("未执行")).toHaveLength(5);
  await chooseSelect(form.getByRole("combobox",{name:"验证用途"}),"主对话");
  fireEvent.click(form.getByRole("button",{name:"全部验证"}));
  fireEvent.click(within(await screen.findByRole("dialog",{name:"验证模型调用？"})).getByRole("button",{name:"全部验证"}));
  await waitFor(()=>expect(invoke.mock.calls.filter(([command])=>command==="model_connection_validate")).toHaveLength(11));
  const requests=invoke.mock.calls.filter(([command])=>command==="model_connection_validate").slice(3).map(([,args])=>args.request);
  expect(requests).toHaveLength(8);
  for(const request of requests)expect(request).toMatchObject({runtimeKind:"workbench",connectionId:connection.connectionId,modelId:model.modelId,expectedAuthGeneration:["configuration","network","catalog"].includes(request.layer)?0:3});
  expect(requests.some(request=>request.expectedAuthGeneration===9)).toBe(false);
});

it("完整验证先选择模型并一次确认发送范围与费用，取消不发送任何验证请求",async()=>{
  connections=[connection];render(<ModelConnectionSettings/>);
  fireEvent.click(await screen.findByRole("button",{name:"本地服务 本地模型 · 启用"}));
  fireEvent.click(await screen.findByRole("button",{name:"验证连接"}));
  const form=within(screen.getByRole("form",{name:"连接验证"}));
  expect(form.getByRole("button",{name:"全部验证"})).toHaveProperty("disabled",true);
  await chooseSelect(form.getByRole("combobox",{name:"验证模型"}),"本地模型");
  fireEvent.click(form.getByRole("button",{name:"全部验证"}));
  const dialog=within(await screen.findByRole("dialog",{name:"验证模型调用？"}));
  expect(dialog.getByText(/可能产生服务费用/)).toBeTruthy();
  expect(invoke.mock.calls.some(([command])=>command==="model_connection_validate")).toBe(false);
  fireEvent.click(dialog.getByRole("button",{name:"取消"}));
  expect(invoke.mock.calls.some(([command])=>command==="model_connection_validate")).toBe(false);
  expect(form.getByRole("combobox",{name:"验证模型"}).textContent).toBe("本地模型");
});

it("已保存但目录刷新失败时保留新连接，只重试读取目录，不重复保存", async () => {
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === "model_connection_save") {
      connections = [connection];
      return { result: connection, catalogRefreshError: "model_metadata_saved_catalog_refresh_failed" };
    }
    if (command === "ai_model_catalog_read") return {};
    return base(command, args);
  });
  render(<ModelConnectionSettings/>);
  await screen.findByText("尚未添加连接");
  fireEvent.click(within(screen.getByRole("complementary",{name:"连接列表"})).getByRole("button",{name:"添加连接"}));
  const form = within(screen.getByRole("form", { name: "添加连接" }));
  fireEvent.change(form.getByLabelText("名称"), { target: { value: "本地服务" } });
  fireEvent.change(form.getByLabelText("API 密钥"), { target: { value: "synthetic-ui-component-secret" } });
  fireEvent.click(form.getByRole("button", { name: "保存" }));
  await screen.findByText("更改已保存，模型目录尚未刷新。");
  expect(screen.getByRole("form", { name: "编辑连接" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "本地服务 本地模型 · 启用" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "刷新目录" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_model_catalog_read", { reason: "retry" }));
  await waitFor(() => expect(screen.queryByText("更改已保存，模型目录尚未刷新。")).toBeNull());
  expect(invoke.mock.calls.filter(([command]) => command === "model_connection_save")).toHaveLength(1);
});

it("取消后残留凭据只能显式清理，不重新保存或恢复连接", async () => {
  const record: ModelConnectionChange = { intent: "connection_save", operationId: "33333333333333333333333333333333", requestDigest: "test-request",
    expectedConfigRevision: null, connection: { ...connection, secretRef: { service: "cn.cilai.personal.workbench.vault.model", account: `${connection.profileId}:${connection.connectionId}:1` } },
    credentialAction: "replace", networkCredentialAction: "keep", state: "aborted", credentialCleanup: "pending", preparedAt: 1 };
  let pending = record;
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === "model_connection_change_list") return [pending];
    if (command === "model_connection_credential_cleanup") { pending = { ...record, credentialCleanup: "confirmed" }; return pending; }
    return base(command, args);
  });
  render(<ModelConnectionSettings/>);
  fireEvent.click(screen.getByRole("button",{name:"凭据"}));
  await screen.findByText("连接已取消，凭据清理待确认。");
  expect(screen.queryByLabelText("继续本地服务的密钥")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "清理残留凭据" }));
  await waitFor(() => expect(screen.queryByText("连接已取消，凭据清理待确认。")).toBeNull());
  expect(invoke).toHaveBeenCalledWith("model_connection_credential_cleanup", { request: { operationId: record.operationId } });
  expect(invoke.mock.calls.some(([command]) => ["model_connection_save", "model_connection_resume", "model_connection_abort"].includes(command))).toBe(false);
});

it("既有连接固定类型、服务与认证方式，密钥更新不改变连接身份", async () => {
  connections = [connection];
  render(<ModelConnectionSettings/>);
  fireEvent.click(await screen.findByRole("button", { name: "本地服务 本地模型 · 启用" }));
  const form = within(screen.getByRole("form", { name: "编辑连接" }));
  expect(form.getByRole("combobox", {name:"服务"})).toHaveProperty("disabled",true);
  fireEvent.click(form.getByRole("button",{name:"连接设置"}));
  const drawer=within(screen.getByRole("dialog",{name:"连接设置"}));
  expect(drawer.getByLabelText("类型")).toHaveProperty("readOnly",true);
  expect(drawer.getByLabelText("类型")).toHaveProperty("value","本地模型");
  expect(drawer.getByLabelText("认证")).toHaveProperty("readOnly",true);
  expect(drawer.getByLabelText("认证")).toHaveProperty("value","无认证");
  expect(drawer.getByLabelText("名称")).toHaveProperty("disabled",false);
});


it("连接设置取消后恢复修改前的名称和网络，不提交也不验证", async()=>{
  connections=[connection]; render(<ModelConnectionSettings/>);
  const form=within(await screen.findByRole("form",{name:"编辑连接"}));
  fireEvent.click(form.getByRole("button",{name:"连接设置"}));
  let drawer=within(screen.getByRole("dialog",{name:"连接设置"}));
  expect(document.activeElement).toBe(drawer.getByLabelText("名称"));
  fireEvent.change(drawer.getByLabelText("名称"),{target:{value:"未保存名称"}});
  fireEvent.click(form.getByRole("button",{name:"连接设置"}));
  await chooseSelect(drawer.getByRole("combobox",{name:"网络"}),"指定代理");
  fireEvent.change(drawer.getByLabelText("代理地址"),{target:{value:"http://127.0.0.1:12345"}});
  fireEvent.click(drawer.getByRole("button",{name:"取消"}));
  fireEvent.click(form.getByRole("button",{name:"连接设置"}));
  drawer=within(screen.getByRole("dialog",{name:"连接设置"}));
  expect(drawer.getByLabelText("名称")).toHaveProperty("value","本地服务");
  expect(drawer.getByRole("combobox",{name:"网络"}).textContent).toBe("直接连接");
  expect(drawer.queryByLabelText("代理地址")).toBeNull();
  expect(invoke.mock.calls.some(([command])=>["model_connection_save","model_connection_validate"].includes(command))).toBe(false);
});

it("新增连接内关闭网络设置只回到新增弹窗，取消新增不产生写入",async()=>{
  render(<ModelConnectionSettings/>);await screen.findByText("尚未添加连接");
  fireEvent.click(within(screen.getByRole("complementary",{name:"连接列表"})).getByRole("button",{name:"添加连接"}));
  const add=within(screen.getByRole("dialog",{name:"添加连接"}));
  fireEvent.change(add.getByLabelText("名称"),{target:{value:"保留的名称"}});
  fireEvent.click(add.getByRole("button",{name:"连接设置"}));
  fireEvent.keyDown(window,{key:"Escape"});
  expect(screen.queryByRole("dialog",{name:"连接设置"})).toBeNull();
  expect(within(screen.getByRole("dialog",{name:"添加连接"})).getByLabelText("名称")).toHaveProperty("value","保留的名称");
  fireEvent.keyDown(window,{key:"Escape"});
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(invoke.mock.calls.some(([command])=>command==="model_connection_save")).toBe(false);
});

it("初次读取失败显示错误与重试，不把未知资料呈现为空连接列表",async()=>{
  invoke.mockRejectedValue(Error("model_profile_unavailable"));
  render(<ModelConnectionSettings/>);await screen.findByRole("alert");
  expect(screen.queryByText("尚未添加连接")).toBeNull();
  expect(screen.queryByText("添加第一个连接")).toBeNull();
  expect(within(screen.getByRole("complementary",{name:"连接列表"})).getByRole("button",{name:"添加连接"})).toHaveProperty("disabled",true);
  expect(invoke.mock.calls.every(([command])=>["model_connection_list","model_connection_change_list","model_purpose_list"].includes(command))).toBe(true);
});

it("搜索连接只筛选侧栏，不修改用途、发送验证或切换模型",async()=>{
  connections=[connection];render(<ModelConnectionSettings/>);
  await screen.findByRole("form",{name:"编辑连接"});
  const before=invoke.mock.calls.length;
  fireEvent.change(screen.getByRole("searchbox",{name:"搜索连接"}),{target:{value:"没有匹配"}});
  expect(screen.getByText("没有匹配的连接")).toBeTruthy();
  expect(screen.getByRole("form",{name:"编辑连接"})).toBeTruthy();
  expect(invoke.mock.calls).toHaveLength(before);
  expect(invoke.mock.calls.some(([command])=>["model_purpose_save","model_connection_validate","model_connection_save"].includes(command))).toBe(false);
});


it("保存完成但元数据重读失败时可关闭设置，旧投影不能继续写入",async()=>{
  connections=[connection];let readFailed=false;const base=invoke.getMockImplementation()!;
  invoke.mockImplementation(async(command,args)=>{
    if(command==="model_connection_save"){readFailed=true;return{result:{...connection,configRevision:2},catalogRefreshError:null};}
    if(command==="model_connection_list"&&readFailed)throw Error("model_profile_unavailable");
    return base(command,args);
  });
  render(<ModelConnectionSettings/>);const form=within(await screen.findByRole("form",{name:"编辑连接"}));
  fireEvent.click(form.getByRole("button",{name:"连接设置"}));
  const drawer=within(screen.getByRole("dialog",{name:"连接设置"}));
  fireEvent.click(drawer.getByRole("button",{name:"保存"}));
  await drawer.findByRole("alert");
  await waitFor(()=>expect(drawer.getByRole("button",{name:"取消"})).toHaveProperty("disabled",false));
  expect(drawer.getByRole("button",{name:"保存"})).toHaveProperty("disabled",true);
  fireEvent.click(drawer.getByRole("button",{name:"取消"}));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(form.getByRole("button",{name:"保存"}).closest("fieldset")).toHaveProperty("disabled",true);
  expect(invoke.mock.calls.filter(([command])=>command==="model_connection_save")).toHaveLength(1);
});


it.each(["cn.cilai.personal.workbench.model", "cn.cilai.personal.workbench.vault.model.extra"])("模型旧引用%s必须空输入重存，不能保留成新本地凭据", async (service) => {
  connections = [{ ...connection, config: { ...connection.config, authMode: "api_key" }, secretRef: { service, account: "private-model-reference" } }];
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === "model_connection_save") throw new Error("model_credential_reentry_required");
    return original(command, args);
  });
  render(<ModelConnectionSettings/>);
  const form = within(await screen.findByRole("form", { name: "编辑连接" }));
  expect(form.getByText("需重新保存到工作台")).toBeTruthy();
  expect(form.getByLabelText("API 密钥")).toHaveProperty("value", "");
  expect(form.getByLabelText("API 密钥")).toHaveProperty("required", true);
  expect(form.queryByRole("button", { name: "取消更换" })).toBeNull();
  expect(form.queryByText("本地凭据已登记")).toBeNull();
  expect(document.body.textContent).not.toContain("private-model-reference");
  fireEvent.click(form.getByRole("button", { name: "保存" }));
  expect(invoke.mock.calls.some(([command]) => command === "model_connection_save")).toBe(false);
  fireEvent.change(form.getByLabelText("API 密钥"), { target: { value: "synthetic-reentry-only" } });
  fireEvent.click(form.getByRole("button", { name: "保存" }));
  await screen.findByRole("alert");
  const saves = invoke.mock.calls.filter(([command]) => command === "model_connection_save");
  expect(saves).toHaveLength(1);
  expect(saves[0][1].request.prepare).toMatchObject({ connectionId: connection.connectionId, expectedConfigRevision: 1, credentialAction: "replace" });
  expect(saves[0][1].request.secret).toBe("synthetic-reentry-only");
  expect(screen.getByRole("alert").textContent).toBe("请重新保存到工作台本地凭据库。刷新");
  expect(form.getByLabelText("API 密钥")).toHaveProperty("value", "");
  expect(JSON.stringify(saves[0][1].request.prepare)).not.toContain("synthetic-reentry-only");
});

it("精确本地引用只显示已登记，不读取或展示旧密钥与引用值", async () => {
  connections = [{ ...connection, config: { ...connection.config, authMode: "api_key" }, secretRef: { service: "cn.cilai.personal.workbench.vault.model", account: "private-local-model-reference" } }];
  render(<ModelConnectionSettings/>);
  const form = within(await screen.findByRole("form", { name: "编辑连接" }));
  expect(form.getByText("本地凭据已登记")).toBeTruthy();
  expect(form.getByText("工作台本地加密")).toBeTruthy();
  expect(form.queryByLabelText("API 密钥")).toBeNull();
  expect(document.body.textContent).not.toContain("private-local-model-reference");
  fireEvent.click(form.getByRole("button", { name: "更换密钥" }));
  expect(form.getByLabelText("API 密钥")).toHaveProperty("value", "");
  expect(invoke.mock.calls.every(([command]) => !/credential.*read|connection_save/.test(command))).toBe(true);
});

it("取消的旧系统凭据仅提示手工清理，不发删除或保存请求", async () => {
  const original = invoke.getMockImplementation()!;
  const record: ModelConnectionChange = { intent: "connection_save", operationId: "legacy-operation", requestDigest: "legacy",
    expectedConfigRevision: null, connection: { ...connection, secretRef: { service: "cn.cilai.personal.workbench.model", account: "private-legacy-account" } },
    credentialAction: "replace", networkCredentialAction: "keep", state: "aborted", credentialCleanup: "pending", preparedAt: 1 };
  invoke.mockImplementation(async (command: string, args: unknown) => command === "model_connection_change_list" ? [record] : original(command, args));
  render(<ModelConnectionSettings/>);
  fireEvent.click(screen.getByRole("button", { name: "凭据" }));
  await screen.findByText("旧系统凭据需手工清理，尚未完成。");
  expect(screen.queryByRole("button", { name: "清理残留凭据" })).toBeNull();
  expect(document.body.textContent).not.toContain("private-legacy-account");
  expect(invoke.mock.calls.some(([command]) => /cleanup$|connection_save|connection_resume/.test(command))).toBe(false);
});


it("旧系统凭据的已准备keep操作不能作为本地库继续保存，仍可取消", async () => {
  const original = invoke.getMockImplementation()!;
  const record: ModelConnectionChange = { intent: "connection_save", operationId: "legacy-keep-operation", requestDigest: "legacy",
    expectedConfigRevision: 1, connection: { ...connection, config: { ...connection.config, authMode: "api_key" }, secretRef: { service: "cn.cilai.personal.workbench.model", account: "private-old-reference" } },
    credentialAction: "keep", networkCredentialAction: "keep", state: "prepared", credentialCleanup: "not_required", preparedAt: 1 };
  invoke.mockImplementation(async (command: string, args: unknown) => command === "model_connection_change_list" ? [record] : original(command, args));
  render(<ModelConnectionSettings/>);fireEvent.click(screen.getByRole("button", { name: "凭据" }));
  await screen.findByText("需重新保存到工作台。请取消此操作后重新输入凭据。");
  expect(screen.getByRole("button", { name: "继续" })).toHaveProperty("disabled", true);
  expect(screen.getByRole("button", { name: "取消操作" })).toHaveProperty("disabled", false);
  fireEvent.click(screen.getByRole("button", { name: "继续" }));
  expect(invoke.mock.calls.some(([command]) => command === "model_connection_resume")).toBe(false);
  expect(document.body.textContent).not.toContain("private-old-reference");
});


it("切换连接不会把尚未保存的密钥带到另一个资料作用域", async () => {
  const first: ModelConnection = { ...connection, config: { ...connection.config, authMode: "api_key" }, secretRef: { service: "cn.cilai.personal.workbench.model", account: "legacy-one" } };
  const second: ModelConnection = { ...first, connectionId: "44444444444444444444444444444444", config: { ...first.config, name: "另一个连接" }, secretRef: { service: "cn.cilai.personal.workbench.model", account: "legacy-two" } };
  connections = [first, second];
  render(<ModelConnectionSettings/>);
  const firstForm = within(await screen.findByRole("form", { name: "编辑连接" }));
  fireEvent.change(firstForm.getByLabelText("API 密钥"), { target: { value: "synthetic-unsaved-first-key" } });
  fireEvent.click(screen.getByRole("button", { name: "另一个连接 本地模型 · 启用" }));
  expect(within(screen.getByRole("form", { name: "编辑连接" })).getByLabelText("API 密钥")).toHaveProperty("value", "");
  fireEvent.click(screen.getByRole("button", { name: "本地服务 本地模型 · 启用" }));
  expect(within(screen.getByRole("form", { name: "编辑连接" })).getByLabelText("API 密钥")).toHaveProperty("value", "");
  expect(document.body.innerHTML).not.toContain("synthetic-unsaved-first-key");
  expect(invoke.mock.calls.some(([command]) => command === "model_connection_save")).toBe(false);
});
