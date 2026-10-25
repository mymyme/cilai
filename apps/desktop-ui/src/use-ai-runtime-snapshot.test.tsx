import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { emptyAiSnapshot, type AiRuntimeSnapshot } from "./ai-runtime";
import { useAiRuntimeSnapshot } from "./use-ai-runtime-snapshot";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
afterEach(() => { cleanup(); invoke.mockReset(); window.localStorage.clear(); });
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (error: Error) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function useSnapshot(workspaceId: string) { const [, setSelectedConversationId] = useState<string | null>(null); return useAiRuntimeSnapshot({ workspaceId, runtimeRevision: 0, onMessage: vi.fn(), setSelectedConversationId }); }
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
