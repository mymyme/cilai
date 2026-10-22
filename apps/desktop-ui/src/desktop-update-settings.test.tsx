import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DesktopUpdateControl, type DesktopUpdateState } from "./desktop-update";
import { SettingsPage } from "./settings";

const flushSessions = vi.hoisted(() => vi.fn());
vi.mock("./conversation-session", () => ({ flushConversationSessions: flushSessions }));
const idle: DesktopUpdateState = {
  autoDownload: true,
  channel: 'stable', selectedChannel: 'stable', currentChannel: 'stable', latestVersion: null,
  supported: true, phase: "idle", currentVersion: "0.2.5", availableVersion: null,
  releaseName: null, releaseNotes: null, percent: null, transferred: null, total: null,
  bytesPerSecond: null, error: null, checkedAt: null, trigger: null,
};
function installBridge(initial = idle) {
  let state = { ...initial };
  const listeners = new Set<(state: DesktopUpdateState) => void>();
  const push = (patch: Partial<DesktopUpdateState>) => {
    state = { ...state, ...patch };
    act(() => listeners.forEach((listener) => listener(state)));
    return state;
  };
  const bridge = {
    getState: vi.fn(async () => state),
    check: vi.fn(async () => push({ phase: "idle", checkedAt: 1_760_000_000_000, trigger: "manual" })),
    checkSelectedChannel: vi.fn(async () => push({ channel: state.selectedChannel, phase: "idle", checkedAt: 1_760_000_000_000, trigger: "settings" })),
    selectChannel: vi.fn(async (channel: DesktopUpdateState['channel']) => push({ channel, selectedChannel: channel, phase: 'idle', checkedAt: null,
      availableVersion: null, latestVersion: null, error: null })),
    download: vi.fn(async () => state), install: vi.fn(async () => null),
    pause: vi.fn(async () => push({ phase: 'paused' })),
    setAutoDownload: vi.fn(async (autoDownload: boolean) => push({ autoDownload })),
    onState: vi.fn((listener: (state: DesktopUpdateState) => void) => {
      listeners.add(listener); return () => { listeners.delete(listener); };
    }),
  };
  window.__CILAI_UPDATER__ = bridge;
  return { bridge, push, listeners };
}
function renderSettings(platform = "windows") {
  return render(<SettingsPage section="updates" onSectionChange={vi.fn()} workspace={null}
    hostPlatform={platform} onMessage={vi.fn()} projects={[]}/>);
}
beforeEach(() => { flushSessions.mockReset(); flushSessions.mockResolvedValue(undefined); });
afterEach(() => { cleanup(); delete window.__CILAI_UPDATER__; });

describe("设置中的手动更新", () => {
  it('显示每 4 小时检查和自动下载开关，可在下载期间暂停再继续', async () => {
    const { bridge, push } = installBridge();
    renderSettings();
    const toggle = await screen.findByRole('switch', { name: '自动下载更新' });
    expect(screen.getByText(/运行期间每 4 小时检查一次/)).toBeTruthy();
    expect(toggle.getAttribute('aria-checked')).toBe('true');
    fireEvent.click(toggle);
    await waitFor(() => expect(bridge.setAutoDownload).toHaveBeenCalledWith(false));
    expect(toggle.getAttribute('aria-checked')).toBe('false');
    push({ phase: 'downloading', availableVersion: '0.2.6', percent: 35 });
    fireEvent.click(screen.getByRole('button', { name: '暂停下载' }));
    const resume = await screen.findByRole('button', { name: '继续下载' });
    expect(bridge.pause).toHaveBeenCalledTimes(1);
    fireEvent.click(resume);
    await waitFor(() => expect(bridge.download).toHaveBeenCalledTimes(1));
    expect(bridge.install).not.toHaveBeenCalled();
  });
  it('保留手动通道选择，但自动检查结果明确显示实际检查的安装通道', async () => {
    const { bridge } = installBridge({ ...idle, channel: 'testing', currentChannel: 'testing', selectedChannel: 'stable', checkedAt: 1, latestVersion: '0.2.19' });
    renderSettings();
    await screen.findByText('测试版暂无可用更新');
    expect(screen.getByRole('combobox').textContent).toBe('正式版');
    expect(screen.getByText('测试版最新版本：0.2.19')).toBeTruthy();
    expect(bridge.check).not.toHaveBeenCalled();
    expect(bridge.checkSelectedChannel).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', {name:'检查更新'}));
    await screen.findByText('正式版暂无可用更新');
    expect(bridge.checkSelectedChannel).toHaveBeenCalledTimes(1);
    expect(bridge.check).not.toHaveBeenCalled();
  });
  it('切换测试或正式通道后重新检查，旧候选不能继续安装', async () => {
    const { bridge } = installBridge({ ...idle, phase: 'downloaded', availableVersion: '0.2.6' });
    renderSettings(); await screen.findByRole('button', { name: '重启更新' });
    fireEvent.click(screen.getByRole('combobox', { name: '更新通道' }));
    fireEvent.click(screen.getByRole('option', { name: '测试版' }));
    await waitFor(() => expect(bridge.selectChannel).toHaveBeenCalledWith('testing'));
    expect(screen.queryByRole('button', { name: '重启更新' })).toBeNull();
    expect(screen.getByRole('combobox').textContent).toBe('测试版');
    expect(bridge.install).not.toHaveBeenCalled();
    expect(screen.getByText('当前安装：正式版')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '检查更新' }));
    await screen.findByText(/(?:正式版|测试版)暂无可用更新/);
    expect(bridge.checkSelectedChannel).toHaveBeenCalledTimes(1);
  });
  it.each(['0.2.8', '0.2.9'])('较高或同版本测试版可切换到最新正式版 %s', async version => {
    const { bridge } = installBridge({ ...idle, phase: 'available', currentVersion: '0.2.9', currentChannel: 'testing', latestVersion: version, availableVersion: version, checkedAt: 1 });
    renderSettings(); await screen.findByText(`可更新至 ${version}（正式版）`);
    expect(screen.getByText('当前安装：测试版')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '下载更新' }));
    await waitFor(() => expect(bridge.download).toHaveBeenCalledTimes(1));
  });
  it.each(["windows", "macos"])("%s 设置可手动检查，检查前不宣称已是最新版本", async (platform) => {
    const { bridge } = installBridge();
    renderSettings(platform);
    await screen.findByText("当前版本：0.2.5");
    expect(screen.getByRole("button", { name: "软件更新" }).getAttribute("aria-current")).toBe("page");
    expect(screen.queryByText(/(?:正式版|测试版)暂无可用更新/)).toBeNull();
    expect(bridge.checkSelectedChannel).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "检查更新" }));
    await screen.findByText(/(?:正式版|测试版)暂无可用更新/);
    expect(bridge.checkSelectedChannel).toHaveBeenCalledTimes(1);
    expect(screen.getByText(/上次检查/)).toBeTruthy();
  });
  it("检查期间禁用重复操作，失败显示原因并能再次检查", async () => {
    const { bridge, push } = installBridge();
    let finish!: () => void;
    bridge.checkSelectedChannel.mockImplementationOnce(() => new Promise((resolve) => {
      push({ phase: "checking" });
      finish = () => resolve(push({ phase: "error", error: "更新源连接失败" }));
    }));
    renderSettings(); await screen.findByText("当前版本：0.2.5");
    fireEvent.click(screen.getByRole("button", { name: "检查更新" }));
    expect((screen.getByRole("button", { name: "正在检查…" }) as HTMLButtonElement).disabled).toBe(true);
    await act(async () => finish());
    expect(screen.getByRole("alert").textContent).toBe("更新源连接失败");
    expect(screen.queryByText(/(?:正式版|测试版)暂无可用更新/)).toBeNull();
    bridge.checkSelectedChannel.mockImplementationOnce(async () => push({ phase: "idle", error: null, checkedAt: Date.now() }));
    fireEvent.click(screen.getByRole("button", { name: "检查更新" }));
    await screen.findByText(/(?:正式版|测试版)暂无可用更新/);
    expect(bridge.checkSelectedChannel).toHaveBeenCalledTimes(2);
  });
  it("发现版本后由员工下载，设置和标题栏同步进度与完成状态", async () => {
    const { bridge, push } = installBridge({ ...idle, phase: "available", availableVersion: "0.2.6", releaseNotes: "改进体验" });
    bridge.download.mockImplementationOnce(async () => push({ phase: "downloading", percent: 0 }));
    render(<><DesktopUpdateControl platformSignal={0}/><SettingsPage section="updates" onSectionChange={vi.fn()}
      workspace={null} hostPlatform="windows" onMessage={vi.fn()} projects={[]}/></>);
    await screen.findByText("可更新至 0.2.6（正式版）");
    expect(screen.getByText("改进体验")).toBeTruthy();
    expect(bridge.download).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "下载更新" }));
    await waitFor(() => expect(bridge.download).toHaveBeenCalledTimes(1));
    push({ phase: "downloading", percent: 42, transferred: 42 * 1024 * 1024, total: 100 * 1024 * 1024 });
    expect(screen.getByRole("progressbar", { name: "更新下载进度" }).getAttribute("value")).toBe("42");
    expect(screen.getByText("42% · 42.0 MB / 100.0 MB")).toBeTruthy();
    expect((screen.getByRole("button", { name: "检查更新" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByLabelText("词来工作台有新版本"));
    expect(within(screen.getByRole("dialog")).getByText("42%")).toBeTruthy();
    push({ phase: "downloaded", percent: 100 });
    expect(screen.getAllByRole("button", { name: "重启更新" })).toHaveLength(2);
    expect((screen.getByRole("button", { name: "检查更新" }) as HTMLButtonElement).disabled).toBe(true);
    expect(bridge.install).not.toHaveBeenCalled();
  });
  it("下载失败可重试下载，不把它变成新的版本检查", async () => {
    const { bridge } = installBridge({ ...idle, phase: "error", availableVersion: "0.2.6", error: "下载校验失败" });
    renderSettings(); await screen.findByRole("alert");
    fireEvent.click(screen.getByRole("button", { name: "重试下载" }));
    await waitFor(() => expect(bridge.download).toHaveBeenCalledTimes(1));
    expect(bridge.checkSelectedChannel).not.toHaveBeenCalled();
  });
  it("重启前等待草稿保存，重复点击只触发一次，保存失败保留应用供重试", async () => {
    const { bridge } = installBridge({ ...idle, phase: "downloaded", availableVersion: "0.2.6" });
    let fail!: (error: Error) => void;
    flushSessions.mockImplementationOnce(() => new Promise((_resolve, reject) => { fail = reject; }));
    renderSettings(); const restart = await screen.findByRole("button", { name: "重启更新" });
    fireEvent.click(restart); fireEvent.click(restart);
    expect(flushSessions).toHaveBeenCalledTimes(1);
    expect(bridge.install).not.toHaveBeenCalled();
    await act(async () => fail(new Error("草稿保存失败")));
    expect(screen.getByRole("alert").textContent).toContain("草稿保存失败");
    fireEvent.click(screen.getByRole("button", { name: "重启更新" }));
    await waitFor(() => expect(bridge.install).toHaveBeenCalledTimes(1));
  });
  it("助手启动失败明确提示，并可再次重启更新", async () => {
    const { bridge } = installBridge({ ...idle, phase: "downloaded", availableVersion: "0.2.6" });
    bridge.install.mockRejectedValueOnce(new Error("更新助手启动失败"));
    renderSettings(); fireEvent.click(await screen.findByRole("button", { name: "重启更新" }));
    expect((await screen.findByRole("alert")).textContent).toContain("更新助手启动失败");
    fireEvent.click(screen.getByRole("button", { name: "重启更新" }));
    await waitFor(() => expect(bridge.install).toHaveBeenCalledTimes(2));
  });
  it("状态读取失败不显示最新，检查更新可恢复", async () => {
    const { bridge } = installBridge(); bridge.getState.mockRejectedValueOnce(new Error("读取更新状态失败"));
    renderSettings(); await screen.findByRole("alert");
    expect(screen.queryByText(/(?:正式版|测试版)暂无可用更新/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "检查更新" }));
    await screen.findByText(/(?:正式版|测试版)暂无可用更新/);
    expect(screen.queryByRole("alert")).toBeNull();
  });
  it("较晚返回的初始状态不覆盖新的下载状态，离开页面取消订阅", async () => {
    const { bridge, push, listeners } = installBridge();
    let finish!: (state: DesktopUpdateState) => void;
    bridge.getState.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const view = renderSettings();
    push({ phase: "downloaded", availableVersion: "0.2.6" });
    await act(async () => finish(idle));
    expect(screen.getByRole("button", { name: "重启更新" })).toBeTruthy();
    view.unmount(); expect(listeners.size).toBe(0);
  });
  it("未提供更新接口的运行方式明确说明不可用", () => {
    renderSettings();
    expect(screen.getByText("当前运行方式不支持应用内更新。")).toBeTruthy();
    expect((screen.getByRole("button", { name: "检查更新" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.queryByText(/(?:正式版|测试版)暂无可用更新/)).toBeNull();
  });
  it("未配置更新的历史宿主明确显示不可用", async () => {
    const { bridge } = installBridge({ ...idle, supported: false });
    renderSettings(); await screen.findByText("当前版本：0.2.5");
    expect(screen.getByText("当前运行方式不支持应用内更新。")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "检查更新" }));
    expect(bridge.checkSelectedChannel).not.toHaveBeenCalled();
  });
});
