import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DesktopUpdateControl, latestClientUpdateSignal, type DesktopUpdateState } from "./desktop-update";
const flushSessions = vi.hoisted(() => vi.fn());
vi.mock("./conversation-session", () => ({ flushConversationSessions: flushSessions }));
beforeEach(() => { flushSessions.mockReset(); flushSessions.mockResolvedValue(undefined); });

const idle: DesktopUpdateState = {
  autoDownload: true,
  supported: true,
  channel: 'stable', selectedChannel: 'stable', currentChannel: 'stable', latestVersion: null,
  phase: "idle",
  currentVersion: "1.0.0",
  availableVersion: null,
  releaseName: null,
  releaseNotes: null,
  percent: null,
  transferred: null,
  total: null,
  bytesPerSecond: null,
  error: null,
  checkedAt: null,
  trigger: null,
};

function installBridge(initial: DesktopUpdateState) {
  let listener: ((state: DesktopUpdateState) => void) | null = null;
  const bridge = {
    getState: vi.fn(async () => initial),
    check: vi.fn(async () => initial),
    checkSelectedChannel: vi.fn(async () => initial),
    selectChannel: vi.fn(async () => initial),
    download: vi.fn(async () => initial),
    pause: vi.fn(async () => ({ ...initial, phase: 'paused' as const })),
    setAutoDownload: vi.fn(async (autoDownload: boolean) => ({ ...initial, autoDownload })),
    install: vi.fn(async () => null),
    onState: vi.fn((handler: (state: DesktopUpdateState) => void) => {
      listener = handler;
      return () => { listener = null; };
    }),
  };
  window.__CILAI_UPDATER__ = bridge;
  return { bridge, push: (state: DesktopUpdateState) => listener?.(state) };
}

afterEach(() => {
  cleanup();
  delete window.__CILAI_UPDATER__;
});

describe("客户端更新入口", () => {
  it("草稿保存完成前不退出应用进行更新", async () => {
    let finish!: () => void;
    flushSessions.mockImplementation(() => new Promise<void>((resolve) => { finish = resolve; }));
    const { bridge } = installBridge({ ...idle, phase: "downloaded", availableVersion: "1.1.0" });
    render(<DesktopUpdateControl platformSignal={0}/>);
    fireEvent.click(await screen.findByLabelText("重启更新词来工作台"));
    fireEvent.click(screen.getByRole("button", { name: "重启更新" }));
    expect(flushSessions).toHaveBeenCalledTimes(1);
    expect(bridge.install).not.toHaveBeenCalled();
    finish();
    await waitFor(() => expect(bridge.install).toHaveBeenCalledTimes(1));
  });

  it("草稿保存失败时保留应用，重试成功后才更新", async () => {
    flushSessions.mockRejectedValueOnce(new Error("草稿保存失败"));
    const { bridge } = installBridge({ ...idle, phase: "downloaded", availableVersion: "1.1.0" });
    render(<DesktopUpdateControl platformSignal={0}/>);
    fireEvent.click(await screen.findByLabelText("重启更新词来工作台"));
    fireEvent.click(screen.getByRole("button", { name: "重启更新" }));
    expect(await screen.findByText(/草稿保存失败/)).toBeTruthy();
    expect(bridge.install).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "重启更新" }));
    await waitFor(() => expect(bridge.install).toHaveBeenCalledTimes(1));
  });
  it("更新源故障仍显示错误并允许重新检查", async () => {
    const { bridge } = installBridge({ ...idle, phase: "error", error: "更新源暂时不可用" });
    render(<DesktopUpdateControl platformSignal={0}/>);
    fireEvent.click(await screen.findByLabelText("词来工作台更新失败"));
    expect(screen.getByText("更新源暂时不可用")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "重新检查" }));
    await waitFor(() => expect(bridge.check).toHaveBeenCalledTimes(1));
  });
  it("没有新版本时不占用标题栏位置", async () => {
    installBridge(idle);
    render(<DesktopUpdateControl platformSignal={0}/>);
    await waitFor(() => expect(window.__CILAI_UPDATER__?.getState).toHaveBeenCalled());
    expect(screen.queryByLabelText("词来工作台有新版本")).toBeNull();
  });

  it("发现版本后显示提示，由员工明确开始下载", async () => {
    const state = { ...idle, phase: "available" as const, availableVersion: "1.1.0", releaseNotes: "修复已知问题" };
    const { bridge } = installBridge(state);
    render(<DesktopUpdateControl platformSignal={0}/>);
    const trigger = await screen.findByLabelText("词来工作台有新版本");
    fireEvent.click(trigger);
    expect(screen.getByText("正式版 1.1.0")).toBeTruthy();
    expect(screen.getByText("修复已知问题")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "下载更新" }));
    await waitFor(() => expect(bridge.download).toHaveBeenCalledTimes(1));
  });

  it("下载中显示进度，完成后只提供明确重启操作", async () => {
    const available = { ...idle, phase: "available" as const, availableVersion: "1.1.0" };
    const { bridge, push } = installBridge(available);
    render(<DesktopUpdateControl platformSignal={0}/>);
    fireEvent.click(await screen.findByLabelText("词来工作台有新版本"));
    push({ ...available, phase: "downloading", percent: 42, transferred: 42, total: 100 });
    expect(await screen.findByText("42%")).toBeTruthy();
    expect((screen.getByRole("button", { name: "正在下载 42%" }) as HTMLButtonElement).disabled).toBe(true);
    push({ ...available, phase: "downloaded", percent: 100 });
    fireEvent.click(await screen.findByRole("button", { name: "重启更新" }));
    await waitFor(() => expect(bridge.install).toHaveBeenCalledTimes(1));
  });

  it("平台发布事件只触发重新检查", async () => {
    const { bridge } = installBridge(idle);
    const view = render(<DesktopUpdateControl platformSignal={0}/>);
    await waitFor(() => expect(bridge.getState).toHaveBeenCalledTimes(1));
    view.rerender(<DesktopUpdateControl platformSignal={18}/>);
    await waitFor(() => expect(bridge.check).toHaveBeenCalledTimes(1));
    view.rerender(<DesktopUpdateControl platformSignal={18}/>);
    expect(bridge.check).toHaveBeenCalledTimes(1);
  });

  it("只识别平台发布事件的最新序号", () => {
    expect(latestClientUpdateSignal([
      { sequence: 2, eventId: "a", eventType: "file.state_changed", workspaceId: null, objectType: null, objectId: null, payloadJson: "{}", correlationId: null, occurredAt: 1 },
      { sequence: 5, eventId: "b", eventType: "workbench.client_update_published", workspaceId: null, objectType: null, objectId: null, payloadJson: "{}", correlationId: null, occurredAt: 2 },
      { sequence: 7, eventId: "c", eventType: "workbench.client_update_published", workspaceId: null, objectType: null, objectId: null, payloadJson: "{}", correlationId: null, occurredAt: 3 },
    ], 2)).toBe(7);
  });

  it("忽略本次启动前的历史发布事件", () => {
    expect(latestClientUpdateSignal([
      { sequence: 7, eventId: "c", eventType: "workbench.client_update_published", workspaceId: null, objectType: null, objectId: null, payloadJson: "{}", correlationId: null, occurredAt: 3 },
    ], 4)).toBe(0);
  });
});
