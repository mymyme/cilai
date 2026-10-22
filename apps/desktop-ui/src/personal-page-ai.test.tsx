import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { PersonalWebApplication } from "./personal-web-application";
import { PersonalPageAiConfirmation } from "./personal-page-ai";

const host = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("./host", () => host);
const props = { projectId: "personal1", packageDigest: `sha256:${"a".repeat(64)}`, html: "<h1>页面</h1>", title: "个人页面", active: true };
const approval = { confirmationId: "confirm1", applicationName: "个人页面", toolName: "setTitle", toolTitle: "更新标题", description: "更新当前页面标题", riskLevel: "medium", arguments: { title: "模型标题" }, executionStatus: "awaiting_confirmation", confirmationView: { reviewMode: "personalPageArguments", inputSchema: { type: "object", properties: { title: { type: "string" } }, required: ["title"], additionalProperties: false } } };
const run = () => ({ runId: "run1", status: "waiting_confirmation", writeRequest: { approval, validUntil: Date.now() + 60_000 } });
beforeEach(() => {
  host.invoke.mockReset();
  host.invoke.mockImplementation(async (command, args) => {
    if (command === "personal_document_create") return "app://personal-capability/page1";
    if (command === "personal_page_ai_dispatch") return args.request.method.startsWith("model.run.") ? run() : { modelSessionId: "session1" };
    return { runId: "run1", status: "completed", writeRequest: null };
  });
});
afterEach(cleanup);
async function open(onOpenModels?: () => void) {
  render(<PersonalWebApplication {...props} onOpenModels={onOpenModels}/>);
  const frame = await screen.findByTitle(props.title) as HTMLIFrameElement;
  await act(async () => {});
  const source = host.invoke.mock.calls.find(([command]) => command === "personal_document_create")![1].html;
  const channel = source.match(/const channel="([^"]+)"/)![1];
  const send = (method: string, params: unknown = {}, source: Window | null = frame.contentWindow) => act(() => window.dispatchEvent(new MessageEvent("message", { source, data: { type: "cilai-personal-ai-request", channel, id: crypto.randomUUID(), method, params } })));
  return { frame, channel, send };
}

test("页面不能自批或伪造来源，模型请求使用其登记URL", async () => {
  const { send } = await open();
  send("model.session.open", {}, window);
  send("model.write.resolve", { decision: "approved" });
  expect(host.invoke).toHaveBeenCalledTimes(1);
  send("model.session.open");
  await waitFor(() => expect(host.invoke).toHaveBeenCalledWith("personal_page_ai_dispatch", { url: "app://personal-capability/page1", request: { method: "model.session.open", requestId: expect.any(String), params: {} } }));
  expect(host.invoke.mock.calls.some(([command]) => command === "personal_page_ai_resolve")).toBe(false);
});

test("工作台独立显示确认，完整编辑参数后才批准", async () => {
  const { send } = await open();
  send("model.run.create", { modelSessionId: "session1", modelId: "model1", prompt: "更新标题", clientMessageId: "message1" });
  expect(await screen.findByRole("dialog", { name: "确认页面操作" })).toBeTruthy();
  expect(host.invoke.mock.calls.some(([command]) => command === "personal_page_ai_resolve")).toBe(false);
  fireEvent.change(screen.getByRole("textbox", { name: "页面操作参数" }), { target: { value: '{"title":"用户标题"}' } });
  fireEvent.click(screen.getByRole("button", { name: "批准" }));
  await waitFor(() => expect(host.invoke).toHaveBeenCalledWith("personal_page_ai_resolve", { url: "app://personal-capability/page1", runId: "run1", confirmationId: "confirm1", decision: "approved", arguments: { title: "用户标题" } }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
});

test("拒绝不提交修改参数，未知结果禁止重复批准", async () => {
  const resolve = vi.fn().mockRejectedValue(new Error("personal_page_action_outcome_unconfirmed"));
  render(<PersonalPageAiConfirmation pending={run()} resolve={resolve}/>);
  fireEvent.click(screen.getByRole("button", { name: "拒绝" }));
  await waitFor(() => expect(resolve).toHaveBeenCalledExactlyOnceWith("rejected", undefined));
  await screen.findByText("结果未确认，请核对回执。");
  expect((screen.getByRole("button", { name: "批准" }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByRole("button", { name: "拒绝" }) as HTMLButtonElement).disabled).toBe(true);
});

test("本地JSON及服务端约束拒绝后保留参数，未执行的校验可修改", async () => {
  const resolve = vi.fn().mockRejectedValueOnce(new Error("application_write_arguments_invalid")).mockResolvedValue(undefined);
  render(<PersonalPageAiConfirmation pending={run()} resolve={resolve}/>);
  const editor = screen.getByRole("textbox", { name: "页面操作参数" });
  fireEvent.change(editor, { target: { value: "{" } }); fireEvent.click(screen.getByRole("button", { name: "批准" }));
  expect(await screen.findByText("参数必须是有效 JSON")).toBeTruthy(); expect(resolve).not.toHaveBeenCalled();
  fireEvent.change(editor, { target: { value: '{"title":1}' } }); fireEvent.click(screen.getByRole("button", { name: "批准" }));
  await screen.findByText("Error: application_write_arguments_invalid");
  expect((editor as HTMLTextAreaElement).value).toBe('{"title":1}');
  fireEvent.change(editor, { target: { value: '{"title":"正确"}' } }); fireEvent.click(screen.getByRole("button", { name: "批准" }));
  await waitFor(() => expect(resolve).toHaveBeenLastCalledWith("approved", { title: "正确" }));
});

test("非对象JSON在本地拒绝并保留可编辑输入", async () => {
  const resolve = vi.fn(); render(<PersonalPageAiConfirmation pending={run()} resolve={resolve}/>);
  const editor = screen.getByRole("textbox", { name: "页面操作参数" });
  for (const value of ["[]", "null", "42", '"text"']) {
    fireEvent.change(editor, { target: { value } }); fireEvent.click(screen.getByRole("button", { name: "批准" }));
    await screen.findByText("参数必须是 JSON 对象");
    expect((editor as HTMLTextAreaElement).disabled).toBe(false); expect((editor as HTMLTextAreaElement).value).toBe(value);
  }
  expect(resolve).not.toHaveBeenCalled();
});

test("隐藏确认不响应全局Escape，重新激活后才处理", async () => {
  const resolve = vi.fn().mockResolvedValue(undefined);
  const view = render(<PersonalPageAiConfirmation pending={run()} resolve={resolve} active={false}/>);
  fireEvent.keyDown(window, { key: "Escape" }); expect(resolve).not.toHaveBeenCalled();
  view.rerender(<PersonalPageAiConfirmation pending={run()} resolve={resolve} active/>);
  fireEvent.keyDown(window, { key: "Escape" }); await waitFor(() => expect(resolve).toHaveBeenCalledExactlyOnceWith("rejected", undefined));
});

test("只读核对临时失败后继续读取，模型创建不会被重发", async () => {
  let reads = 0;
  host.invoke.mockImplementation(async (command, args) => {
    if (command === "personal_document_create") return "app://personal-capability/page1";
    if (command === "personal_page_ai_dispatch") {
      if (args.request.method === "model.run.create") return { runId: "run1", status: "running" };
      if (args.request.method === "model.run.get") { reads++; if (reads === 1) throw new Error("temporary_read_error"); return run(); }
    }
  });
  const { send } = await open(); send("model.run.create", { prompt: "保留原请求" });
  await screen.findByText("temporary_read_error");
  await screen.findByRole("dialog", { name: "确认页面操作" }, { timeout: 3500 });
  expect(reads).toBeGreaterThanOrEqual(2);
  expect(host.invoke.mock.calls.filter(([command, args]) => command === "personal_page_ai_dispatch" && args.request.method === "model.run.create")).toHaveLength(1);
});

test("连接错误回给原页面，输入和clientMessageId不被替换或自动重发", async () => {
  const onOpenModels = vi.fn();
  const { frame, send } = await open(onOpenModels);
  const post = vi.spyOn(frame.contentWindow!, "postMessage");
  host.invoke.mockImplementation(async command => { if (command === "personal_page_ai_dispatch") throw new Error("model_purpose_binding_required"); });
  const params = { modelSessionId: "session1", modelId: "model1", prompt: "需要保留的输入", clientMessageId: "message1" };
  send("model.run.create", params);
  await waitFor(() => expect(post).toHaveBeenCalledWith(expect.objectContaining({ type: "cilai-personal-ai-result", error: "Error: model_purpose_binding_required" }), "*"));
  expect(host.invoke.mock.calls.filter(([command]) => command === "personal_page_ai_dispatch")).toHaveLength(1);
  expect(host.invoke).toHaveBeenLastCalledWith("personal_page_ai_dispatch", expect.objectContaining({ request: expect.objectContaining({ params }) }));
  expect(await screen.findByText("请先配置此模型用途。")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "设置模型" })); expect(onOpenModels).toHaveBeenCalledTimes(1);
});
