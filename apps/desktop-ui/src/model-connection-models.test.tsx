import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ConnectionModels } from "./model-connection-models";
import type { ModelConnection, ModelDescriptor, ModelValidationRecord } from "./generated/protocol";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
const connection: ModelConnection = { profileId: "1".repeat(32), connectionId: "2".repeat(32), configRevision: 4, authGeneration: 7, authScopes: [], secretRef: null, deleted: false, createdAt: 1, updatedAt: 1,
  config: { name: "我的服务", kind: "custom_responses", baseUrl: "https://fixture.invalid/v1", authMode: "api_key", localRuntime: null, enabled: true, network: { mode: "direct", proxyUrl: null, proxySecretRef: null, proxyAuth: "none", serviceAuth: "none", headerNames: [] } } };
const model: ModelDescriptor = { descriptorId: "one", connectionId: connection.connectionId, modelId: "nvidia/model:free", displayName: "模型一", revision: 6, configRevision: 4, authGeneration: 7, source: "service_declared", capabilities: {}, contextWindowTokens: 128000, inputModalities: ["text", "image"], defaultReasoningEffort: "high", supportedReasoningEfforts: ["low", "high"], runtimeVersion: null, serviceVersion: null, updatedAt: 1 };
const onChanged = vi.fn(async () => {});
let models: ModelDescriptor[];
let records: ModelValidationRecord[];
let activeConnection: ModelConnection;
let validationSequence: number;
const order = ["configuration", "network", "authentication", "protocol", "catalog", "inference", "streaming", "tools"];
function receipt(request: Record<string, unknown>, patch: Partial<ModelValidationRecord> = {}): ModelValidationRecord {
  return { runtimeKind: request.runtimeKind as ModelValidationRecord["runtimeKind"], validationId: `validation-${++validationSequence}`,
    connectionId: String(request.connectionId), modelId: request.modelId as string | null, layer: request.layer as ModelValidationRecord["layer"],
    configRevision: Number(request.expectedConfigRevision), authGeneration: Number(request.expectedAuthGeneration), outcome: "passed", completionRevision: validationSequence,
    reason: null, runtimeVersion: "0.163.0", serviceVersion: null, updatedAt: 1, ...patch };
}
const validationCalls = () => invoke.mock.calls.filter(([name]) => name === "model_connection_validate");
async function confirmAll(title = "验证模型调用？") {
  fireEvent.click(screen.getByRole("form", { name: "连接验证" }).querySelector('button[type="submit"]')!);
  fireEvent.click(within(screen.getByRole("dialog", { name: title })).getByRole("button", { name: "全部验证" }));
}
async function finished(expected: number) {
  await waitFor(() => expect(validationCalls()).toHaveLength(expected));
  await waitFor(() => expect(within(screen.getByRole("form", { name: "连接验证" })).getByRole("button", { name: "全部验证" })).toHaveProperty("disabled", false));
}
beforeEach(() => {
  models = [model, { ...model, descriptorId: "two", modelId: "openai/model-two", displayName: "模型二", source: "user_declared", inputModalities: ["text"] }];
  records = []; activeConnection = connection; validationSequence = 0;
  onChanged.mockReset(); onChanged.mockResolvedValue(); invoke.mockReset();
  invoke.mockImplementation(async (command: string, args: { request: Record<string, unknown> }) => {
    if (command === "model_connection_list") return structuredClone([activeConnection]);
    if (command === "model_descriptor_list") return structuredClone(models);
    if (command === "model_validation_list") return structuredClone(records);
    if (command === "model_connection_catalog_refresh") return structuredClone(models);
    if (command === "model_descriptor_save") return { result: model, catalogRefreshError: null };
    if (command === "model_connection_validate") return receipt(args.request);
    throw Error(`unexpected ${command}`);
  });
});
afterEach(cleanup);
async function ready(props = connection) {
  activeConnection = props;
  const view = render(<ConnectionModels connection={props} disabled={false} onChanged={onChanged}/>);
  await screen.findByText("模型一");
  return view;
}
async function choose(control: HTMLElement, label: string) {
  fireEvent.click(control); fireEvent.click(await screen.findByRole("option", { name: label }));
}
async function validation(props = connection) {
  await ready(props); fireEvent.click(screen.getByRole("button", { name: "验证连接" }));
  const form = within(screen.getByRole("form", { name: "连接验证" }));
  await choose(form.getByRole("combobox", { name: "验证模型" }), "模型一");
  return form;
}
describe("紧凑模型管理", () => {
  it("进入仅读元数据，按真实ID前缀分组及声明输入显示，不声称在线或已验证", async () => {
    await ready();
    expect(invoke.mock.calls.map(([name]) => name).sort()).toEqual(["model_descriptor_list", "model_validation_list"].sort());
    expect(screen.getByText("nvidia")).toBeTruthy(); expect(screen.getByText("openai")).toBeTruthy();
    expect(screen.getByText("图片")).toBeTruthy(); expect(screen.getByText("目录声明")).toBeTruthy(); expect(screen.getByText("手动声明")).toBeTruthy();
    expect(screen.queryByText("已连接")).toBeNull(); expect(screen.queryByRole("form", { name: "连接验证" })).toBeNull();
    fireEvent.change(screen.getByRole("searchbox", { name: "搜索模型" }), { target: { value: "model-two" } });
    expect(screen.queryByText("模型一")).toBeNull(); expect(screen.getByText("模型二")).toBeTruthy();
    expect(invoke.mock.calls).toHaveLength(2);
  });
  it("添加弹窗取消不保存，Escape关闭并恢复触发焦点", async () => {
    await ready(); const trigger = screen.getByRole("button", { name: "添加模型" }); trigger.focus(); fireEvent.click(trigger);
    const modal = within(screen.getByRole("dialog", { name: "添加模型" }));
    fireEvent.change(modal.getByLabelText("模型 ID"), { target: { value: "new/model" } });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull(); expect(document.activeElement).toBe(trigger);
    expect(invoke.mock.calls.some(([name]) => name === "model_descriptor_save")).toBe(false);
  });
  it("手动登记仅发送真实字段，未知不填能力或默认Token", async () => {
    await ready(); fireEvent.click(screen.getByRole("button", { name: "添加模型" }));
    const dialog = within(screen.getByRole("dialog", { name: "添加模型" }));
    fireEvent.change(dialog.getByLabelText("模型 ID"), { target: { value: "custom/model" } });
    fireEvent.click(dialog.getByRole("button", { name: "登记模型" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("model_descriptor_save", { request: { connectionId: connection.connectionId, modelId: "custom/model", displayName: "custom/model", expectedRevision: null, contextWindowTokens: null, inputModalities: [], defaultReasoningEffort: null, supportedReasoningEfforts: [] } }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(invoke.mock.calls.some(([name]) => name === "model_connection_validate")).toBe(false);
  });
  it("编辑保留来源已有声明与精确修订，不能改成另一个模型ID", async () => {
    await ready(); fireEvent.click(screen.getByRole("button", { name: "编辑 模型一" }));
    const dialog = within(screen.getByRole("dialog", { name: "编辑模型" }));
    expect(dialog.getByLabelText("模型 ID")).toHaveProperty("readOnly", true);
    fireEvent.change(dialog.getByLabelText("显示名称"), { target: { value: "新名称" } });
    fireEvent.click(dialog.getByRole("button", { name: "保存模型" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("model_descriptor_save", { request: { connectionId: connection.connectionId, modelId: model.modelId, displayName: "新名称", expectedRevision: 6, contextWindowTokens: 128000, inputModalities: ["text", "image"], defaultReasoningEffort: "high", supportedReasoningEfforts: ["low", "high"] } }));
  });
  it("保存失败保留表单且不自动重做，错误清楚可见", async () => {
    const base = invoke.getMockImplementation()!; invoke.mockImplementation(async (name, args) => name === "model_descriptor_save" ? Promise.reject(Error("model_revision_conflict")) : base(name, args));
    await ready(); fireEvent.click(screen.getByRole("button", { name: "编辑 模型一" }));
    const dialog = within(screen.getByRole("dialog", { name: "编辑模型" }));
    fireEvent.click(dialog.getByRole("button", { name: "保存模型" }));
    await dialog.findByRole("alert"); expect(dialog.getByLabelText("模型 ID")).toHaveProperty("value", model.modelId);
    expect(invoke.mock.calls.filter(([name]) => name === "model_descriptor_save")).toHaveLength(1);
    fireEvent.click(dialog.getByRole("button", { name: "取消" })); expect(screen.queryByRole("dialog")).toBeNull();
  });
  it("已记录目录只浏览和筛选，显式读取才发请求且不假添加", async () => {
    await ready(); fireEvent.click(screen.getByRole("button", { name: "查看目录" }));
    const modal = within(screen.getByRole("dialog", { name: "模型目录" }));
    expect(modal.getAllByText("已记录")).toHaveLength(2);
    fireEvent.click(modal.getByRole("button", { name: "图片 1" }));
    expect(modal.queryByText("模型二")).toBeNull();
    expect(invoke.mock.calls.some(([name]) => name === "model_connection_catalog_refresh")).toBe(false);
    fireEvent.click(modal.getByRole("button", { name: "读取目录" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("model_connection_catalog_refresh", { request: { connectionId: connection.connectionId } }));
    expect(invoke.mock.calls.some(([name]) => name === "model_descriptor_save")).toBe(false);
    await waitFor(() => expect(modal.getByRole("button", { name: "完成" })).toHaveProperty("disabled", false));
    fireEvent.click(modal.getByRole("button", { name: "完成" }));
  });
  it("目录读取失败可见且不自动重试、不显示假已添加", async () => {
    const base = invoke.getMockImplementation()!; invoke.mockImplementation(async (name, args) => name === "model_connection_catalog_refresh" ? Promise.reject(Error("model_catalog_request_failed")) : base(name, args));
    await ready(); fireEvent.click(screen.getByRole("button", { name: "查看目录" }));
    const modal = within(screen.getByRole("dialog", { name: "模型目录" }));
    fireEvent.click(modal.getByRole("button", { name: "读取目录" })); await modal.findByRole("alert");
    expect(invoke.mock.calls.filter(([name]) => name === "model_connection_catalog_refresh")).toHaveLength(1);
    expect(modal.queryByText("已添加")).toBeNull();
  });
  it("所选模型一次确认后按八层顺序各一次，元数据代次为0、其余为冻结代次", async () => {
    const form = await validation(); expect(form.queryByRole("combobox", { name: "验证层级" })).toBeNull();
    await confirmAll(); await finished(8);
    expect(validationCalls().map(([, args]) => args.request.layer)).toEqual(order);
    for (const [, args] of validationCalls()) expect(args.request).toEqual({ runtimeKind: "workbench", connectionId: connection.connectionId, modelId: model.modelId,
      layer: args.request.layer, expectedConfigRevision: 4, expectedAuthGeneration: ["configuration", "network", "catalog"].includes(args.request.layer) ? 0 : 7 });
    expect(document.querySelectorAll('.model-library-validation [data-outcome="passed"]')).toHaveLength(8);
    expect(onChanged).toHaveBeenCalledTimes(1);
  });
  it("真实调用需模型与单次费用确认，取消或Escape零验证并保留面板", async () => {
    await ready(); fireEvent.click(screen.getByRole("button", { name: "验证连接" }));
    const form = within(screen.getByRole("form", { name: "连接验证" }));
    expect(form.getByRole("button", { name: "全部验证" })).toHaveProperty("disabled", true);
    await choose(form.getByRole("combobox", { name: "验证模型" }), "模型一");
    fireEvent.click(form.getByRole("button", { name: "全部验证" }));
    const modal = within(screen.getByRole("dialog", { name: "验证模型调用？" }));
    expect(modal.getByText(/隔离测试工具/)).toBeTruthy(); expect(modal.getByText(/不发送用户历史，可能产生服务费用/)).toBeTruthy();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "验证模型调用？" })).toBeNull(); expect(screen.getByRole("form", { name: "连接验证" })).toBeTruthy();
    expect(validationCalls()).toHaveLength(0);
    fireEvent.click(form.getByRole("button", { name: "全部验证" }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "验证模型调用？" })).getByRole("button", { name: "取消" }));
    expect(validationCalls()).toHaveLength(0);
    await confirmAll(); await finished(8);
  });
  it("ChatGPT未授权用途只做三项元数据，已授权用途按真实代次跑八项且不借旧记录", async () => {
    records = [{ validationId: "old", connectionId: connection.connectionId, runtimeKind: "workbench", modelId: model.modelId, layer: "tools", configRevision: 4, authGeneration: 2, outcome: "passed", completionRevision: 1, reason: null, runtimeVersion: null, serviceVersion: null, updatedAt: 1 }];
    const cg = { ...connection, config: { ...connection.config, kind: "chatgpt" as const, authMode: "runtime_managed_chatgpt" as const }, authScopes: [{ runtimeKind: "workbench" as const, authGeneration: 3, state: "active" as const, operationId: "auth" }] };
    const form = await validation(cg);
    expect(document.querySelectorAll('.model-library-validation [data-outcome="passed"]')).toHaveLength(0);
    await choose(form.getByRole("combobox", { name: "验证用途" }), "页面 AI");
    expect(form.getByText("此用途未授权，仅检查配置、网络和目录。")).toBeTruthy();
    fireEvent.click(form.getByRole("button", { name: "全部验证" }));
    const modal = within(screen.getByRole("dialog", { name: "验证配置？" })); expect(modal.getByText(/不调用模型，不发送用户历史/)).toBeTruthy();
    fireEvent.click(modal.getByRole("button", { name: "取消" })); expect(validationCalls()).toHaveLength(0);
    await confirmAll("验证配置？"); await finished(3);
    expect(validationCalls().map(([, args]) => args.request.layer)).toEqual(["configuration", "network", "catalog"]);
    expect(validationCalls().every(([, args]) => args.request.runtimeKind === "application" && args.request.expectedAuthGeneration === 0)).toBe(true);
    expect(form.getAllByText("此用途未授权")).toHaveLength(5);
    await choose(form.getByRole("combobox", { name: "验证用途" }), "主对话");
    await confirmAll(); await finished(11);
    expect(validationCalls().slice(3).map(([, args]) => args.request.layer)).toEqual(order);
    expect(validationCalls().slice(3).every(([, args]) => args.request.expectedAuthGeneration === (["configuration", "network", "catalog"].includes(args.request.layer) ? 0 : 3))).toBe(true);
  });
});

it("保存进行中不允许Escape关闭，也不因重复点击重发", async () => {
  const base = invoke.getMockImplementation()!;
  let finish!: (value: unknown) => void;
  invoke.mockImplementation(async (name, args) => name === "model_descriptor_save" ? new Promise((resolve) => { finish = resolve; }) : base(name, args));
  await ready(); fireEvent.click(screen.getByRole("button", { name: "编辑 模型一" }));
  const modal = within(screen.getByRole("dialog", { name: "编辑模型" }));
  fireEvent.click(modal.getByRole("button", { name: "保存模型" }));
  await waitFor(() => expect(modal.getByRole("button", { name: "保存中…" })).toHaveProperty("disabled", true));
  fireEvent.keyDown(window, { key: "Escape" }); expect(screen.getByRole("dialog", { name: "编辑模型" })).toBeTruthy();
  fireEvent.click(modal.getByRole("button", { name: "保存中…" }));
  expect(invoke.mock.calls.filter(([name]) => name === "model_descriptor_save")).toHaveLength(1);
  finish({ result: model, catalogRefreshError: null }); await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
});
it("普通完整失败保留模型与原因并继续独立层，每层不自动重试", async () => {
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name, args) => name === "model_connection_validate" && args.request.layer === "authentication"
    ? receipt(args.request, { outcome: "failed", reason: "model_validation_authentication_not_established" }) : base(name, args));
  const form = await validation(); await confirmAll(); await finished(8);
  expect(form.getByRole("combobox", { name: "验证模型" }).textContent).toBe("模型一");
  expect(validationCalls().map(([, args]) => args.request.layer)).toEqual(order);
  expect(document.querySelectorAll('.model-library-validation [data-outcome="passed"]')).toHaveLength(7);
  expect(document.querySelector('.model-library-validation [data-layer="authentication"]')?.textContent).toContain("失败");
  expect(document.querySelector('.model-library-validation [data-layer="authentication"] .model-library-validation-reason')?.textContent).toBeTruthy();
});

it("首次元数据读取失败显示错误，不伪装成空目录", async () => {
  invoke.mockRejectedValue(Error("model_metadata_unavailable"));
  render(<ConnectionModels connection={connection} disabled={false} onChanged={onChanged}/>);
  await screen.findByRole("alert");
  expect(screen.queryByText("尚无模型记录")).toBeNull();
  expect(screen.getByRole("button", { name: "添加模型" })).toHaveProperty("disabled", true);
  expect(screen.getByRole("button", { name: "重试读取" })).toBeTruthy();
  expect(invoke.mock.calls.some(([name]) => name === "model_connection_validate")).toBe(false);
});
it("切换连接时不显示旧资料的模型或把迟到结果当成当前目录", async () => {
  const { rerender } = render(<ConnectionModels connection={connection} disabled={false} onChanged={onChanged}/>);
  await screen.findByText("模型一");
  invoke.mockImplementation(() => new Promise(() => {}));
  rerender(<ConnectionModels connection={{ ...connection, connectionId: "3".repeat(32) }} disabled={false} onChanged={onChanged}/>);
  expect(screen.queryByText("模型一")).toBeNull();
  expect(screen.queryByText("尚无模型记录")).toBeNull();
  expect(screen.getByRole("button", { name: "添加模型" })).toHaveProperty("disabled", true);
});

it("目录读取成功后本地元数据读取失败仍可关闭或显式重读，不授权旧记录", async () => {
  const base = invoke.getMockImplementation()!;
  let failRead = false;
  invoke.mockImplementation(async (name, args) => {
    if (name === "model_connection_catalog_refresh") { failRead = true; return structuredClone(models); }
    if (name === "model_validation_list" && failRead) throw Error("model_metadata_unavailable");
    return base(name, args);
  });
  await ready(); fireEvent.click(screen.getByRole("button", { name: "查看目录" }));
  const modal = within(screen.getByRole("dialog", { name: "模型目录" }));
  fireEvent.click(modal.getByRole("button", { name: "读取目录" }));
  await modal.findByRole("alert");
  expect(modal.getByText("目录不可用")).toBeTruthy();
  expect(modal.queryByText("模型一")).toBeNull(); expect(modal.queryByText("没有匹配的模型")).toBeNull();
  expect(modal.getByRole("button", { name: "读取目录" })).toHaveProperty("disabled", true);
  expect(modal.getByRole("button", { name: "关闭" })).toHaveProperty("disabled", false);
  expect(modal.getByRole("button", { name: "完成" })).toHaveProperty("disabled", false);
  failRead = false; fireEvent.click(modal.getByRole("button", { name: "重试读取" }));
  await modal.findByText("模型一");
  expect(invoke.mock.calls.filter(([name]) => name === "model_connection_catalog_refresh")).toHaveLength(1);
  expect(invoke.mock.calls.filter(([name]) => name === "model_validation_list")).toHaveLength(3);
  fireEvent.keyDown(window, { key: "Escape" }); expect(screen.queryByRole("dialog")).toBeNull();
});
it.each(["关闭", "完成", "Escape"])("父页读取不可用时目录仍可用%s关闭，禁止写和目录调用", async (action) => {
  const { rerender } = render(<ConnectionModels connection={connection} disabled={false} onChanged={onChanged}/>);
  await screen.findByText("模型一"); fireEvent.click(screen.getByRole("button", { name: "查看目录" }));
  rerender(<ConnectionModels connection={connection} disabled onChanged={onChanged}/>);
  const modal = within(screen.getByRole("dialog", { name: "模型目录" }));
  expect(modal.getByRole("button", { name: "读取目录" })).toHaveProperty("disabled", true);
  expect(modal.getByRole("button", { name: "编辑 模型一" })).toHaveProperty("disabled", true);
  if (action === "Escape") fireEvent.keyDown(window, { key: "Escape" });
  else fireEvent.click(modal.getByRole("button", { name: action }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(invoke.mock.calls.some(([name]) => name === "model_connection_catalog_refresh" || name === "model_descriptor_save" || name === "model_connection_validate")).toBe(false);
});
it("目录请求实际进行中禁止关闭且不重复发送，响应后恢复关闭", async () => {
  const base = invoke.getMockImplementation()!; let finish!: (value: unknown) => void;
  invoke.mockImplementation(async (name, args) => name === "model_connection_catalog_refresh" ? new Promise((resolve) => { finish = resolve; }) : base(name, args));
  await ready(); fireEvent.click(screen.getByRole("button", { name: "查看目录" }));
  const modal = within(screen.getByRole("dialog", { name: "模型目录" }));
  fireEvent.click(modal.getByRole("button", { name: "读取目录" }));
  expect(modal.getByRole("button", { name: "完成" })).toHaveProperty("disabled", true);
  fireEvent.keyDown(window, { key: "Escape" }); expect(screen.getByRole("dialog", { name: "模型目录" })).toBeTruthy();
  fireEvent.click(modal.getByRole("button", { name: "读取目录" }));
  expect(invoke.mock.calls.filter(([name]) => name === "model_connection_catalog_refresh")).toHaveLength(1);
  finish(models); await waitFor(() => expect(modal.getByRole("button", { name: "完成" })).toHaveProperty("disabled", false));
  fireEvent.click(modal.getByRole("button", { name: "完成" })); expect(screen.queryByRole("dialog")).toBeNull();
});

it("本轮开始后历史通过不填本轮状态，进行中串行等待且防重复确认或关闭", async () => {
  records = order.map((layer) => receipt({ runtimeKind: "workbench", connectionId: connection.connectionId, modelId: model.modelId, layer, expectedConfigRevision: 4, expectedAuthGeneration: ["configuration", "network", "catalog"].includes(layer) ? 0 : 7 }));
  const base = invoke.getMockImplementation()!; let finish!: () => void;
  invoke.mockImplementation(async (name, args) => name === "model_connection_validate" && args.request.layer === "configuration" ? new Promise(resolve => { finish = () => resolve(receipt(args.request)); }) : base(name, args));
  const form = await validation(); expect(form.getAllByText("上次通过")).toHaveLength(8);
  await confirmAll(); await waitFor(() => expect(validationCalls()).toHaveLength(1));
  expect(form.queryByText("上次通过")).toBeNull(); expect(form.getByText("进行中")).toBeTruthy(); expect(form.getAllByText("待执行")).toHaveLength(7);
  expect(document.querySelectorAll('.model-library-validation [data-outcome="passed"]')).toHaveLength(0);
  fireEvent.submit(screen.getByRole("form", { name: "连接验证" })); fireEvent.click(form.getByRole("button", { name: "验证中…" }));
  fireEvent.click(form.getByRole("button", { name: "关闭" })); fireEvent.keyDown(window, { key: "Escape" });
  expect(screen.getByRole("form", { name: "连接验证" })).toBeTruthy(); expect(validationCalls()).toHaveLength(1);
  await act(async () => finish()); await finished(8);
});
it("目录未列出模型的完整失败不冒充推理失败，记录仍存在时继续独立验证", async () => {
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name, args) => name === "model_connection_validate" && args.request.layer === "catalog" ? receipt(args.request, { outcome: "failed", reason: "model_validation_model_not_in_catalog" }) : base(name, args));
  await validation(); await confirmAll(); await finished(8);
  expect(document.querySelector('[data-layer="catalog"]')?.getAttribute("data-outcome")).toBe("failed");
  expect(document.querySelector('[data-layer="inference"]')?.getAttribute("data-outcome")).toBe("passed");
});
it.each(["model_descriptor_stale", "model_descriptor_not_found", "model_validation_model_required", "model_credential_access_failed", "model_network_credential_required", "model_proxy_authentication_rejected", "model_validation_network_failed", "model_validation_timeout", "model_validation_cleanup_unconfirmed", "model_validation_failed", "unknown_future_reason"])("安全或未知失败%s保留真实失败并停止剩余，不自动重做", async reason => {
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name, args) => name === "model_connection_validate" ? receipt(args.request, { outcome: "failed", reason }) : base(name, args));
  const form = await validation(); await confirmAll(); await finished(1);
  expect(document.querySelector('[data-layer="configuration"]')?.getAttribute("data-outcome")).toBe("failed");
  expect(form.getAllByText("未执行")).toHaveLength(7); expect(form.getByRole("alert")).toBeTruthy(); expect(onChanged).toHaveBeenCalledTimes(1);
  if (reason === "unknown_future_reason") expect(form.getByRole("alert").textContent).toContain(reason);
});
it.each(["model_validation_cleanup_unconfirmed", "model_validation_stale", "model_connection_not_found", "request_timeout"])("请求异常%s标结果未确认并停止剩余", async reason => {
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name, args) => name === "model_connection_validate" ? Promise.reject(Error(reason)) : base(name, args));
  const form = await validation(); await confirmAll(); await finished(1);
  expect(form.getByText("结果未确认")).toBeTruthy(); expect(form.getAllByText("未执行")).toHaveLength(7);
  expect(document.querySelectorAll('.model-library-validation [data-outcome="passed"]')).toHaveLength(0);
});
it.each(["connectionId", "runtimeKind", "modelId", "layer", "configRevision", "authGeneration", "validationId", "completionRevision", "outcome"])("不匹配或不完整回执%s不当成功、不继续", async field => {
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name, args) => {
    if (name !== "model_connection_validate") return base(name, args);
    const value: Record<string, unknown> = { ...receipt(args.request) };
    value[field] = field === "outcome" || field === "completionRevision" ? null : field === "configRevision" || field === "authGeneration" ? 999 : field === "validationId" ? "" : "other";
    return value;
  });
  const form = await validation(); await confirmAll(); await finished(1);
  expect(form.getByText("结果未确认")).toBeTruthy(); expect(form.getAllByText("未执行")).toHaveLength(7);
});
it("重复旧回执ID不当作本轮新结果", async () => {
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name, args) => name === "model_connection_validate" ? receipt(args.request, { validationId: "same-id" }) : base(name, args));
  const form = await validation(); await confirmAll(); await finished(2);
  expect(form.getByText("结果未确认")).toBeTruthy(); expect(form.getAllByText("未执行")).toHaveLength(6);
});
it.each(["config", "auth", "disabled", "deleted", "missing"])("每层前读取真实连接，%s变化不借新scope继续", async change => {
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name, args) => {
    if (name === "model_connection_list" && change === "missing" && validationCalls().length) return [];
    if (name !== "model_connection_validate") return base(name, args);
    const value = receipt(args.request);
    activeConnection = change === "config" ? { ...connection, configRevision: 5 } : change === "auth" ? { ...connection, authGeneration: 8 }
      : change === "disabled" ? { ...connection, config: { ...connection.config, enabled: false } } : change === "deleted" ? { ...connection, deleted: true } : connection;
    return value;
  });
  const form = await validation(); await confirmAll(); await finished(1);
  expect(document.querySelector('[data-layer="configuration"]')?.getAttribute("data-outcome")).toBe("passed");
  expect(form.queryByText("结果未确认")).toBeNull(); expect(form.getAllByText("未执行")).toHaveLength(7);
});
it("模型目录正常推进修订不自我中断，真正移除则停剩余", async () => {
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name, args) => {
    if (name !== "model_connection_validate") return base(name, args);
    if (args.request.layer === "catalog") models = models.map(item => ({ ...item, revision: item.revision + 1 }));
    return receipt(args.request);
  });
  await validation(); await confirmAll(); await finished(8);
  expect(document.querySelectorAll('.model-library-validation [data-outcome="passed"]')).toHaveLength(8);
  invoke.mockImplementation(async (name, args) => {
    if (name !== "model_connection_validate") return base(name, args);
    if (args.request.layer === "catalog") models = [];
    return receipt(args.request);
  });
  await confirmAll(); await waitFor(() => expect(validationCalls()).toHaveLength(13));
  await screen.findByText("所选模型已移除，已停止后续验证。", { selector: "p" });
  expect(document.querySelector('[data-layer="inference"]')?.getAttribute("data-outcome")).toBe("not_run");
  expect(document.querySelectorAll('.model-library-validation [data-outcome="not_run"]')).toHaveLength(3);
});
it.each(["auth", "config", "connection", "disabled", "unmount"])("在途%s变化或卸载不发送下一层，也不冒用新资料", async change => {
  const base = invoke.getMockImplementation()!; let finish!: () => void;
  invoke.mockImplementation(async (name, args) => name === "model_connection_validate" ? new Promise(resolve => { finish = () => resolve(receipt(args.request)); }) : base(name, args));
  const view = await ready(); fireEvent.click(screen.getByRole("button", { name: "验证连接" }));
  await choose(within(screen.getByRole("form", { name: "连接验证" })).getByRole("combobox", { name: "验证模型" }), "模型一");
  await confirmAll(); await waitFor(() => expect(validationCalls()).toHaveLength(1));
  if (change === "unmount") view.unmount();
  else {
    const changed = change === "auth" ? { ...connection, authGeneration: 8 } : change === "config" ? { ...connection, configRevision: 5 }
      : change === "connection" ? { ...connection, connectionId: "3".repeat(32) } : connection;
    view.rerender(<ConnectionModels connection={changed} disabled={change === "disabled"} onChanged={onChanged}/>);
  }
  await act(async () => finish());
  expect(validationCalls()).toHaveLength(1);
  expect(document.querySelectorAll('.model-library-validation [data-outcome="passed"]')).toHaveLength(change === "disabled" ? 1 : 0);
  if (change === "unmount") expect(onChanged).not.toHaveBeenCalled();
});
it("最终读取失败保留八项真实回执，可关闭或仅重新读取，不重发验证", async () => {
  onChanged.mockRejectedValueOnce(Error("metadata_refresh_failed"));
  const form = await validation(); await confirmAll(); await finished(8); await form.findByRole("alert");
  expect(document.querySelectorAll('.model-library-validation [data-outcome="passed"]')).toHaveLength(8);
  expect(form.getByRole("button", { name: "关闭" })).toHaveProperty("disabled", false);
  fireEvent.click(form.getByRole("button", { name: "重新读取" }));
  await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(2)); expect(validationCalls()).toHaveLength(8);
  fireEvent.click(form.getByRole("button", { name: "关闭" })); expect(screen.queryByRole("dialog")).toBeNull();
});

it("发送前读取失败只标未执行，不能把未发送的验证称为结果未知", async () => {
  const form = await validation(); const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name, args) => name === "model_connection_list" ? Promise.reject(Error("connection_read_failed")) : base(name, args));
  await confirmAll(); await finished(0);
  expect(form.getAllByText("未执行")).toHaveLength(8); expect(form.queryByText("结果未确认")).toBeNull();
});
it("合法完成回执先保留，再因父读取不可用停止后续，关闭和重读仍可用", async () => {
  const base = invoke.getMockImplementation()!; let finish!: () => void;
  invoke.mockImplementation(async (name, args) => name === "model_connection_validate" ? new Promise(resolve => { finish = () => resolve(receipt(args.request)); }) : base(name, args));
  const view = await ready(); fireEvent.click(screen.getByRole("button", { name: "验证连接" }));
  const form = within(screen.getByRole("form", { name: "连接验证" })); await choose(form.getByRole("combobox", { name: "验证模型" }), "模型一");
  await confirmAll(); await waitFor(() => expect(validationCalls()).toHaveLength(1));
  view.rerender(<ConnectionModels connection={connection} disabled onChanged={onChanged}/>);
  await act(async () => finish());
  expect(document.querySelector('[data-layer="configuration"]')?.getAttribute("data-outcome")).toBe("passed");
  expect(form.queryByText("结果未确认")).toBeNull(); expect(form.getAllByText("未执行")).toHaveLength(7); expect(validationCalls()).toHaveLength(1);
  expect(form.getByRole("button", { name: "重新读取" })).toHaveProperty("disabled", false);
  fireEvent.click(form.getByRole("button", { name: "关闭" })); expect(screen.queryByRole("dialog")).toBeNull();
});
it("回传已知历史validationId不冒充本轮结果", async () => {
  records = [receipt({ runtimeKind: "workbench", connectionId: connection.connectionId, modelId: model.modelId, layer: "configuration", expectedConfigRevision: 4, expectedAuthGeneration: 0 })];
  const old = records[0]; const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name, args) => name === "model_connection_validate" ? old : base(name, args));
  const form = await validation(); await confirmAll(); await finished(1);
  expect(form.getByText("结果未确认")).toBeTruthy(); expect(form.getAllByText("未执行")).toHaveLength(7);
});
it("确认期间变更用途使原确认失效，重新确认才使用新用途", async () => {
  const form = await validation(); fireEvent.click(form.getByRole("button", { name: "全部验证" }));
  expect(screen.getByRole("dialog", { name: "验证模型调用？" })).toBeTruthy();
  await choose(form.getByRole("combobox", { name: "验证用途" }), "页面 AI");
  expect(screen.queryByRole("dialog", { name: "验证模型调用？" })).toBeNull(); expect(validationCalls()).toHaveLength(0);
  fireEvent.click(form.getByRole("button", { name: "全部验证" }));
  expect(within(screen.getByRole("dialog", { name: "验证模型调用？" })).getByText(/（页面 AI）依次验证/)).toBeTruthy();
  fireEvent.click(within(screen.getByRole("dialog", { name: "验证模型调用？" })).getByRole("button", { name: "取消" })); expect(validationCalls()).toHaveLength(0);
});

it("已知运行时失败通过统一中文映射逐项显示，保留真实状态与八项顺序", async () => {
  const base = invoke.getMockImplementation()!;
  const failedLayers = ["protocol", "inference", "streaming"];
  invoke.mockImplementation(async (name, args) => name === "model_connection_validate" && failedLayers.includes(args.request.layer)
    ? receipt(args.request, { outcome: "failed", reason: "model_validation_runtime_failed" }) : base(name, args));
  const form = await validation(); await confirmAll(); await finished(8);
  expect(validationCalls().map(([, args]) => args.request.layer)).toEqual(order);
  for (const layer of failedLayers) {
    const item = document.querySelector(`.model-library-validation [data-layer="${layer}"]`);
    expect(item?.getAttribute("data-outcome")).toBe("failed");
    expect(item?.querySelector(".model-library-validation-reason")?.textContent).toBe("模型验证未完成，运行时返回了错误。");
  }
  expect(form.queryByText("model_validation_runtime_failed")).toBeNull();
  expect(document.querySelectorAll('.model-library-validation [data-outcome="passed"]')).toHaveLength(5);
});

it("旧模型声明明确显示重新确认，停止在协议前且不重试", async () => {
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name, args) => name === "model_connection_validate" && args.request.layer === "protocol"
    ? receipt(args.request, { outcome: "failed", reason: "model_descriptor_stale" }) : base(name, args));
  const form = await validation(); await confirmAll(); await finished(4);
  expect(validationCalls().map(([, args]) => args.request.layer)).toEqual(order.slice(0,4));
  expect(form.getAllByText("未执行")).toHaveLength(4);
  expect(form.getByText("模型记录与当前连接配置不一致，请编辑模型并保存后验证。", {selector: ".model-library-validation-reason"})).toBeTruthy();
  expect(form.queryByText("model_validation_failed")).toBeNull();
});

it.each([{passed:true,reverse:false},{passed:true,reverse:true},{passed:false,reverse:false},{passed:false,reverse:true}])("上次结果按完成修订选择：最新通过$passed，倒序$reverse", async ({passed,reverse}) => {
  const request = { runtimeKind: "workbench", connectionId: connection.connectionId, modelId: model.modelId, layer: "protocol", expectedConfigRevision: 4, expectedAuthGeneration: 7 };
  const newest = receipt(request, {validationId:"0".repeat(32),completionRevision:50,updatedAt:1,outcome:passed?"passed":"failed"});
  const old = receipt(request, {validationId:"f".repeat(32),completionRevision:45,updatedAt:9999,outcome:passed?"failed":"passed"});
  records = reverse ? [old, newest] : [newest, old];
  const original = structuredClone(records);
  const form = await validation();
  const item = screen.getByRole("form", {name:"连接验证"}).querySelector('[data-layer="protocol"]')!;
  expect(within(item as HTMLElement).getByText(passed?"上次通过":"上次失败")).toBeTruthy();
  expect(records).toEqual(original);
  expect(validationCalls()).toHaveLength(0);
  fireEvent.click(form.getByRole("button", {name:"关闭"}));
  fireEvent.click(screen.getByRole("button", {name:"验证连接"}));
  const reopened = within(screen.getByRole("form", {name:"连接验证"}));
  await choose(reopened.getByRole("combobox", {name:"验证模型"}), "模型一");
  expect(reopened.getByText(passed?"上次通过":"上次失败")).toBeTruthy();
  expect(validationCalls()).toHaveLength(0);
});
it("空回复失败有独立说明并继续其他独立层，不把完成当通过", async () => {
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (name,args) => name === "model_connection_validate" && args.request.layer === "protocol"
    ? receipt(args.request,{outcome:"failed",reason:"model_validation_empty_response"}) : base(name,args));
  const form = await validation(); await confirmAll(); await finished(8);
  expect(form.getByText("本次运行已结束，但未收到可用回复。")).toBeTruthy();
  expect(screen.getByRole("form", {name:"连接验证"}).querySelector('[data-layer="protocol"]')?.getAttribute("data-outcome")).toBe("failed");
  expect(screen.getByRole("form", {name:"连接验证"}).querySelectorAll('[data-outcome="passed"]')).toHaveLength(7);
});
