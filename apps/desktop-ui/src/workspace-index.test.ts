import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => vi.fn());
vi.mock("./app-server-rpc", () => ({ rpc }));

import { ACTIVE_INDEX_POLL_MS, indexFailureLabel, indexProgressLabel, useWorkspaceIndexStatus } from "./workspace-index";
import type { WorkspaceIndexStatusResult } from "./generated/protocol";

afterEach(() => { cleanup(); rpc.mockReset(); vi.useRealTimers(); });

const status = (phase: string, extra: Partial<WorkspaceIndexStatusResult> = {}): WorkspaceIndexStatusResult => ({
  workspaceId: "ws", phase, entriesExamined: 0, filesHashed: 0, filesToHash: 0, batchesCommitted: 0, batchesTotal: 0, reason: null, ...extra,
});


describe("索引进度文字", () => {
  it("空闲时不显示", () => {
    expect(indexProgressLabel(status("idle"))).toBeNull();
    expect(indexProgressLabel(null)).toBeNull();
  });

  it("各阶段给出可读的进度", () => {
    expect(indexProgressLabel(status("scanning", { entriesExamined: 12345 }))).toBe("正在建立索引 · 已发现 12,345 项");
    expect(indexProgressLabel(status("hashing", { filesHashed: 3, filesToHash: 10 }))).toBe("正在读取文件内容 · 3 / 10");
    expect(indexProgressLabel(status("committing"))).toBe("正在写入索引…");
    expect(indexProgressLabel(status("committing", { batchesCommitted: 2, batchesTotal: 10 }))).toBe("正在写入索引 · 第 3 / 10 批");
  });

  it("失败时说明原因，未知原因保留代码", () => {
    expect(indexProgressLabel(status("failed", { reason: "workspace_scan_incomplete" }))).toBe("部分文件或文件夹无法读取，索引没有更新");
    expect(indexFailureLabel("something_new")).toBe("建立索引失败（something_new）");
  });

  it.each([
    ["workspace_target_unavailable", "索引目标已不存在或无法读取，索引没有更新"],
    ["workspace_path_outside", "索引目标超出工作区，索引没有更新"],
    ["workspace_target_not_directory", "索引目标不是文件夹，索引没有更新"],
    ["workspace_target_ignored", "索引目标位于忽略目录，索引没有更新"],
    ["workspace_target_unsupported", "索引目标类型不受支持，索引没有更新"],
    ["workspace_root_not_directory", "工作区路径不是文件夹，索引没有更新"],
  ])("区分索引失败 %s", (reason, label) => {
    expect(indexProgressLabel(status("failed", { reason }))).toBe(label);
  });
});

describe("跟踪索引进度", () => {
  it("从进行中回到空闲时通知外壳刷新一次", async () => {
    vi.useFakeTimers();
    rpc.mockResolvedValueOnce(status("scanning", { entriesExamined: 5 })).mockResolvedValueOnce(status("idle"));
    const onSettled = vi.fn();
    const { result } = renderHook(() => useWorkspaceIndexStatus("ws", true, onSettled));
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    expect(result.current.status?.phase).toBe("scanning");
    expect(onSettled).not.toHaveBeenCalled();
    await act(async () => { await vi.advanceTimersByTimeAsync(ACTIVE_INDEX_POLL_MS); });
    expect(result.current.status?.phase).toBe("idle");
    expect(onSettled).toHaveBeenCalledTimes(1);
  });

  it("一直空闲时不触发刷新", async () => {
    vi.useFakeTimers();
    rpc.mockResolvedValue(status("idle"));
    const onSettled = vi.fn();
    renderHook(() => useWorkspaceIndexStatus("ws", true, onSettled));
    await act(async () => { await vi.advanceTimersByTimeAsync(20_000); });
    expect(onSettled).not.toHaveBeenCalled();
  });

  it("读不到进度时明确报告，而不是当作空闲", async () => {
    vi.useFakeTimers();
    rpc.mockRejectedValueOnce(new Error("workspace_index_unavailable"));
    const { result } = renderHook(() => useWorkspaceIndexStatus("ws", true, vi.fn()));
    await act(async () => { await vi.advanceTimersByTimeAsync(0); });
    expect(result.current.pollError).toContain("workspace_index_unavailable");
  });

  it("没有选中本地工作区时不轮询", () => {
    renderHook(() => useWorkspaceIndexStatus(null, true, vi.fn()));
    expect(rpc).not.toHaveBeenCalled();
  });
});
