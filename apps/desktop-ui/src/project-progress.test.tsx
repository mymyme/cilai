import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { ProjectProgressPage } from "./project-progress";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({
  invoke,
}));
vi.mock("./app-server-rpc", async (original) => ({
  ...await original<typeof import("./app-server-rpc")>(),
  sha256: async () => "test-fingerprint",
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
    if (command === "ai_runtime_query") return Promise.resolve({
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((accept, fail) => { resolve = accept; reject = fail; });
  return { promise, resolve, reject };
}

function pageProps() {
  return {
    eventSequence: 0, files: [], hostPlatform: "macos", initialPhaseId: null, initialProjectId: project.projectId,
    workspace: null, operatorName: "测试人", onExitProject: vi.fn(), onMessage: vi.fn(), onOpenAi: vi.fn(),
    onOpenConversation: vi.fn(), onOpenRun: vi.fn(), onOpenFile: vi.fn(), onPhaseSelected: vi.fn(),
  };
}

test("切换项目后迟到的推进事实不能覆盖新项目或留下旧任务操作入口", async () => {
  const old = deferred<{ plans: []; tasks: ReturnType<typeof task>[] }>();
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => {
    if (args?.method === "project/get") return { project: { ...project, projectId: args.params.projectId, displayName: args.params.projectId } };
    if (args?.method === "projectProgress/get") return args.params.projectId === "project-a" ? old.promise : { plans: [], tasks: [{ ...task("todo", 2), projectId: "project-b", title: "乙项目任务" }] };
    return original(command, args);
  });
  const props = pageProps();
  const view = render(<ProjectProgressPage {...props} initialProjectId="project-a"/>);
  await waitFor(() => expect(invoke.mock.calls.some(([, args]) => args?.method === "projectProgress/get" && args.params.projectId === "project-a")).toBe(true));
  view.rerender(<ProjectProgressPage {...props} initialProjectId="project-b"/>);
  await screen.findByText("乙项目任务");
  await act(async () => old.resolve({ plans: [], tasks: [{ ...task("todo", 1), projectId: "project-a", title: "甲项目任务" }] }));
  expect(screen.getByRole("heading", { name: "project-b" })).toBeTruthy();
  expect(screen.getByText("乙项目任务")).toBeTruthy();
  expect(screen.queryByText("甲项目任务")).toBeNull();
  expect(props.onPhaseSelected).not.toHaveBeenCalledWith(expect.any(String), expect.any(String));
});

test.each(["success", "failure"] as const)("同项目连续刷新忽略较早读取的%s回执", async (outcome) => {
  const old = deferred<{ plans: []; tasks: ReturnType<typeof task>[] }>();
  const original = invoke.getMockImplementation()!;
  let reads = 0;
  invoke.mockImplementation((command, args) => {
    if (args?.method === "projectProgress/get") return ++reads === 1 ? old.promise : { plans: [], tasks: [{ ...task("todo", 2), title: "最新任务" }] };
    return original(command, args);
  });
  const props = pageProps();
  const view = render(<ProjectProgressPage {...props}/>);
  await waitFor(() => expect(reads).toBe(1));
  view.rerender(<ProjectProgressPage {...props} eventSequence={1}/>);
  await screen.findByText("最新任务");
  await act(async () => {
    if (outcome === "success") old.resolve({ plans: [], tasks: [{ ...task("todo", 1), title: "过期任务" }] });
    else old.reject(new Error("过期读取失败"));
  });
  expect(screen.getByText("最新任务")).toBeTruthy();
  expect(screen.queryByText("过期任务")).toBeNull();
  expect(props.onMessage).not.toHaveBeenCalled();
});

test("切换工作区后旧项目列表回执不继续发起旧项目读取", async () => {
  const old = deferred<{ projects: typeof project[] }>();
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => {
    if (args?.method === "project/list") return args.params.workspaceId === "workspace-a" ? old.promise : { projects: [{ ...project, projectId: "project-b", displayName: "乙项目" }] };
    return original(command, args);
  });
  const props = pageProps();
  const workspace = { workspaceId: "workspace-a", workspaceKind: "local", displayName: "甲", localRoot: "/tmp/a", localStatus: "ready", stateRevision: 0, platformBound: false, platformRevision: null, teamSpaceId: null, managedRoot: null, deviceId: null, baselineRevision: null, changeCursor: null, permissionRole: null, permissionRevision: null };
  const view = render(<ProjectProgressPage {...props} initialProjectId={null} workspace={workspace}/>);
  await waitFor(() => expect(invoke.mock.calls.some(([, args]) => args?.method === "project/list")).toBe(true));
  view.rerender(<ProjectProgressPage {...props} initialProjectId={null} workspace={{ ...workspace, workspaceId: "workspace-b" }}/>);
  await screen.findByRole("heading", { name: "乙项目" });
  await act(async () => old.resolve({ projects: [{ ...project, projectId: "project-a" }] }));
  expect(screen.getByRole("heading", { name: "乙项目" })).toBeTruthy();
  expect(invoke.mock.calls.some(([, args]) => args?.method === "projectProgress/get" && args.params.projectId === "project-a")).toBe(false);
});

test.each(["success", "failure"] as const)("切换项目后忽略旧项目计划创建的%s回执", async (outcome) => {
  const pending = deferred<{ plan: { planId: string; status: string; phases: [] } }>();
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => {
    if (args?.method === "project/get") return { project: { ...project, projectId: args.params.projectId, displayName: args.params.projectId } };
    if (args?.method === "phasePlan/create") return pending.promise;
    return original(command, args);
  });
  const props = pageProps();
  const view = render(<ProjectProgressPage {...props}/>);
  fireEvent.click(await screen.findByRole("button", { name: "设置阶段" }));
  for (const label of ["计划说明", "阶段名称", "阶段目标", "完成标准"]) fireEvent.change(screen.getByLabelText(label), { target: { value: label } });
  fireEvent.click(screen.getByRole("button", { name: "保存草案" }));
  await waitFor(() => expect(invoke.mock.calls.some(([, args]) => args?.method === "phasePlan/create")).toBe(true));
  props.onMessage.mockClear();
  view.rerender(<ProjectProgressPage {...props} initialProjectId="project-b"/>);
  await screen.findByRole("heading", { name: "project-b" });
  await act(async () => {
    if (outcome === "success") pending.resolve({ plan: { planId: "old-plan", status: "draft", phases: [] } });
    else pending.reject(new Error("旧项目写入失败"));
  });
  expect(screen.getByRole("heading", { name: "project-b" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "设置阶段" }));
  expect(screen.queryByRole("button", { name: "激活计划" })).toBeNull();
  expect(props.onMessage).not.toHaveBeenCalled();
});

test("变更回执已到达时，先前开始的刷新不能撤回已保存计划", async () => {
  const pending = deferred<{ plan: { planId: string; status: string; phases: [] } }>();
  const refresh = deferred<{ plans: []; tasks: [] }>();
  const original = invoke.getMockImplementation()!;
  let reads = 0;
  invoke.mockImplementation((command, args) => {
    if (args?.method === "phasePlan/create") return pending.promise;
    if (args?.method === "projectProgress/get" && ++reads > 1) return refresh.promise;
    return original(command, args);
  });
  const props = pageProps();
  const view = render(<ProjectProgressPage {...props}/>);
  fireEvent.click(await screen.findByRole("button", { name: "设置阶段" }));
  for (const label of ["计划说明", "阶段名称", "阶段目标", "完成标准"]) fireEvent.change(screen.getByLabelText(label), { target: { value: label } });
  fireEvent.click(screen.getByRole("button", { name: "保存草案" }));
  await waitFor(() => expect(invoke.mock.calls.some(([, args]) => args?.method === "phasePlan/create")).toBe(true));
  view.rerender(<ProjectProgressPage {...props} eventSequence={1}/>);
  await waitFor(() => expect(reads).toBe(2));
  await act(async () => pending.resolve({ plan: { planId: "saved-plan", status: "draft", phases: [] } }));
  await act(async () => refresh.resolve({ plans: [], tasks: [] }));
  expect(screen.getByRole("button", { name: "激活计划" })).toBeTruthy();
});

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
