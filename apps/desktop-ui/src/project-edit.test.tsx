import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({
  invoke,
}));

const { ProjectEditDialog } = await import("./project-edit");

const target = {
  projectId: "proj-1",
  displayName: "旧名",
  goal: "旧目标",
  stateRevision: 3,
  primaryWorkspaceId: "ws-1",
  workspaceIds: ["ws-1", "ws-2"],
};

const workspace = (id: string, name: string) => ({
  workspaceId: id,
  workspaceKind: "local",
  displayName: name,
  localRoot: `/tmp/${id}`,
  localStatus: "ready",
  stateRevision: 1,
  platformBound: false,
  platformRevision: null,
  teamSpaceId: null,
  managedRoot: null,
  deviceId: null,
  baselineRevision: null,
  changeCursor: null,
  permissionRole: null,
  permissionRevision: null,
});

const workspaces = [workspace("ws-1", "主文件夹"), workspace("ws-2", "支撑文件夹"), workspace("ws-3", "别的项目的")];

const setup = (over: Partial<Parameters<typeof ProjectEditDialog>[0]> = {}) => {
  const handlers = {
    onAddFolder: vi.fn(), onRemoveFolder: vi.fn(),
    onReveal: vi.fn(), onSaved: vi.fn(), onMessage: vi.fn(), onClose: vi.fn(),
  };
  render(<ProjectEditDialog connectingWorkspace={false} hostPlatform="macos" target={target} workspaces={workspaces} {...handlers} {...over}/>);
  return handlers;
};

beforeEach(() => { invoke.mockReset(); });
afterEach(cleanup);

describe("项目编辑", () => {
  test("名称与目标可编辑，且预填当前值", () => {
    setup();
    expect((screen.getByLabelText(/项目名称/) as HTMLInputElement).value).toBe("旧名");
    expect((screen.getByLabelText(/项目说明/) as HTMLTextAreaElement).value).toBe("旧目标");
  });

  test("未改动时保存不可用，改动后才可用", () => {
    setup();
    const save = () => screen.getByRole("button", { name: "保存" }) as HTMLButtonElement;
    expect(save().disabled).toBe(true);
    fireEvent.change(screen.getByLabelText(/项目名称/), { target: { value: "新名" } });
    expect(save().disabled).toBe(false);
  });

  test("添加文件夹后保存可用，确认后关闭已完成的编辑", () => {
    const handlers = {
      onAddFolder: vi.fn(), onRemoveFolder: vi.fn(), onReveal: vi.fn(),
      onSaved: vi.fn(), onMessage: vi.fn(), onClose: vi.fn(),
    };
    const view = render(<ProjectEditDialog connectingWorkspace={false} hostPlatform="macos" target={target} workspaces={workspaces} {...handlers}/>);
    expect((screen.getByRole("button", { name: "保存" }) as HTMLButtonElement).disabled).toBe(true);

    view.rerender(<ProjectEditDialog connectingWorkspace={false} hostPlatform="macos" target={{ ...target, stateRevision: 4, workspaceIds: [...target.workspaceIds, "ws-3"] }} workspaces={workspaces} {...handlers}/>);
    expect((screen.getByRole("button", { name: "保存" }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    expect(handlers.onClose).toHaveBeenCalledOnce();
    expect(invoke).not.toHaveBeenCalled();
  });

  test("文件夹变更后继续修改项目时使用最新修订保存", async () => {
    const updatedProject = { ...target, displayName: "新名", stateRevision: 5 };
    invoke.mockResolvedValue({ project: updatedProject });
    const handlers = {
      onAddFolder: vi.fn(), onRemoveFolder: vi.fn(), onReveal: vi.fn(),
      onSaved: vi.fn(), onMessage: vi.fn(), onClose: vi.fn(),
    };
    const view = render(<ProjectEditDialog connectingWorkspace={false} hostPlatform="macos" target={target} workspaces={workspaces} {...handlers}/>);
    view.rerender(<ProjectEditDialog connectingWorkspace={false} hostPlatform="macos" target={{ ...target, stateRevision: 4, workspaceIds: [...target.workspaceIds, "ws-3"] }} workspaces={workspaces} {...handlers}/>);
    fireEvent.change(screen.getByLabelText(/项目名称/), { target: { value: "新名" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => expect(handlers.onSaved).toHaveBeenCalledWith(updatedProject));
    expect(JSON.stringify(invoke.mock.calls.at(-1))).toContain('"expectedRevision":4');
  });

  test("文件夹操作进行中锁定弹窗操作", () => {
    setup({ connectingWorkspace: true });
    expect((screen.getByRole("button", { name: "添加本地文件夹" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.queryByRole("button", { name: "添加云端" })).toBeNull();
    expect((screen.getByRole("button", { name: "关闭" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "保存" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getAllByRole("button", { name: "移出项目" })[0] as HTMLButtonElement).disabled).toBe(true);
  });

  test("名称清空后不可保存", () => {
    setup();
    fireEvent.change(screen.getByLabelText(/项目名称/), { target: { value: "  " } });
    expect((screen.getByRole("button", { name: "保存" }) as HTMLButtonElement).disabled).toBe(true);
  });

  test("保存时带上编辑器所见的修订，供服务端拒绝过期写入", async () => {
    const updatedProject = { ...target, displayName: "新名" };
    invoke.mockResolvedValue({ project: updatedProject });
    const handlers = setup();
    fireEvent.change(screen.getByLabelText(/项目名称/), { target: { value: "新名" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(handlers.onSaved).toHaveBeenCalled());
    const payload = JSON.stringify(invoke.mock.calls.at(-1));
    expect(payload).toContain("project/update");
    expect(payload).toContain('"expectedRevision":3');
    expect(payload).toContain("新名");
    expect(handlers.onSaved).toHaveBeenCalledWith(updatedProject);
  });

  test("修订冲突时给出可操作的说明，而不是抛出原始错误码", async () => {
    invoke.mockRejectedValue(new Error("project_revision_conflict"));
    const handlers = setup();
    fireEvent.change(screen.getByLabelText(/项目名称/), { target: { value: "新名" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("已被改动"));
    expect(handlers.onSaved).not.toHaveBeenCalled();
  });

  test("只列出本项目的文件夹，不混入别的项目的", () => {
    setup();
    const dialog = screen.getByRole("dialog", { name: "编辑项目" });
    expect(within(dialog).getByText("主文件夹")).toBeTruthy();
    expect(within(dialog).getByText("支撑文件夹")).toBeTruthy();
    expect(within(dialog).queryByText("别的项目的")).toBeNull();
  });

  test("所有文件夹都可移出，且项目编辑不提供容易混淆的应用级移除", () => {
    setup();
    const dialog = screen.getByRole("dialog", { name: "编辑项目" });
    expect(within(dialog).getAllByRole("button", { name: "移出项目" })).toHaveLength(2);
    expect(within(dialog).queryByRole("button", { name: "移除" })).toBeNull();
    expect(within(dialog).queryByText("第一个文件夹")).toBeNull();
  });

  test("原有本地文件夹入口保持直接选择流程", () => {
    const handlers = setup();
    fireEvent.click(screen.getByRole("button", { name: "添加本地文件夹" }));
    expect(handlers.onAddFolder).toHaveBeenCalled();
  });


  test("Windows 文件定位入口使用文件管理器文案", () => {
    setup({ hostPlatform: "windows" });
    expect(screen.getAllByRole("button", { name: "在文件管理器中显示" })).toHaveLength(2);
    expect(screen.queryByRole("button", { name: "在访达中显示" })).toBeNull();
  });

  test("Esc 关闭", () => {
    const handlers = setup();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(handlers.onClose).toHaveBeenCalled();
  });
});
