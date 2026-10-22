import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { ProjectProgressPage } from "./project-progress";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({
  invoke,
}));

const project = {
  projectId: "project-1",
  displayName: "渐进项目",
  goal: "先推进探索，再根据事实决定后续结构",
  status: "active",
  primaryWorkspaceId: null,
  stateRevision: 0,
  updatedAt: 1,
};

const task = (status: string, index: number) => ({
  businessTaskId: `task-${index}`,
  projectId: project.projectId,
  phaseId: null,
  title: `${status} 任务`,
  objective: "推进一项真实工作",
  inputContext: "",
  expectedOutput: "形成可核查结果",
  completionCriteria: "人工确认",
  assigneeType: "user",
  assigneeLabel: "测试人",
  priority: "medium",
  dueAt: null,
  attemptNo: 1,
  status,
  stateRevision: 0,
  updatedAt: 1_800_000_000_000 + index,
});

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation((command: string, args?: { method?: string; params?: { businessTaskId?: string } }) => {
    if (command === "ai_runtime_snapshot") return Promise.resolve({
      localAgentsEnabled: false,
      runtimes: [], models: [], agents: [], messages: [], permissionRequests: [], inboxReadItemIds: [], automationRules: [], skills: [],
      conversations: [
        { conversationId: "conversation-1", title: "项目讨论", archived: false, context: { projectId: project.projectId }, updatedAt: 1_800_000_000_100 },
        { conversationId: "conversation-other", title: "其他项目", archived: false, context: { projectId: "other" }, updatedAt: 1_800_000_000_200 },
      ],
      runs: [
        { runId: "run-1", conversationId: "conversation-1", startedAt: 1_800_000_000_100, completedAt: null, artifacts: [{ artifactId: "artifact-1", label: "分析记录", path: "/tmp/result.md" }] },
        { runId: "run-2", conversationId: "conversation-1", startedAt: 1_800_000_000_200, completedAt: null, artifacts: [{ artifactId: "artifact-2", label: "分析记录新版", path: "/tmp/result.md" }] },
      ],
    });
    if (command !== "app_server_request") return Promise.resolve({});
    if (args?.method === "project/get") return Promise.resolve({ project });
    if (args?.method === "deliverable/list") return Promise.resolve({ deliverables: [] });
    if (args?.method === "projectProgress/get") return Promise.resolve({ projectId: project.projectId, plans: [], tasks: [
      task("todo", 1), task("decision", 2), task("review", 3), task("in_progress", 4), task("completed", 5),
    ] });
    if (args?.method === "businessTask/get") return Promise.resolve({ task: task("decision", 2), contributions: [] });
    if (args?.method === "attentionCase/list") return Promise.resolve({ cases: [{ attentionCaseId: "old-incident", caseType: "agent_run_incident", status: "open", title: "恢复工作归类：旧对话", updatedAt: 1 }] });
    return Promise.resolve({});
  });
});

afterEach(() => cleanup());

describe("项目事实概览", () => {
  test("默认展示五种任务状态，并按待处理到已完成排列", async () => {
    const { container } = render(<ProjectProgressPage
      eventSequence={0}
      files={[]}
      hostPlatform="macos"
      initialPhaseId={null}
      initialProjectId={project.projectId}
      workspace={null}
      onExitProject={vi.fn()}
      onMessage={vi.fn()}
      onOpenAi={vi.fn()}
      onOpenConversation={vi.fn()}
      onOpenRun={vi.fn()}
      onOpenFile={vi.fn()}
      onPhaseSelected={vi.fn()}
    />);

    await waitFor(() => expect(screen.getByRole("button", { name: "概览" }).getAttribute("aria-current")).toBe("page"));
    expect([...container.querySelectorAll(".project-status-grid button span")].map((item) => item.textContent)).toEqual([
      "待处理", "待决策", "待验收", "进行中", "已完成",
    ]);
    expect(screen.getByRole("button", { name: "设置阶段" })).toBeTruthy();
    expect(screen.queryByText("恢复工作归类：旧对话")).toBeNull();
    expect(screen.getByRole("button", { name: /待决策 decision 任务/ })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /待决策 decision 任务/ }));
    const detail = await screen.findByRole("complementary", { name: "任务详情" });
    await within(detail).findByText("推进一项真实工作");
    expect(invoke.mock.calls.some(([command]) => command === "ai_gateway_credential_status")).toBe(false);
    fireEvent.click(within(detail).getByRole("button", { name: "关闭侧边面板" }));
    fireEvent.click(screen.getByRole("button", { name: "任务" }));
    fireEvent.click(screen.getByRole("button", { name: "decision 任务" }));
    await within(await screen.findByRole("complementary", { name: "任务详情" })).findByText("推进一项真实工作");

  });

  test("只统计当前项目对话，并按登记路径去重产物", async () => {
    const { container } = render(<ProjectProgressPage
      eventSequence={0}
      files={[]}
      hostPlatform="macos"
      initialPhaseId={null}
      initialProjectId={project.projectId}
      workspace={null}
      onExitProject={vi.fn()}
      onMessage={vi.fn()}
      onOpenAi={vi.fn()}
      onOpenConversation={vi.fn()}
      onOpenRun={vi.fn()}
      onOpenFile={vi.fn()}
      onPhaseSelected={vi.fn()}
    />);

    await waitFor(() => expect(container.querySelector(".project-overview-streams")).not.toBeNull());
    const header = container.querySelector<HTMLElement>(".project-summary-header")!;
    expect(header.textContent).toBe("渐进项目");
    expect(within(header).getByRole("button", { name: "返回项目列表" })).toBeTruthy();
    expect(container.querySelector(".project-summary-stats")).toBeNull();
    expect(screen.queryByText(project.goal)).toBeNull();
    expect(screen.queryByText("其他项目")).toBeNull();
    expect(screen.getByText("分析记录新版")).toBeTruthy();
  });
});


test("概览状态带入筛选，新建任务保留项目但不误绑当前阶段", async () => {
  const openAi = vi.fn();
  render(<ProjectProgressPage eventSequence={0} files={[]} hostPlatform="macos" initialPhaseId={null} initialProjectId={project.projectId}
    workspace={{ workspaceId: "ws", workspaceKind: "local", displayName: "工作空间", localRoot: "/tmp/ws", localStatus: "ready", stateRevision: 0, platformBound: false, platformRevision: null, teamSpaceId: null, managedRoot: null, deviceId: null, baselineRevision: null, changeCursor: null, permissionRole: null, permissionRevision: null }}
    onExitProject={vi.fn()} onMessage={vi.fn()} onOpenAi={openAi} onOpenConversation={vi.fn()} onOpenRun={vi.fn()} onOpenFile={vi.fn()} onPhaseSelected={vi.fn()}/>);
  fireEvent.click(await screen.findByRole("button", { name: "待决策 1" }));
  expect(screen.getByRole("button", { name: "decision 任务" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "todo 任务" })).toBeNull();
  fireEvent.change(screen.getByRole("searchbox", { name: "搜索项目任务" }), { target: { value: "无匹配" } });
  expect(screen.getByText("没有匹配的任务")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "新建任务" }));
  expect(openAi).toHaveBeenCalledWith(expect.objectContaining({ conversationType: "project", context: expect.objectContaining({ projectId: "project-1", phaseId: null, taskId: null }) }));
  expect(invoke.mock.calls.some(([, args]) => args?.method === "businessTask/create")).toBe(false);
});
