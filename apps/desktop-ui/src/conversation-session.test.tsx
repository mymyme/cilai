import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { StrictMode } from "react";
import { ConversationSessionProvider, useConversationSessionState, useConversationSessionFlush, flushConversationSessions } from "./conversation-session";
import type { ConversationSessionSnapshot } from "./conversation-session-api";
import type { QueuedFollowUp } from "./ai-workbench";

const mocks = vi.hoisted(() => ({ load: vi.fn(), patch: vi.fn() }));
vi.mock("./conversation-session-api", () => ({ loadConversationSession: mocks.load, patchConversationSession: mocks.patch }));
let stored: ConversationSessionSnapshot;
const item: QueuedFollowUp = { queueId: "queued-1", queuedAt: 1, conversationId: "conv-1", content: "整理材料", attachments: [], applicationContext: null,
  selectedSkillId: "skill-report", modelOverride: "model-1", reasoningEffort: "high", executionTarget: "model", permissionMode: "auto-review", status: "queued" };
function Client() {
  const [drafts, setDrafts] = useConversationSessionState<Record<string, string>>("drafts", {});
  const [queue, setQueue] = useConversationSessionState<QueuedFollowUp[]>("queuedFollowUps", []);
  const flush = useConversationSessionFlush();
  return <>
    <input aria-label="草稿" value={drafts.new ?? ""} onChange={(event) => setDrafts({ new: event.target.value })}/>
    <button onClick={() => setQueue([item])}>排队</button>
    <button onClick={() => { void flush().catch(() => {}); }}>保存</button>
    <output aria-label="队列">{queue.map((entry) => `${entry.queueId}:${entry.status}`).join(",")}</output>
  </>;
}
const view = (visible = true, profile = "a", hostPlatform = "macos") => <StrictMode><ConversationSessionProvider key={profile} hostPlatform={hostPlatform} onMessage={vi.fn()}>{visible ? <Client/> : <p>其他页面</p>}</ConversationSessionProvider></StrictMode>;

function SelectionClient({ connectionId }: { connectionId: string | null }) {
  const [model, setModel] = useConversationSessionState<string | null>("starterModelOverride", null);
  const [connection, setConnection] = useConversationSessionState<string | null>("starterConnectionId", null);
  const [initialized, setInitialized] = useConversationSessionState<string | null>("initializedSelection", null);
  const flush = useConversationSessionFlush();
  return <>
    <output aria-label="模型选择">{JSON.stringify({ model, connection, initialized })}</output>
    <button onClick={() => { setModel("same-model"); setConnection(connectionId); setInitialized("conversation:conv-1"); void flush(); }}>选择连接</button>
  </>;
}
beforeEach(() => {
  vi.clearAllMocks();
  Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
  stored = { sessionId: "session-a", revision: 0, values: {} };
  mocks.load.mockImplementation(async () => structuredClone(stored));
  mocks.patch.mockImplementation(async (sessionId, changes) => {
    if (sessionId !== stored.sessionId) throw new Error("ai_profile_changed");
    for (const change of changes) stored.values[change.key] = change.value;
    stored.revision += 1;
    return structuredClone(stored);
  });
});
afterEach(async () => { await act(async () => cleanup()); vi.useRealTimers(); });

test.each(["provider-b", null])("模型、连接%s和初始化标记共同保存并恢复", async (connectionId) => {
  const ui = () => <ConversationSessionProvider hostPlatform="macos" onMessage={vi.fn()}><SelectionClient connectionId={connectionId}/></ConversationSessionProvider>;
  const page = render(ui());
  fireEvent.click(await screen.findByRole("button", { name: "选择连接" }));
  await waitFor(() => expect(stored.values).toMatchObject({ starterModelOverride: "same-model", starterConnectionId: connectionId, initializedSelection: "conversation:conv-1" }));
  expect(Object.hasOwn(stored.values, "starterConnectionId")).toBe(true);
  expect(mocks.patch.mock.calls[0][1].map((change: { key: string }) => change.key)).toEqual(expect.arrayContaining(["starterModelOverride", "starterConnectionId", "initializedSelection"]));
  page.unmount();
  render(ui());
  expect((await screen.findByLabelText("模型选择")).textContent).toBe(JSON.stringify({ model: "same-model", connection: connectionId, initialized: "conversation:conv-1" }));
});

test("切页保留输入，重新挂载从本地服务恢复草稿和冻结队列", async () => {
  const page = render(view());
  fireEvent.change(await screen.findByLabelText("草稿"), { target: { value: "未发送的内容" } });
  fireEvent.click(screen.getByText("排队"));
  await act(() => flushConversationSessions());
  page.rerender(view(false));
  page.rerender(view());
  expect((screen.getByLabelText("草稿") as HTMLInputElement).value).toBe("未发送的内容");
  page.unmount();
  render(view());
  expect((await screen.findByLabelText("草稿") as HTMLInputElement).value).toBe("未发送的内容");
  expect(stored.values.queuedFollowUps).toEqual([item]);
});

test("保存过程中继续输入，旧回执不能覆盖新草稿", async () => {
  render(view());
  const input = await screen.findByLabelText("草稿");
  let finish!: () => void;
  mocks.patch.mockImplementationOnce(async (_id, changes) => {
    await new Promise<void>((resolve) => { finish = resolve; });
    changes.forEach((change: { key: string; value: unknown }) => { stored.values[change.key] = change.value; });
    return structuredClone({ ...stored, revision: ++stored.revision });
  });
  fireEvent.change(input, { target: { value: "第一版" } });
  fireEvent.click(screen.getByText("保存"));
  await waitFor(() => expect(finish).toBeDefined());
  fireEvent.change(input, { target: { value: "第二版" } });
  await act(async () => finish());
  await waitFor(() => expect(stored.values.drafts).toEqual({ new: "第二版" }));
  expect((input as HTMLInputElement).value).toBe("第二版");
});

test("保存失败保留内容并阻止企业切换前的提交，显式重试后恢复", async () => {
  render(view());
  fireEvent.change(await screen.findByLabelText("草稿"), { target: { value: "保留这份材料" } });
  mocks.patch.mockRejectedValueOnce(new Error("磁盘写入失败"));
  fireEvent.click(screen.getByText("保存"));
  expect((await screen.findByRole("alert")).textContent).toContain("草稿未保存");
  await expect(flushConversationSessions()).rejects.toThrow("草稿尚未保存");
  fireEvent.click(screen.getByText("重试"));
  await waitFor(() => expect(stored.values.drafts).toEqual({ new: "保留这份材料" }));
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
});

test("输入恢复失败时不创建空草稿覆盖原数据", async () => {
  mocks.load.mockRejectedValueOnce(new Error("服务离线")).mockRejectedValueOnce(new Error("服务离线"));
  render(view());
  expect((await screen.findByRole("alert")).textContent).toContain("无法恢复草稿");
  expect(screen.queryByLabelText("草稿")).toBeNull();
  expect(mocks.patch).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("重试"));
  expect(await screen.findByLabelText("草稿")).toBeTruthy();
});

test.each(["macos", "windows"])("%s 保存失败后再次主动提交会重新保存，成功后解除阻断", async hostPlatform => {
  render(view(true, "a", hostPlatform));
  fireEvent.change(await screen.findByLabelText("草稿"), { target: { value: "补充说明" } });
  fireEvent.click(screen.getByText("排队"));
  mocks.patch.mockRejectedValueOnce(new Error("本地服务暂不可用"));
  fireEvent.click(screen.getByText("保存"));
  await screen.findByRole("alert");
  const failedCalls = mocks.patch.mock.calls.length;
  await expect(flushConversationSessions()).rejects.toThrow("草稿尚未保存");
  expect(mocks.patch.mock.calls.length).toBe(failedCalls);
  fireEvent.click(screen.getByText("保存"));
  await waitFor(() => expect(stored.values.queuedFollowUps).toEqual([item]));
  expect(stored.values.drafts).toEqual({ new: "补充说明" });
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
});

test("主动提交再次保存失败时仍保留草稿和队列并报告真实原因", async () => {
  render(view());
  fireEvent.change(await screen.findByLabelText("草稿"), { target: { value: "不能丢失" } });
  fireEvent.click(screen.getByText("排队"));
  mocks.patch.mockRejectedValue(new Error("磁盘空间不足"));
  fireEvent.click(screen.getByText("保存"));
  await screen.findByRole("alert");
  fireEvent.click(screen.getByText("保存"));
  await waitFor(() => expect(mocks.patch.mock.calls.length).toBe(2));
  expect(screen.getByRole("alert").textContent).toContain("磁盘空间不足");
  expect((screen.getByLabelText("草稿") as HTMLInputElement).value).toBe("不能丢失");
  expect(screen.getByLabelText("队列").textContent).toBe("queued-1:queued");
  expect(stored.values.queuedFollowUps).toBeUndefined();
});

test("切换企业后忽略旧读取结果", async () => {
  const resolvers: Array<(value: ConversationSessionSnapshot) => void> = [];
  mocks.load.mockImplementation(() => new Promise((resolve) => resolvers.push(resolve)));
  const page = render(view());
  page.rerender(view(true, "b"));
  await act(async () => { resolvers.at(-1)!({ sessionId: "b", revision: 0, values: { drafts: { new: "企业乙" } } }); });
  await act(async () => { resolvers[0]({ sessionId: "a", revision: 0, values: { drafts: { new: "企业甲" } } }); });
  expect((screen.getByLabelText("草稿") as HTMLInputElement).value).toBe("企业乙");
});

test("宿主领取与界面编辑冲突时采用真实队列，不重放已领取指令", async () => {
  stored.values = { queuedFollowUps: [item] };
  render(view());
  await screen.findByLabelText("草稿");
  stored.values.queuedFollowUps = [{ ...item, status: "sending" }];
  stored.revision += 1;
  mocks.patch.mockRejectedValueOnce(new Error("conversation_session_conflict"));
  fireEvent.change(screen.getByLabelText("草稿"), { target: { value: "草稿" } });
  fireEvent.click(screen.getByText("保存"));
  await screen.findByRole("alert");
  expect(screen.getByLabelText("队列").textContent).toBe("queued-1:sending");
  expect((screen.getByLabelText("草稿") as HTMLInputElement).value).toBe("草稿");
});

test("未编辑的默认状态不阻止宿主队列同步，也不主动覆盖服务端", async () => {
  render(view());
  await screen.findByLabelText("草稿");
  stored.values.queuedFollowUps = [item]; stored.revision += 1;
  await act(async () => { window.dispatchEvent(new Event("focus")); });
  await waitFor(() => expect(screen.getByLabelText("队列").textContent).toBe("queued-1:queued"));
  expect(mocks.patch).not.toHaveBeenCalled();
});
