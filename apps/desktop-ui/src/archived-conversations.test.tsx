import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emptyAiQuerySnapshot, type AiConversation, type AiRuntimeQuerySnapshot } from "./ai-runtime";
import { ArchivedConversationsPage } from "./archived-conversations";
import { conversationArchiveChanged } from "./conversation-archive";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({
  invoke,
}));
afterEach(cleanup);
beforeEach(() => { invoke.mockReset(); });

function conversation(id: string, projectId: string | null, overrides: Partial<AiConversation> = {}): AiConversation {
  return {
    conversationId: id, title: `会话 ${id}`, conversationType: "direct", mode: "discuss", agentId: null,
    context: { workspaceId: "ws-1", workspaceName: "工作区", workspaceRoot: "/tmp/ws-1", projectId, projectName: projectId ? "同名项目" : null, phaseId: null, phaseName: null, taskId: null, taskName: null, deliverableId: null, deliverableName: null, localFileId: null, relativePath: null, fileRevision: null },
    workClassification: null, archived: true, createdAt: 1, updatedAt: 2, ...overrides,
  };
}

function setup(conversations: AiConversation[], messages: AiRuntimeQuerySnapshot["messages"] = []) {
  const state: AiRuntimeQuerySnapshot = { ...emptyAiQuerySnapshot("archive"), conversations, messages };
  invoke.mockImplementation(async (command: string, args?: { request: { scope?: string; conversationId?: string; conversationIds?: string[]; archived?: boolean } }) => {
    if (command === "ai_runtime_query") return args?.request.scope === "conversation"
      ? { ...state, scope: "conversation", conversations: state.conversations.filter((item) => item.conversationId === args.request.conversationId), messages: state.messages.filter((message) => message.conversationId === args.request.conversationId) }
      : { ...state, messages: [] };
    if (command === "ai_attachment_read") return new Uint8Array([137, 80, 78, 71]).buffer;
    if (command === "ai_conversation_set_archived") {
      const item = state.conversations.find((candidate) => candidate.conversationId === args?.request.conversationIds?.[0])!;
      return { stateRevision: 1, conversations: [{ ...item, archived: args!.request.archived, updatedAt: 3 }], changedConversationIds: [item.conversationId], failures: [] };
    }
    if (command === "ai_conversation_delete") {
      const ids = new Set(args?.request.conversationIds ?? []);
      const runIds = new Set(state.runs.filter((run) => ids.has(run.conversationId)).map((run) => run.runId));
      state.conversations = state.conversations.filter((item) => !ids.has(item.conversationId));
      state.messages = state.messages.filter((item) => !ids.has(item.conversationId));
      state.runs = state.runs.filter((item) => !ids.has(item.conversationId));
      state.permissionRequests = state.permissionRequests.filter((item) => !runIds.has(item.runId));
      return { deletedConversationIds: [...ids], deletedCount: ids.size, cleanupWarning: false };
    }
    throw new Error(command);
  });
  const onMessage = vi.fn();
  render(<ArchivedConversationsPage projects={[]} onMessage={onMessage}/>);
  return { onMessage, state };
}

describe("已归档会话", () => {
  it("查看详情加载期间撤销归档，迟到的旧快照不能重新显示该详情", async () => {
    const item = conversation("a", null);
    let complete!: (snapshot: AiRuntimeQuerySnapshot) => void;
    invoke.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
    invoke.mockResolvedValue({ ...emptyAiQuerySnapshot("archive"), stateRevision: 3, conversations: [{ ...item, archived: false }] });
    render(<ArchivedConversationsPage projects={[]} onMessage={vi.fn()} openIntent={{ conversationId: "a", token: 1 }}/>);
    await act(async () => {
      window.dispatchEvent(new CustomEvent(conversationArchiveChanged, { detail: {
        archived: false, stateRevision: 3, conversations: [{ ...item, archived: false }], changedConversationIds: ["a"], failures: [],
      } }));
      complete({ ...emptyAiQuerySnapshot("archive"), stateRevision: 2, conversations: [item] });
    });
    await screen.findByText("暂无已归档会话");
    expect(screen.queryByRole("heading", { name: "会话 a" })).toBeNull();
    expect(screen.queryByText("该对话已不在归档中。")).toBeNull();
    expect(invoke).toHaveBeenCalledTimes(3);
  });

  it("跨项目读取，只显示归档会话；同名项目不合并，无归属记录不丢失", async () => {
    setup([conversation("a", "p1"), conversation("b", "p2"), conversation("c", null), conversation("d", "p1", { archived: false })]);
    await screen.findByText("3 条会话");
    expect(screen.getAllByRole("region", { name: "同名项目的归档会话" })).toHaveLength(2);
    expect(screen.getByRole("region", { name: "未分类的归档会话" })).toBeTruthy();
    expect(screen.queryByText("会话 d")).toBeNull();
    expect(invoke).toHaveBeenCalledWith("ai_runtime_query", { request: { scope: "archive" } });
  });

  it("搜索和项目筛选取交集，且同名项目可独立选择", async () => {
    setup([conversation("a", "p1"), conversation("b", "p2"), conversation("c", null)]);
    await screen.findByText("会话 a");
    fireEvent.click(screen.getByRole("combobox", { name: "按项目筛选归档会话" }));
    fireEvent.click(screen.getAllByRole("option", { name: "同名项目" })[0]);
    expect(screen.getByText("会话 a")).toBeTruthy();
    expect(screen.queryByText("会话 b")).toBeNull();
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "会话 b" } });
    expect(screen.getByText("未找到匹配的会话")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "清除搜索已归档会话" }));
    expect(screen.getByText("会话 a")).toBeTruthy();
  });

  it("分组和组内按更新时间倒序排列并支持折叠", async () => {
    setup([conversation("old", "p1", { updatedAt: 1 }), conversation("middle", "p2", { updatedAt: 3 }), conversation("new", "p1", { updatedAt: 5 })]);
    await screen.findByText("会话 new");
    const groups = screen.getAllByRole("region", { name: "同名项目的归档会话" });
    expect(within(groups[0]).getAllByRole("listitem").map((item) => item.textContent?.split("更新于")[0])).toEqual(["会话 new", "会话 old"]);
    fireEvent.click(within(groups[0]).getByRole("button", { expanded: true }));
    expect(screen.queryByText("会话 new")).toBeNull();
    expect(screen.getByText("会话 middle")).toBeTruthy();
  });

  it("查看保留归档状态和搜索条件，正文按时间排列且没有发送入口", async () => {
    const { state } = setup([conversation("a", "p1")], [
      { messageId: "m2", conversationId: "a", role: "assistant", content: "**答复正文**", attachments: [], runId: null, createdAt: 5 },
      { messageId: "m1", conversationId: "a", role: "user", content: "问题正文", attachments: [], runId: null, createdAt: 1 },
    ]);
    await screen.findByText("会话 a");
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "会话 a" } });
    fireEvent.click(screen.getByRole("button", { name: /^会话 a 更新于/ }));
    expect((await screen.findAllByRole("article")).map((item) => item.textContent)).toEqual([expect.stringContaining("问题正文"), expect.stringContaining("答复正文")]);
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(invoke.mock.calls.filter(([command]) => command !== "ai_runtime_query")).toHaveLength(0);
    expect(state.conversations[0].archived).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "返回归档清单" }));
    expect((screen.getByRole("searchbox") as HTMLInputElement).value).toBe("会话 a");
  });

  it("只读详情中的附件继续提供在线预览", async () => {
    Object.defineProperty(URL, "createObjectURL", { configurable: true, value: vi.fn(() => "blob:archive-attachment") });
    Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
    setup([conversation("a", "p1")], [{
      messageId: "m1", conversationId: "a", role: "user", content: "附件内容", runId: null, createdAt: 1,
      attachments: [{ attachmentId: "attachment-1", fileName: "evidence.png", mimeType: "image/png", mediaType: "image", sizeBytes: 4, localPath: "/tmp/evidence.png" }],
    }]);
    await screen.findByText("会话 a");
    fireEvent.click(screen.getByRole("button", { name: /^会话 a 更新于/ }));
    const preview = await screen.findByRole("button", { name: "查看图片 evidence.png" });
    fireEvent.click(preview);
    expect(screen.getByRole("dialog", { name: "查看图片 evidence.png" })).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith("ai_attachment_read", { attachmentId: "attachment-1" });
  });

  it("只取消目标会话归档，不修改原项目、消息和运行信息，刷新数量", async () => {
    const { onMessage } = setup([conversation("a", "p1"), conversation("b", "p2")]);
    await screen.findByText("会话 a");
    fireEvent.click(screen.getByRole("button", { name: "取消归档：会话 a" }));
    await waitFor(() => expect(screen.queryByText("会话 a")).toBeNull());
    expect(screen.getByText("会话 b")).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith("ai_conversation_set_archived", { request: { conversationIds: ["a"], archived: false } });
    expect(onMessage).toHaveBeenCalledWith(expect.stringContaining("已取消归档"));
  });

  it("恢复失败保留记录和详情，恢复中阻止重复提交", async () => {
    const { onMessage } = setup([conversation("a", "p1")]);
    await screen.findByText("会话 a");
    fireEvent.click(screen.getByRole("button", { name: /^会话 a 更新于/ }));
    let reject!: (reason: Error) => void;
    invoke.mockImplementationOnce(() => new Promise((_resolve, rejectPromise) => { reject = rejectPromise; }));
    const button = screen.getByRole("button", { name: "取消归档：会话 a" });
    fireEvent.click(button);
    fireEvent.click(button);
    expect((button as HTMLButtonElement).disabled).toBe(true);
    expect(invoke.mock.calls.filter(([command]) => command === "ai_conversation_set_archived")).toHaveLength(1);
    await act(async () => reject(new Error("读取失败")));
    expect(onMessage).toHaveBeenCalledWith(expect.stringContaining("取消归档失败"), "error");
    expect(screen.getByRole("heading", { name: "会话 a" })).toBeTruthy();
    expect((button as HTMLButtonElement).disabled).toBe(false);
  });

  it("恢复所选项目最后一条后清除失效筛选并回到清单", async () => {
    setup([conversation("a", "p1")]);
    await screen.findByText("会话 a");
    fireEvent.click(screen.getByRole("combobox", { name: "按项目筛选归档会话" }));
    fireEvent.click(screen.getByRole("option", { name: "同名项目" }));
    fireEvent.click(screen.getByRole("button", { name: /^会话 a 更新于/ }));
    fireEvent.click(screen.getByRole("button", { name: "取消归档：会话 a" }));
    await screen.findByText("暂无已归档会话");
    expect(screen.getByRole("combobox").textContent).toContain("所有项目");
    expect(screen.getByText("0 条会话")).toBeTruthy();
  });

  it("逐条彻底删除先确认，取消不提交，确认后立即移除会话", async () => {
    const { onMessage } = setup([conversation("a", "p1"), conversation("b", "p2")]);
    await screen.findByText("2 条会话");
    fireEvent.click(screen.getByRole("button", { name: "彻底删除：会话 a" }));
    const firstDialog = screen.getByRole("dialog", { name: "彻底删除这个会话？" });
    expect(within(firstDialog).getByText("该操作不可恢复。")).toBeTruthy();
    fireEvent.click(within(firstDialog).getByRole("button", { name: "取消" }));
    expect(invoke.mock.calls.filter(([command]) => command === "ai_conversation_delete")).toHaveLength(0);

    fireEvent.click(screen.getByRole("button", { name: "彻底删除：会话 a" }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "彻底删除这个会话？" })).getByRole("button", { name: "彻底删除" }));
    await waitFor(() => expect(screen.queryByText("会话 a")).toBeNull());
    expect(screen.getByText("会话 b")).toBeTruthy();
    expect(invoke).toHaveBeenCalledWith("ai_conversation_delete", { request: { conversationIds: ["a"] } });
    expect(onMessage).toHaveBeenCalledWith("已彻底删除 1 条会话。");
  });

  it("详情页也可彻底删除并在成功后返回清单", async () => {
    setup([conversation("a", "p1")]);
    await screen.findByText("会话 a");
    fireEvent.click(screen.getByRole("button", { name: /^会话 a 更新于/ }));
    fireEvent.click(screen.getByRole("button", { name: "彻底删除：会话 a" }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "彻底删除这个会话？" })).getByRole("button", { name: "彻底删除" }));
    await screen.findByText("暂无已归档会话");
    expect(screen.getByText("0 条会话")).toBeTruthy();
  });

  it("批量选择支持逐条勾选，并只用一次请求删除所选会话", async () => {
    setup([conversation("a", "p1"), conversation("b", "p1"), conversation("c", "p2")]);
    await screen.findByText("3 条会话");
    fireEvent.click(screen.getByRole("button", { name: "批量选择" }));
    expect((screen.getByRole("button", { name: "批量删除" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("checkbox", { name: "选择会话：会话 a" }));
    fireEvent.click(screen.getByRole("button", { name: /^会话 c 更新于/ }));
    expect(screen.getByText("已选 2 条")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "批量删除" }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "彻底删除选中的 2 条会话？" })).getByRole("button", { name: "彻底删除" }));
    await waitFor(() => expect(screen.queryByText("会话 a")).toBeNull());
    expect(screen.queryByText("会话 c")).toBeNull();
    expect(screen.getByText("会话 b")).toBeTruthy();
    expect(invoke.mock.calls.filter(([command]) => command === "ai_conversation_delete")).toEqual([
      ["ai_conversation_delete", { request: { conversationIds: ["a", "c"] } }],
    ]);
  });

  it("全选只选择当前搜索与项目筛选结果，切换筛选会清空选择", async () => {
    setup([conversation("a", "p1"), conversation("b", "p1"), conversation("c", "p2")]);
    await screen.findByText("会话 a");
    fireEvent.click(screen.getByRole("button", { name: "批量选择" }));
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "会话 a" } });
    fireEvent.click(screen.getByRole("button", { name: "全选" }));
    expect(screen.getByText("已选 1 条")).toBeTruthy();
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "" } });
    expect(screen.getByText("已选 0 条")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "全选" }));
    expect(screen.getByText("已选 3 条")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "完成" }));
    expect(screen.queryByRole("checkbox")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "批量选择" }));
    expect(screen.getByText("已选 0 条")).toBeTruthy();
  });

  it("批量删除失败时保留真实记录并重新读取状态", async () => {
    const { onMessage } = setup([conversation("a", "p1"), conversation("b", "p1")]);
    await screen.findByText("会话 a");
    fireEvent.click(screen.getByRole("button", { name: "批量选择" }));
    fireEvent.click(screen.getByRole("button", { name: "全选" }));
    fireEvent.click(screen.getByRole("button", { name: "批量删除" }));
    invoke.mockRejectedValueOnce(new Error("conversation_delete_automation_bound:每日汇总"));
    fireEvent.click(within(screen.getByRole("dialog", { name: "彻底删除选中的 2 条会话？" })).getByRole("button", { name: "彻底删除" }));
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith("删除失败：该会话仍被自动化“每日汇总”使用，请先调整自动化。", "error"));
    expect(await screen.findByText("会话 a")).toBeTruthy();
    expect(screen.getByText("会话 b")).toBeTruthy();
  });

  it("加载失败显示错误和重试，不冒充没有归档", async () => {
    invoke.mockRejectedValueOnce(new Error("读取失败")).mockResolvedValueOnce(emptyAiQuerySnapshot("archive"));
    render(<ArchivedConversationsPage projects={[]} onMessage={vi.fn()}/>);
    await screen.findByText("无法读取已归档会话");
    expect(screen.queryByText("暂无已归档会话")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "重新加载" }));
    await screen.findByText("暂无已归档会话");
  });
});
