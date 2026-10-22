import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { eventSignalNotice, isEventSignalTimeout, useWorkbenchEventSignals } from "./use-workbench-event-signals";

const rpc = vi.hoisted(() => vi.fn());
vi.mock("./app-server-rpc", () => ({ rpc }));

afterEach(() => {
  cleanup();
  rpc.mockReset();
  vi.restoreAllMocks();
});

describe("工作台事件信号回放错误", () => {
  it("只把当前事件回放请求超时识别为可重试延迟", () => {
    expect(isEventSignalTimeout("App Server request timed out: event/replay")).toBe(true);
    expect(isEventSignalTimeout("App Server request timed out: workspace/list")).toBe(false);
    expect(isEventSignalTimeout("event_replay_failed")).toBe(false);
  });

  it("超时和其他回放异常都提示自动重试，不再对已删除的记录页命名", () => {
    expect(eventSignalNotice("App Server request timed out: event/replay"))
      .toBe("工作台状态同步延迟，正在重试。");
    expect(eventSignalNotice("event_replay_failed"))
      .toBe("工作台状态暂时无法同步，正在重试。");
  });

  it("追平分页事件后只返回业务刷新和客户端更新信号", async () => {
    vi.spyOn(Date, "now").mockReturnValue(100);
    rpc
      .mockResolvedValueOnce({
        events: [{ sequence: 1, eventId: "file-1", eventType: "file.state_changed", workspaceId: "ws-1", objectType: "local_file", objectId: "f-1", payloadJson: "{}", correlationId: null, occurredAt: 99 }],
        nextSequence: 1,
        hasMore: true,
      })
      .mockResolvedValueOnce({
        events: [{ sequence: 2, eventId: "update-1", eventType: "workbench.client_update_published", workspaceId: null, objectType: null, objectId: null, payloadJson: "{}", correlationId: null, occurredAt: 101 }],
        nextSequence: 2,
        hasMore: false,
      });
    const onSelectedWorkspaceChanged = vi.fn();
    const { result } = renderHook(() => useWorkbenchEventSignals({
      active: true,
      selectedWorkspaceId: "ws-1",
      serverInstanceId: "server-1",
      onSelectedWorkspaceChanged,
      onMessage: vi.fn(),
    }));

    await waitFor(() => expect(result.current).toEqual({ nextSequence: 2, clientUpdateSignal: 2 }));
    expect(onSelectedWorkspaceChanged).toHaveBeenCalledOnce();
    expect(rpc).toHaveBeenNthCalledWith(1, "event/replay", { afterSequence: null, limit: 100 });
    expect(rpc).toHaveBeenNthCalledWith(2, "event/replay", { afterSequence: 1, limit: 100 });
  });
});
