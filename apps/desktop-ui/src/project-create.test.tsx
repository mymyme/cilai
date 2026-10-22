import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { ProjectCreatePanel } from "./project-create";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({
  invoke,
}));
const workspace = (id: string, displayName: string) => ({ workspaceId: id, workspaceKind: "local", displayName, localRoot: `/tmp/${id}`, localStatus: "ready", stateRevision: 1, platformBound: false, platformRevision: null, teamSpaceId: null, managedRoot: null, deviceId: null, baselineRevision: null, changeCursor: null, permissionRole: null, permissionRevision: null });
const first = workspace("ws-1", "第一文件夹");
const second = workspace("ws-2", "第二文件夹");
const third = workspace("ws-3", "第三文件夹");
const project = { projectId: "project-new", displayName: "测试项目", goal: "形成验收材料", primaryWorkspaceId: null };
const defaults = { existingProjects: [], workspaces: [first, second], onAddWorkspace: async () => undefined, onCreated: vi.fn(), onMessage: vi.fn() };
const addFolder = async (name: string) => {
  fireEvent.click(screen.getByRole("button", { name: "添加本地文件夹…" }));
  await screen.findByRole("button", { name: `移除文件夹：${name}` });
  await vi.waitFor(() => expect((screen.getByRole("button", { name: "添加本地文件夹…" }) as HTMLButtonElement).disabled).toBe(false));
};
beforeEach(() => { vi.clearAllMocks(); invoke.mockReset(); invoke.mockResolvedValue({ project }); });
afterEach(() => cleanup());

describe("新建项目面板", () => {
  test("无目录可创建项目且不请求云服务，随后可另行关联目录", async () => {
    const onCreated = vi.fn();
    render(<ProjectCreatePanel {...defaults} onCreated={onCreated}/>);
    expect(screen.queryByRole("button", { name: /云端/ })).toBeNull();
    fireEvent.change(screen.getByLabelText("项目名称"), { target: { value: "先规划" } });
    fireEvent.change(screen.getByLabelText("项目说明"), { target: { value: "稍后整理目录" } });
    fireEvent.click(screen.getByRole("button", { name: "创建项目" }));
    await vi.waitFor(() => expect(onCreated).toHaveBeenCalledWith(project, null));
    expect(invoke).toHaveBeenCalledExactlyOnceWith("app_server_request", expect.objectContaining({ method: "project/create", params: expect.objectContaining({ workspaceIds: [], displayName: "先规划" }) }));
    expect(invoke.mock.calls[0][1].params.teamSpaceIds).toBeUndefined();
  });
  test("初始不自动关联已有文件夹，文件夹区域位于表单最后", () => {
    const { container } = render(<ProjectCreatePanel {...defaults}/>);
    expect((screen.getByLabelText("项目名称") as HTMLInputElement).value).toBe("");
    expect(screen.queryByRole("list", { name: "已选择的文件夹" })).toBeNull();
    expect(screen.queryByRole("combobox")).toBeNull();
    expect(screen.queryByText("文件夹：暂不关联，可稍后添加")).toBeNull();
    expect(container.querySelector(".project-create-folders")?.nextElementSibling?.className).toBe("project-create-actions");
  });

  test("添加多个文件夹直接显示名称和路径，重复选择不重复显示", async () => {
    const pick = vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(second).mockResolvedValueOnce(first);
    render(<ProjectCreatePanel {...defaults} onAddWorkspace={pick}/>);
    await addFolder(first.displayName);
    await addFolder(second.displayName);
    await addFolder(first.displayName);
    const list = screen.getByRole("list", { name: "已选择的文件夹" });
    expect(within(list).getAllByRole("listitem")).toHaveLength(2);
    expect(within(list).getByText(first.localRoot)).toBeTruthy();
    expect(within(list).getByText(second.localRoot)).toBeTruthy();
    expect(screen.queryByRole("combobox")).toBeNull();
  });

  test("移除仅取消本次选择，刷新不恢复，重新添加仍可选", async () => {
    const pick = vi.fn().mockResolvedValue(first);
    const props = { ...defaults, onAddWorkspace: pick };
    const { rerender } = render(<ProjectCreatePanel {...props}/>);
    await addFolder(first.displayName);
    fireEvent.change(screen.getByLabelText("项目名称"), { target: { value: "手工命名" } });
    fireEvent.change(screen.getByLabelText("项目说明"), { target: { value: "保留目标" } });
    fireEvent.click(screen.getByRole("button", { name: `移除文件夹：${first.displayName}` }));
    rerender(<ProjectCreatePanel {...props} workspaces={[{ ...first }, { ...second }]}/>);
    expect(screen.queryByRole("list")).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
    await addFolder(first.displayName);
    expect((screen.getByLabelText("项目名称") as HTMLInputElement).value).toBe("手工命名");
    expect((screen.getByLabelText("项目说明") as HTMLTextAreaElement).value).toBe("保留目标");
  });

  test("首个文件夹只补齐空项目名，后续添加不覆盖用户输入", async () => {
    const pick = vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(second);
    render(<ProjectCreatePanel {...defaults} onAddWorkspace={pick}/>);
    await addFolder(first.displayName);
    expect((screen.getByLabelText("项目名称") as HTMLInputElement).value).toBe(first.displayName);
    fireEvent.change(screen.getByLabelText("项目名称"), { target: { value: "自定义项目" } });
    await addFolder(second.displayName);
    expect((screen.getByLabelText("项目名称") as HTMLInputElement).value).toBe("自定义项目");
  });

  test("取消目录选择保持表单和列表不变", async () => {
    const pick = vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(null);
    render(<ProjectCreatePanel {...defaults} onAddWorkspace={pick}/>);
    await addFolder(first.displayName);
    fireEvent.click(screen.getByRole("button", { name: "添加本地文件夹…" }));
    await screen.findByRole("button", { name: "添加本地文件夹…" });
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
    expect((screen.getByLabelText("项目名称") as HTMLInputElement).value).toBe(first.displayName);
  });

  test("创建时一次提交保留的多个文件夹，不提交已移除项", async () => {
    const pick = vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(second).mockResolvedValueOnce(third);
    const onCreated = vi.fn();
    render(<ProjectCreatePanel {...defaults} onAddWorkspace={pick} onCreated={onCreated}/>);
    await addFolder(first.displayName);
    await addFolder(second.displayName);
    await addFolder(third.displayName);
    fireEvent.click(screen.getByRole("button", { name: `移除文件夹：${second.displayName}` }));
    fireEvent.change(screen.getByLabelText("项目说明"), { target: { value: "形成验收材料" } });
    fireEvent.click(screen.getByRole("button", { name: "创建项目" }));
    await vi.waitFor(() => expect(onCreated).toHaveBeenCalledWith(project, first.workspaceId));
    expect(invoke).toHaveBeenCalledExactlyOnceWith("app_server_request", expect.objectContaining({ method: "project/create", params: expect.objectContaining({ workspaceIds: [first.workspaceId, third.workspaceId] }) }));
  });






  test("加入已有项目一次提交所有选中文件夹", async () => {
    invoke.mockResolvedValue({ project, workspaceIds: [first.workspaceId, second.workspaceId] });
    const pick = vi.fn().mockResolvedValueOnce(first).mockResolvedValueOnce(second);
    render(<ProjectCreatePanel {...defaults} existingProjects={[project]} onAddWorkspace={pick}/>);
    await addFolder(first.displayName);
    await addFolder(second.displayName);
    fireEvent.click(screen.getByLabelText("或加入已有项目"));
    fireEvent.click(screen.getByRole("option", { name: project.displayName }));
    await vi.waitFor(() => expect(defaults.onCreated).toHaveBeenCalledWith(project, first.workspaceId));
    expect(invoke).toHaveBeenCalledExactlyOnceWith("app_server_request", expect.objectContaining({ method: "project/attachWorkspace", params: expect.objectContaining({ projectId: project.projectId, workspaceIds: [first.workspaceId, second.workspaceId] }) }));
  });

  test("选择期间禁止提交、移除和重复打开，选择失败保留列表", async () => {
    let rejectPick!: (reason: Error) => void;
    const pick = vi.fn().mockResolvedValueOnce(first).mockImplementationOnce(() => new Promise((_, reject) => { rejectPick = reject; }));
    render(<ProjectCreatePanel {...defaults} onAddWorkspace={pick}/>);
    await addFolder(first.displayName);
    fireEvent.change(screen.getByLabelText("项目说明"), { target: { value: "形成验收材料" } });
    fireEvent.click(screen.getByRole("button", { name: "添加本地文件夹…" }));
    expect((screen.getByRole("button", { name: "正在添加…" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "创建项目" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: `移除文件夹：${first.displayName}` }) as HTMLButtonElement).disabled).toBe(true);
    rejectPick(new Error("目录不可访问"));
    await vi.waitFor(() => expect(defaults.onMessage).toHaveBeenCalledWith("Error: 目录不可访问", "error"));
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
  });

  test("提交失败保留表单和列表供用户修改", async () => {
    invoke.mockRejectedValue(new Error("文件夹已属于其他项目"));
    render(<ProjectCreatePanel {...defaults} onAddWorkspace={async () => first}/>);
    await addFolder(first.displayName);
    fireEvent.change(screen.getByLabelText("项目说明"), { target: { value: "形成验收材料" } });
    fireEvent.click(screen.getByRole("button", { name: "创建项目" }));
    await vi.waitFor(() => expect(defaults.onMessage).toHaveBeenCalledWith("Error: 文件夹已属于其他项目", "error"));
    expect(defaults.onCreated).not.toHaveBeenCalled();
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
    expect((screen.getByLabelText("项目说明") as HTMLTextAreaElement).value).toBe("形成验收材料");
  });

  test("按 Esc 或点击遮罩关闭弹窗，名称或目标为空时不能提交", () => {
    const onCancel = vi.fn();
    const { container } = render(<ProjectCreatePanel {...defaults} onCancel={onCancel}/>);
    expect((screen.getByRole("button", { name: "创建项目" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.mouseDown(container.querySelector(".project-create-backdrop")!);
    fireEvent.mouseDown(container.querySelector(".project-create-panel")!);
    expect(onCancel).toHaveBeenCalledTimes(2);
  });
});
