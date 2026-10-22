import { afterEach, describe, expect, test, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const rpc = vi.hoisted(() => vi.fn());
vi.mock("./app-server-rpc", () => ({ rpc }));

const { ProjectPortfolioPage } = await import("./project-portfolio");

afterEach(() => {
  cleanup();
  rpc.mockReset();
});

describe("项目总览表格", () => {
  test("后台更新保留已显示表格和创建表单，完成后更新内容", async () => {
    const item = { projectId: "project-1", displayName: "测试项目", goal: "目标", status: "active", workspaceId: null, workspaceIds: [], stateRevision: 0, updatedAt: 1, activePhases: [], nextPhaseName: null, taskTotal: 0, taskCompleted: 0, deliverableAccepted: 0, deliverableTotal: 0, blockerCount: 0, decisionCount: 0, incidentCount: 0 };
    rpc.mockResolvedValueOnce({ items: [item] });
    const props = { createIntent: 0, deleteProjectId: null, eventSequence: 0, onAddWorkspace: vi.fn(), onEditProject: vi.fn(), onMessage: vi.fn(), onOpenProgress: vi.fn(), workspaces: [] };
    const view = render(<ProjectPortfolioPage {...props}/>);
    const table = await screen.findByRole("table");
    fireEvent.click(screen.getByRole("button", { name: "新建项目" }));
    fireEvent.change(screen.getByLabelText("项目名称"), { target: { value: "正在填写" } });
    let finish!: (value: unknown) => void;
    rpc.mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    view.rerender(<ProjectPortfolioPage {...props} eventSequence={1}/>);
    expect(screen.getByRole("table")).toBe(table);
    expect(screen.queryByText("正在读取各项目推进事实…")).toBeNull();
    expect((screen.getByLabelText("项目名称") as HTMLInputElement).value).toBe("正在填写");
    await act(async () => finish({ items: [{ ...item, displayName: "项目新名称" }] }));
    expect(screen.getByRole("table")).toBe(table);
    expect(screen.getByRole("button", { name: "项目新名称" })).toBeTruthy();
    expect((screen.getByLabelText("项目名称") as HTMLInputElement).value).toBe("正在填写");
  });

  test("以带表头的表格展示项目，并保留进入、编辑和删除入口", async () => {
    rpc.mockResolvedValue({
      items: [{
        projectId: "project-1",
        displayName: "测试项目",
        goal: "验证项目表格",
        status: "active",
        workspaceId: "workspace-1",
        workspaceDisplayName: "测试工作区",
        workspaceIds: ["workspace-1", "workspace-2"],
        stateRevision: 1,
        updatedAt: Date.now(),
        hasPlan: true,
        activePhases: [{ phaseId: "phase-1", name: "需求阶段", status: "in_progress" }],
        nextPhaseName: null,
        taskTotal: 3,
        taskCompleted: 1,
        deliverableTotal: 2,
        deliverableAccepted: 1,
        blockerCount: 1,
        decisionCount: 0,
        incidentCount: 0,
      }],
    });
    const onOpenProgress = vi.fn();
    const onEditProject = vi.fn();

    render(<ProjectPortfolioPage
      createIntent={0}
      deleteProjectId={null}
      eventSequence={0}
      onAddWorkspace={vi.fn()}
      onEditProject={onEditProject}
      onMessage={vi.fn()}
      onOpenProgress={onOpenProgress}
      workspaces={[
        { workspaceId: "workspace-1", workspaceKind: "local", displayName: "测试工作区", localRoot: "/tmp/workspace-1", localStatus: "ready", stateRevision: 1, platformBound: false, platformRevision: null, teamSpaceId: null, managedRoot: null, deviceId: null, baselineRevision: null, changeCursor: null, permissionRole: null, permissionRevision: null },
        { workspaceId: "workspace-2", workspaceKind: "local", displayName: "辅助工作区", localRoot: "/tmp/workspace-2", localStatus: "ready", stateRevision: 1, platformBound: false, platformRevision: null, teamSpaceId: null, managedRoot: null, deviceId: null, baselineRevision: null, changeCursor: null, permissionRole: null, permissionRevision: null },
      ]}/>
    );

    const table = await screen.findByRole("table");
    expect(within(table).getAllByRole("columnheader").map((cell) => cell.textContent)).toEqual([
      "项目", "当前阶段", "已完成 / 总任务", "已验收 / 总交付", "待介入", "工作区", "项目更新", "操作",
    ]);
    const row = within(table).getAllByRole("row")[1];
    const projectCell = within(row).getAllByRole("cell")[0];
    expect(projectCell.textContent).toBe("测试项目");
    expect(projectCell.textContent).not.toContain("验证项目表格");
    expect(projectCell.textContent).not.toContain("需要介入");
    expect(row.textContent).toContain("需求阶段 · 进行中");
    expect(row.textContent).toContain("1/3");
    expect(row.textContent).toContain("1/2");
    expect(row.textContent).toContain("2 个");
    expect(within(row).getAllByRole("cell")[5].getAttribute("title")).toBe("测试工作区、辅助工作区");

    fireEvent.click(within(row).getByTitle("进入 测试项目"));
    expect(onOpenProgress).toHaveBeenCalledWith("workspace-1", "project-1", "phase-1");
    fireEvent.click(within(row).getByRole("button", { name: "编辑项目 测试项目" }));
    expect(onEditProject).toHaveBeenCalledWith("project-1");
    fireEvent.click(within(row).getByLabelText("更多项目操作 测试项目"));
    fireEvent.click(within(row).getByRole("button", { name: "删除项目 测试项目" }));
    await waitFor(() => expect(screen.getByRole("dialog", { name: "删除项目确认" })).toBeTruthy());
  });
});
