import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { emptyAiSnapshot, type AiRuntimeSnapshot } from "./ai-runtime";
import { useAiRuntimeSnapshot } from "./use-ai-runtime-snapshot";
import { conversationArchiveChanged } from "./conversation-archive";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
afterEach(() => { cleanup(); invoke.mockReset(); window.localStorage.clear(); });
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (error: Error) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function useSnapshot(workspaceId: string) { const [, setSelectedConversationId] = useState<string | null>(null); return useAiRuntimeSnapshot({ workspaceId, runtimeRevision: 0, selectedConversationId: null, onMessage: vi.fn(), setSelectedConversationId }); }
test("刷新重叠时调用方等待后续新快照，不提前返回旧数据", async () => {
  const first = deferred<AiRuntimeSnapshot>(); const second = deferred<AiRuntimeSnapshot>();
  invoke.mockImplementationOnce(() => first.promise).mockImplementationOnce(() => second.promise);
  const { result } = renderHook(() => useSnapshot("workspace-a"));
  let completed = false; let pending!: Promise<AiRuntimeSnapshot | null>;
  act(() => { pending = result.current.refresh(false).then(value => { completed = true; return value; }); });
  await act(async () => first.resolve({ ...emptyAiSnapshot, stateRevision: 1 }));
  await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
  expect(completed).toBe(false);
  await act(async () => second.resolve({ ...emptyAiSnapshot, stateRevision: 2 }));
  expect((await pending)?.stateRevision).toBe(2); expect(result.current.snapshot.stateRevision).toBe(2);
});
test("切换工作区后旧请求失败不污染新作用域错误状态", async () => {
  const old = deferred<AiRuntimeSnapshot>(); const current = deferred<AiRuntimeSnapshot>();
  invoke.mockImplementationOnce(() => old.promise).mockImplementationOnce(() => current.promise);
  const { result, rerender } = renderHook(({ workspaceId }) => useSnapshot(workspaceId), { initialProps: { workspaceId: "old" } });
  rerender({ workspaceId: "current" });
  await act(async () => old.reject(new Error("old-workspace-failed")));
  await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
  expect(result.current.snapshotError).toBeNull();
  await act(async () => current.resolve({ ...emptyAiSnapshot, stateRevision: 3 }));
  expect(result.current.snapshotLoaded).toBe(true); expect(result.current.snapshot.stateRevision).toBe(3);
});
test("主对话只取当前对话最新页，加载更早页后可跨分页边界继续浏览", async () => {
  const conversationId = "thread-1";
  const cursor = { sessionId: "session-1", conversationId, beforeMessageId: "latest-1", archived: false };
  const message = (messageId: string, content: string) => ({
    messageId, conversationId, content, role: "user" as const, attachments: [], runId: null, createdAt: 1,
  });
  const newest = message("latest-1", "最新原文");
  const older = message("older-1", "更早原文");
  const other = deferred<unknown>();
  invoke.mockImplementation((command: string, args: { conversationId: string }) => {
    if (command === "ai_runtime_snapshot" && args.conversationId === conversationId) return Promise.resolve({
      ...emptyAiSnapshot, messages: [newest], catalogPreviews: {},
      historyWindow: { sessionId: cursor.sessionId, conversationId, previousCursor: cursor },
    });
    if (command === "ai_conversation_history_page") return Promise.resolve({
      sessionId: cursor.sessionId, conversationId, messages: [older], runs: [], permissionRequests: [], previousCursor: null,
    });
    return other.promise;
  });
  const { result, rerender } = renderHook(({ selection }) => useAiRuntimeSnapshot({
    workspaceId: "workspace", runtimeRevision: 0, selectedConversationId: selection, onMessage: vi.fn(), setSelectedConversationId: vi.fn(),
  }), { initialProps: { selection: conversationId } });
  await waitFor(() => expect(result.current.snapshot.messages).toEqual([newest]));
  expect(invoke).toHaveBeenCalledWith("ai_runtime_snapshot", { refreshNativeCatalog: true, conversationId });
  await act(async () => { expect(await result.current.loadOlder()).toBe(true); });
  expect(invoke).toHaveBeenCalledWith("ai_conversation_history_page", { conversationId, archived: false, cursor });
  expect(result.current.snapshot.messages).toEqual([older, newest]);
  expect(result.current.snapshot.historyWindow?.previousCursor).toBeNull();
  await act(async () => { await result.current.refresh(false); });
  expect(result.current.snapshot.messages).toEqual([older, newest]);
  rerender({ selection: "thread-2" });
  await act(async () => other.resolve({ ...emptyAiSnapshot, historyWindow: { sessionId: cursor.sessionId, conversationId: "thread-2", previousCursor: null } }));
  expect(result.current.snapshot.messages).toEqual([]);
  expect(result.current.snapshot.historyWindow?.conversationId).toBe("thread-2");
});

test("旧归档读取不循环刷新，运行会话重新初始化后可接受新修订", async () => {
  invoke.mockResolvedValue({ ...emptyAiSnapshot, runtimeSessionId: "first", stateRevision: 1 });
  const { result } = renderHook(() => useSnapshot("workspace-a"));
  await waitFor(() => expect(result.current.snapshotLoaded).toBe(true));
  act(() => { window.dispatchEvent(new CustomEvent(conversationArchiveChanged, {
    detail: { stateRevision: 2, conversations: [], changedConversationIds: [], failures: [], archived: true },
  })); });
  await act(async () => { expect(await result.current.refresh(false)).toBeNull(); });
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(result.current.snapshotError).toContain("重试");
  invoke.mockResolvedValue({ ...emptyAiSnapshot, runtimeSessionId: "second", stateRevision: 0 });
  await act(async () => { await result.current.refresh(false); });
  expect(result.current.snapshot.runtimeSessionId).toBe("second");
  expect(result.current.snapshotError).toBeNull();
});
