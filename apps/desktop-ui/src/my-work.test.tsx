import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { MyWorkPage } from "./my-work";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({
  invoke,
}));

const workItem = {
  workItemId: "task:task-1",
  sourceType: "business_task",
  sourceId: "task-1",
  projectId: "project-1",
  projectName: "桌面端优化",
  primaryWorkspaceId: "workspace-1",
  phaseId: null,
  phaseName: null,
  taskId: "task-1",
  deliverableId: null,
  deliverableTitle: null,
  attentionCaseId: null,
  title: "聚合工作台任务详情",
  actionType: "task_execute",
  focusGroup: "in_progress",
  responsibilityType: "collaboration",
  responsibilityLabel: "AI 对话协作",
  priority: "medium",
  dueAt: null,
  sourceRevision: 1,
  updatedAt: 1_700_000_000_000,
};

const businessTask = {
  businessTaskId: "task-1",
  projectId: "project-1",
  phaseId: null,
  title: "聚合工作台任务详情",
  objective: "让用户从任务目的出发处理当前工作",
  inputContext: "工作台卡片及其关联 AI 对话",
  expectedOutput: "可直接判断下一步的任务详情",
  completionCriteria: "任务合同、候选成果和工作记录层级清楚且不重复",
  assigneeType: "collaboration",
  assigneeLabel: "AI 对话协作",
  priority: "medium",
  dueAt: null,
  attemptNo: 1,
  status: "in_progress",
  stateRevision: 1,
  updatedAt: 1_700_000_000_000,
};

const contributions = [{
  runId: "run-2", conversationId: "conversation-2", conversationStatus: "active", stage: "review", outcomeTitle: "已补充验收状态", outcomeSummary: "形成第二份候选文件", reason: "已经形成第二份候选文件", confidence: 0.95, createdAt: 1_700_000_000_300,
}, {
  runId: "run-1", conversationId: "conversation-1", conversationStatus: "active", stage: "in_progress", outcomeTitle: "已形成任务详情方案", outcomeSummary: "正在实现任务详情", reason: "正在实现", confidence: 0.96, createdAt: 1_700_000_000_100,
}];

const snapshot = {
  runtimes: [], models: [], agents: [], permissionRequests: [], inboxReadItemIds: [], automationRules: [], skills: [],
  conversations: [{
    conversationId: "conversation-1",
    title: "讨论工作台任务详情",
    conversationType: "task",
    mode: "discuss",
    agentId: "agent-1",
    context: { workspaceId: "workspace-1", workspaceName: "主工作区", workspaceRoot: "/tmp/workspace-1", projectId: "project-1", projectName: "桌面端优化", phaseId: null, phaseName: null, taskId: "task-1", taskName: "聚合工作台任务详情", deliverableId: null, deliverableName: null, localFileId: null, relativePath: null, fileRevision: null },
    workClassification: { runId: "run-1", taskId: "task-1", taskTitle: "聚合工作台任务详情", stage: "in_progress", reason: "正在实现", confidence: 0.96 },
    archived: false,
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_100,
  }, {
    conversationId: "conversation-2",
    title: "补充任务详情验收状态",
    conversationType: "task",
    mode: "discuss",
    agentId: "agent-1",
    context: { workspaceId: "workspace-1", workspaceName: "主工作区", workspaceRoot: "/tmp/workspace-1", projectId: "project-1", projectName: "桌面端优化", phaseId: null, phaseName: null, taskId: "task-1", taskName: "聚合工作台任务详情", deliverableId: null, deliverableName: null, localFileId: null, relativePath: null, fileRevision: null },
    workClassification: { runId: "run-2", taskId: "task-1", taskTitle: "聚合工作台任务详情", stage: "review", reason: "已经形成第二份候选文件", confidence: 0.95 },
    archived: false,
    createdAt: 1_700_000_000_200,
    updatedAt: 1_700_000_000_300,
  }],
  messages: [
    { messageId: "message-1", conversationId: "conversation-1", role: "user", content: "把对话和产物放进任务详情", runId: null, createdAt: 1_700_000_000_000 },
    { messageId: "message-2", conversationId: "conversation-1", role: "assistant", content: "已经形成任务详情方案", runId: "run-1", createdAt: 1_700_000_000_100 },
    { messageId: "message-3", conversationId: "conversation-2", role: "user", content: "补充多文件和状态展示", runId: null, createdAt: 1_700_000_000_200 },
    { messageId: "message-4", conversationId: "conversation-2", role: "assistant", content: "已经补充状态展示", runId: "run-2", createdAt: 1_700_000_000_300 },
  ],
  runs: [{
    runId: "run-1", conversationId: "conversation-1", agentId: "agent-1", inputMessageId: "message-1", retryOfRunId: null, provider: "codex", model: "Qwen3.6-35B-A3B-NVFP4", executionTarget: "model", status: "completed", pid: null, startedAt: 1_700_000_000_000, completedAt: 1_700_000_000_100, output: "已经形成任务详情方案", errorReason: null, classificationStatus: "applied", sessionId: null, skillIds: [], toolPermissions: [], permissionRequestId: null, automationRuleId: null, automationRuleVersion: null, sandboxMode: "read-only", events: [], artifacts: [{ artifactId: "artifact-old", artifactType: "document", label: "任务详情旧稿", path: "/tmp/task-detail.md" }, { artifactId: "artifact-1", artifactType: "document", label: "任务详情说明", path: "/tmp/task-detail-latest.md" }], usage: { inputTokens: 10, cachedInputTokens: 0, outputTokens: 20 }, logTail: [],
  }, {
    runId: "run-2", conversationId: "conversation-2", agentId: "agent-1", inputMessageId: "message-3", retryOfRunId: null, provider: "codex", model: "Qwen3.8-27B-NVFP4", executionTarget: "model", status: "completed", pid: null, startedAt: 1_700_000_000_200, completedAt: 1_700_000_000_300, output: "已经补充状态展示", errorReason: null, classificationStatus: "applied", sessionId: null, skillIds: [], toolPermissions: [], permissionRequestId: null, automationRuleId: null, automationRuleVersion: null, sandboxMode: "read-only", events: [], artifacts: [{ artifactId: "artifact-2", artifactType: "document", label: "任务详情验收标准", path: "/tmp/task-acceptance.md" }], usage: { inputTokens: 12, cachedInputTokens: 0, outputTokens: 24 }, logTail: [],
  }],
};

beforeEach(() => {
  Object.defineProperty(URL, "createObjectURL", { configurable: true, value: vi.fn(() => "blob:task-preview") });
  Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
  invoke.mockImplementation((command: string, args?: { method?: string }) => {
    if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [workItem], total: 1 });
    if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.resolve({ task: businessTask, contributions });
    if (command === "ai_runtime_snapshot") return Promise.resolve(snapshot);
    return Promise.resolve(null);
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("工作台卡片对话入口", () => {
  test.each([
    ["todo", "继续处理"],
    ["in_progress", "继续处理"],
    ["decision", "确认决策"],
    ["review", "查看成果"],
  ])("%s 的主操作打开该阶段对应对话", async (focusGroup, action) => {
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [{ ...workItem, focusGroup }], total: 1 });
      if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.resolve({ task: businessTask, contributions: [
        { ...contributions[0], stage: focusGroup === "todo" ? "review" : "todo" },
        { ...contributions[1], stage: focusGroup },
      ] });
      if (command === "ai_runtime_snapshot") return Promise.resolve({ ...snapshot, conversations: [
        ...snapshot.conversations,
        { ...snapshot.conversations[0], conversationId: "unrelated", updatedAt: 1_800_000_000_000 },
      ] });
      return Promise.reject(new Error(`unexpected ${command}`));
    });
    const onOpenConversation = vi.fn();
    const onOpenSource = vi.fn();
    render(<MyWorkPage eventSequence={0} onMessage={vi.fn()} onOpenConversation={onOpenConversation} onOpenRun={vi.fn()} onOpenSource={onOpenSource} workspaces={[]}/>);
    fireEvent.click(await screen.findByRole("button", { name: action }));
    await waitFor(() => expect(onOpenConversation).toHaveBeenCalledExactlyOnceWith("conversation-1"));
    expect(onOpenSource).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "关闭任务详情" })).toBeNull();
    expect(screen.queryByLabelText("处理决策")).toBeNull();
    expect(invoke.mock.calls.filter(([, args]) => args?.method).map(([, args]) => args.method)).toEqual(["myWork/get", "businessTask/get"]);
    fireEvent.click(screen.getByRole("button", { name: workItem.projectName }));
    expect(onOpenSource).toHaveBeenCalledWith(expect.objectContaining({ taskId: "task-1" }));
  });

  test("同阶段多条当前对话打开最近的有效任务贡献", async () => {
    const onOpenConversation = vi.fn();
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [{ ...workItem, focusGroup: "review" }], total: 1 });
      if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.resolve({ task: businessTask, contributions: [...contributions].reverse() });
      if (command === "ai_runtime_snapshot") return Promise.resolve(snapshot);
      return Promise.resolve(null);
    });
    render(<MyWorkPage eventSequence={0} onMessage={vi.fn()} onOpenConversation={onOpenConversation} onOpenRun={vi.fn()} onOpenSource={vi.fn()} workspaces={[]}/>);
    fireEvent.click(await screen.findByRole("button", { name: "查看成果" }));
    await waitFor(() => expect(onOpenConversation).toHaveBeenCalledExactlyOnceWith("conversation-2"));
  });

  test.each(["已归档", "已完成", "已被替换", "阶段已改变", "没有关联"])("%s 时明确提示，不打开其他对话", async (condition) => {
    const onMessage = vi.fn();
    const onOpenConversation = vi.fn();
    const onOpenSource = vi.fn();
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [{ ...workItem, focusGroup: "review" }], total: 1 });
      if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.resolve({ task: businessTask, contributions: condition === "没有关联" ? [] : [
        contributions[0],
        { ...contributions[0], createdAt: 1_700_000_000_500, conversationStatus: condition === "已完成" ? "completed" : "active", stage: condition === "阶段已改变" ? "todo" : "review" },
      ] });
      if (command === "ai_runtime_snapshot") return Promise.resolve({
        ...snapshot,
        conversations: snapshot.conversations.map((entry) => ({ ...entry, archived: entry.conversationId === "conversation-2" && condition === "已归档" })),
        runs: snapshot.runs.map((entry) => ({ ...entry, superseded: entry.runId === "run-2" && condition === "已被替换" })),
      });
      return Promise.resolve(null);
    });
    render(<MyWorkPage eventSequence={0} onMessage={onMessage} onOpenConversation={onOpenConversation} onOpenRun={vi.fn()} onOpenSource={onOpenSource} workspaces={[]}/>);
    const action = await screen.findByRole("button", { name: "查看成果" });
    fireEvent.click(action);
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith(expect.stringContaining("未找到该事项当前可打开的关联对话"), "error"));
    expect(onOpenConversation).not.toHaveBeenCalled();
    expect(onOpenSource).not.toHaveBeenCalled();
    expect(action.hasAttribute("disabled")).toBe(false);
  });

  test.each(["阻塞", "决策"])("%s 通过准确来源运行打开对话", async (source) => {
    const onOpenConversation = vi.fn();
    const item = source === "阻塞" ? { ...workItem, focusGroup: "todo", blockingRunId: "run-1" }
      : { ...workItem, focusGroup: "decision", attentionCaseId: "decision-1" };
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [item], total: 1 });
      if (command === "app_server_request" && args?.method === "attentionCase/list") return Promise.resolve({ cases: [{ attentionCaseId: "decision-1", agentRunId: "run-1" }] });
      if (command === "ai_runtime_snapshot") return Promise.resolve(snapshot);
      return Promise.reject(new Error(`unexpected ${command}`));
    });
    render(<MyWorkPage eventSequence={0} onMessage={vi.fn()} onOpenConversation={onOpenConversation} onOpenRun={vi.fn()} onOpenSource={vi.fn()} workspaces={[]}/>);
    fireEvent.click(await screen.findByRole("button", { name: source === "阻塞" ? "查看阻塞" : "确认决策" }));
    await waitFor(() => expect(onOpenConversation).toHaveBeenCalledExactlyOnceWith("conversation-1"));
    expect(invoke.mock.calls.some(([, args]) => args?.method === "businessTask/get")).toBe(false);
  });

  test("关联读取失败时保留工作台并允许再次打开", async () => {
    const onMessage = vi.fn();
    const onOpenConversation = vi.fn();
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [{ ...workItem, focusGroup: "review" }], total: 1 });
      if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.reject(new Error("任务读取超时"));
      if (command === "ai_runtime_snapshot") return Promise.resolve(snapshot);
      return Promise.resolve(null);
    });
    render(<MyWorkPage eventSequence={0} onMessage={onMessage} onOpenConversation={onOpenConversation} onOpenRun={vi.fn()} onOpenSource={vi.fn()} workspaces={[]}/>);
    const action = await screen.findByRole("button", { name: "查看成果" });
    fireEvent.click(action);
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith(expect.stringContaining("任务读取超时"), "error"));
    expect(onOpenConversation).not.toHaveBeenCalled();
    expect(action.hasAttribute("disabled")).toBe(false);
  });
});

describe("工作台任务详情", () => {
  test("按任务聚合原始对话、消息和运行产物", async () => {
    const onOpenConversation = vi.fn();
    const onOpenRun = vi.fn();
    render(<MyWorkPage
      eventSequence={0}
      onMessage={vi.fn()}
      onOpenConversation={onOpenConversation}
      onOpenRun={onOpenRun}
      onOpenSource={vi.fn()}
      workspaces={[]}
    />);

    await waitFor(() => expect(screen.getByText("聚合工作台任务详情")).toBeDefined());
    const card = screen.getByText("聚合工作台任务详情").closest("article")!;
    expect(within(card).queryByText("AI 对话协作")).toBeNull();
    expect(within(card).queryByRole("button", { name: "AI 执行" })).toBeNull();
    expect(card.querySelectorAll("footer button")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: workItem.title }));

    await waitFor(() => expect(screen.getByText("讨论工作台任务详情")).toBeDefined());
    expect(screen.getByText("让用户从任务目的出发处理当前工作")).toBeDefined();
    expect(screen.getByText("可直接判断下一步的任务详情")).toBeDefined();
    expect(screen.getByText("任务合同、候选成果和工作记录层级清楚且不重复")).toBeDefined();
    expect(screen.queryByText("执行 AI")).toBeNull();
    expect(screen.getByText("协作方式")).toBeDefined();
    expect(screen.getByText("AI 对话协作")).toBeDefined();
    expect(screen.getByText("补充任务详情验收状态")).toBeDefined();
    expect(screen.getByRole("heading", { name: "当前对话与产出" })).toBeDefined();
    const taskDefinition = screen.getByText("任务定义").closest("details")!;
    expect(taskDefinition.open).toBe(true);
    fireEvent.click(taskDefinition.querySelector("summary")!);
    expect(taskDefinition.open).toBe(false);
    expect(screen.queryByText("已形成任务详情方案")).toBeNull();
    expect(screen.queryByText("正在实现任务详情")).toBeNull();
    expect(screen.getByText("task-detail-latest.md")).toBeDefined();
    expect(screen.getByText("task-acceptance.md")).toBeDefined();
    expect(screen.getByText("task-detail.md")).toBeDefined();
    expect(screen.queryByRole("button", { name: "查看运行" })).toBeNull();
    expect(screen.queryByRole("button", { name: "验收" })).toBeNull();

    const firstConversation = screen.getByText("讨论工作台任务详情").closest("article")!;
    fireEvent.click(within(firstConversation).getByRole("button", { name: "打开对话：讨论工作台任务详情" }));
    expect(onOpenConversation).toHaveBeenCalledWith("conversation-1");
  });

  test("待验收显示查看成果按钮，标题仍可进入成果详情", async () => {
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [{ ...workItem, focusGroup: "review", actionType: "task_review" }], total: 1 });
      if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.resolve({ task: { ...businessTask, status: "review" }, contributions });
      if (command === "ai_runtime_snapshot") return Promise.resolve(snapshot);
      return Promise.resolve(null);
    });
    render(<MyWorkPage eventSequence={0} onMessage={vi.fn()} onOpenConversation={vi.fn()} onOpenRun={vi.fn()} onOpenSource={vi.fn()} workspaces={[]}/>);
    expect(await screen.findByRole("button", { name: "查看成果" })).toBeDefined();
    const title = screen.getByRole("button", { name: workItem.title });
    expect(title.closest("article")?.getAttribute("role")).toBeNull();
    fireEvent.click(title);
    expect(await screen.findByRole("heading", { name: "当前对话与产出" })).toBeDefined();
    expect(screen.getAllByRole("button", { name: /打开对话：/ })).toHaveLength(2);
    expect(screen.queryByRole("button", { name: "继续跟进" })).toBeNull();
    expect(screen.queryByRole("button", { name: "开始验收" })).toBeNull();
    expect(screen.queryByRole("button", { name: "验收" })).toBeNull();
  });

  test("已完成对话退出当前区域并进入历史记录", async () => {
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [workItem], total: 1 });
      if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.resolve({
        task: businessTask,
        contributions: contributions.map((item) => item.runId === "run-1" ? { ...item, conversationStatus: "completed", stage: null } : item),
      });
      if (command === "ai_runtime_snapshot") return Promise.resolve(snapshot);
      return Promise.resolve(null);
    });
    render(<MyWorkPage eventSequence={0} onMessage={vi.fn()} onOpenConversation={vi.fn()} onOpenRun={vi.fn()} onOpenSource={vi.fn()} workspaces={[]}/>);
    fireEvent.click(await screen.findByRole("button", { name: workItem.title }));
    const currentSection = (await screen.findByText("当前对话与产出")).closest("section")!;
    expect(within(currentSection).queryByText("讨论工作台任务详情")).toBeNull();
    const historyLabels = screen.getAllByText("历史记录");
    expect(historyLabels).toHaveLength(2);
    fireEvent.click(historyLabels[1]);
    expect(await screen.findByText("讨论工作台任务详情")).toBeDefined();
  });

  test("阻塞只指向该任务的失败运行，不混入项目内其他对话", async () => {
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [{ ...workItem, focusGroup: "todo", blockingRunId: "failed-run", blockingReason: "缺少任务依赖" }], total: 1 });
      if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.resolve({ task: businessTask, contributions });
      if (command === "ai_runtime_snapshot") return Promise.resolve({ ...snapshot, conversations: [...snapshot.conversations, { ...snapshot.conversations[0], conversationId: "unrelated", title: "无关项目对话", context: { ...snapshot.conversations[0].context, taskId: null }, workClassification: null }] });
      return Promise.resolve(null);
    });
    const onOpenRun = vi.fn();
    render(<MyWorkPage eventSequence={0} onMessage={vi.fn()} onOpenConversation={vi.fn()} onOpenRun={onOpenRun} onOpenSource={vi.fn()} workspaces={[]}/>);
    const todoColumn = (await screen.findByRole("heading", { name: "待处理" })).closest("section")!;
    expect(within(todoColumn).getByText("聚合工作台任务详情")).toBeDefined();
    fireEvent.click(await screen.findByRole("button", { name: workItem.title }));
    expect(await screen.findByText("缺少任务依赖")).toBeDefined();
    expect(screen.getByText("桌面端优化 · 待处理")).toBeDefined();
    await screen.findByText("讨论工作台任务详情");
    expect(screen.queryByText("无关项目对话")).toBeNull();
    const blocker = screen.getByRole("heading", { name: "当前阻塞" }).closest("section")!;
    fireEvent.click(within(blocker).getByRole("button", { name: "打开对话" }));
    expect(onOpenRun).toHaveBeenCalledWith("failed-run");
  });

  test("无任务编号的历史异常仍关联完整对话和后续产物", async () => {
    const classificationItem = {
      ...workItem,
      workItemId: "attention:classification-incident",
      sourceType: "agent_run_incident",
      sourceId: "classification-incident",
      taskId: null,
      attentionCaseId: "classification-incident",
      title: "恢复工作归类：生成产品配图",
      actionType: "agent_incident_review",
      focusGroup: "todo",
      responsibilityType: "user",
      responsibilityLabel: "当前用户",
      blockingRunId: "run-1",
      blockingReason: "工作归类输出未完成",
    };
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [classificationItem], total: 1 });
      if (command === "app_server_request" && args?.method === "attentionCase/list") return Promise.resolve({
        projectId: "project-1",
        cases: [{
          attentionCaseId: "classification-incident",
          caseType: "agent_run_incident",
          projectId: "project-1",
          phaseId: null,
          taskId: null,
          deliverableId: null,
          title: classificationItem.title,
          detail: classificationItem.blockingReason,
          status: "open",
          responsibilityType: "user",
          responsibilityLabel: "当前用户",
          reporterLabel: "词来 桌面端",
          priority: "high",
          dueAt: null,
          agentRunId: "run-1",
          agentLabel: "AI 工作归类",
          resolutionSummary: null,
          decisionOptions: null,
          decisionImpact: null,
          decisionEvidence: null,
          decisionRecommendation: null,
          decisionConsequence: null,
          decisionAction: null,
          decisionActionSummary: null,
          decisionActorLabel: null,
          decisionDelegatedTo: null,
          decisionActionAt: null,
          stateRevision: 0,
          createdAt: 1_700_000_000_100,
          updatedAt: 1_700_000_000_100,
          resolvedAt: null,
        }],
      });
      if (command === "ai_runtime_snapshot") return Promise.resolve({
        ...snapshot,
        runs: [...snapshot.runs, { ...snapshot.runs[0], runId: "run-later", startedAt: 1_700_000_000_400, artifacts: [{ artifactId: "later-image", artifactType: "image", label: "后续图片", path: "/tmp/later.png" }] }],
      });
      return Promise.resolve(null);
    });
    const onOpenRun = vi.fn();
    render(<MyWorkPage eventSequence={0} onMessage={vi.fn()} onOpenConversation={vi.fn()} onOpenRun={onOpenRun} onOpenSource={vi.fn()} workspaces={[]}/>);

    expect(await screen.findByText("恢复工作归类：生成产品配图")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: classificationItem.title }));
    expect(await screen.findByText("工作归类输出未完成")).toBeDefined();
    expect(screen.getAllByText("讨论工作台任务详情").length).toBeGreaterThan(0);
    expect(screen.queryByText("对话记录不可用")).toBeNull();
    expect(screen.getAllByText("后续图片").length).toBeGreaterThan(0);
    expect(screen.queryByText("补充任务详情验收状态")).toBeNull();
    const blocker = screen.getByRole("heading", { name: "当前阻塞" }).closest("section")!;
    fireEvent.click(within(blocker).getByRole("button", { name: "打开对话" }));
    expect(onOpenRun).toHaveBeenCalledWith("run-1");
  });

  test("同一对话只展示归入当前任务的运行", async () => {
    const foreignRun = {
      ...snapshot.runs[0],
      runId: "run-other-task",
      inputMessageId: "message-other-task",
      startedAt: 1_700_000_000_400,
      completedAt: 1_700_000_000_500,
      output: "这是另一个任务的结果",
      artifacts: [{ artifactId: "artifact-other", artifactType: "document", label: "其他任务文件", path: "/tmp/other-task.md" }],
    };
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [workItem], total: 1 });
      if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.resolve({ task: businessTask, contributions });
      if (command === "ai_runtime_snapshot") return Promise.resolve({
        ...snapshot,
        conversations: snapshot.conversations.map((conversation) => conversation.conversationId === "conversation-1" ? { ...conversation, workClassification: { ...conversation.workClassification!, runId: foreignRun.runId, taskId: "task-2", taskTitle: "其他任务" } } : conversation),
        messages: [...snapshot.messages, { messageId: "message-other-task", conversationId: "conversation-1", role: "user", content: "处理另一个任务", runId: null, createdAt: 1_700_000_000_400 }],
        runs: [...snapshot.runs, foreignRun],
      });
      return Promise.resolve(null);
    });

    render(<MyWorkPage eventSequence={0} onMessage={vi.fn()} onOpenConversation={vi.fn()} onOpenRun={vi.fn()} onOpenSource={vi.fn()} workspaces={[]}/>);
    fireEvent.click(await screen.findByRole("button", { name: workItem.title }));
    await screen.findByText("当前对话与产出");
    expect(screen.queryByText("这是另一个任务的结果")).toBeNull();
    expect(screen.queryByText("其他任务文件")).toBeNull();
    expect(screen.getByText("当前对话").parentElement?.textContent).toContain("2");
  });

  test("任务详情不展示已归档对话及其产物", async () => {
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [workItem], total: 1 });
      if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.resolve({ task: businessTask, contributions });
      if (command === "ai_runtime_snapshot") return Promise.resolve({
        ...snapshot,
        conversations: snapshot.conversations.map((conversation) => conversation.conversationId === "conversation-2" ? { ...conversation, archived: true } : conversation),
      });
      return Promise.resolve(null);
    });
    render(<MyWorkPage eventSequence={0} onMessage={vi.fn()} onOpenConversation={vi.fn()} onOpenRun={vi.fn()} onOpenSource={vi.fn()} workspaces={[]}/>);
    fireEvent.click(await screen.findByRole("button", { name: workItem.title }));
    await screen.findByRole("heading", { name: "当前对话与产出" });
    expect(screen.getByRole("button", { name: "打开对话：讨论工作台任务详情" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "打开对话：补充任务详情验收状态" })).toBeNull();
    expect(screen.queryByText("task-acceptance.md")).toBeNull();
    expect(screen.getByText("当前对话").parentElement?.textContent).toContain("1");
  });

});

function mockDetail(nextSnapshot: unknown, nextContributions = contributions, imageRead: () => Promise<unknown> = () => Promise.resolve(new Uint8Array([137, 80, 78, 71]).buffer)) {
  invoke.mockImplementation((command: string, args?: { method?: string }) => {
    if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [workItem], total: 1 });
    if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.resolve({ task: businessTask, contributions: nextContributions });
    if (command === "ai_runtime_snapshot") return Promise.resolve(nextSnapshot);
    if (command === "ai_artifact_read_image") return imageRead();
    if (command === "ai_artifact_open") return Promise.resolve(null);
    return Promise.reject(new Error(`unexpected ${command}`));
  });
}

async function openDetail() {
  const onOpenConversation = vi.fn();
  const view = render(<MyWorkPage eventSequence={0} onMessage={vi.fn()} onOpenConversation={onOpenConversation} onOpenRun={vi.fn()} onOpenSource={vi.fn()} workspaces={[]}/>);
  fireEvent.click(await screen.findByRole("button", { name: workItem.title }));
  await screen.findByRole("heading", { name: "当前对话与产出" });
  return { ...view, onOpenConversation };
}

const imageArtifact = { artifactId: "image-one", artifactType: "image", label: "生成图片", path: "/tmp/image-one.png" };

describe("任务产物浏览", () => {
  test("后一轮没有文件时保留之前的产物，排除被替换运行和回复文件", async () => {
    const oldRun = { ...snapshot.runs[0], runId: "run-old", startedAt: 1_699_999_999_900, artifacts: [imageArtifact] };
    const replacedRun = { ...snapshot.runs[0], runId: "run-replaced", superseded: true, startedAt: 1_700_000_000_500, artifacts: [{ ...imageArtifact, artifactId: "replaced", path: "/tmp/replaced.png" }] };
    mockDetail({ ...snapshot, runs: [oldRun, replacedRun, { ...snapshot.runs[0], artifacts: [{ artifactId: "reply", artifactType: "assistant-response", label: "回复", path: "/tmp/reply.md" }, { artifactId: "no-path", artifactType: "file", label: "未登记", path: null }] }, snapshot.runs[1]] }, [
      { ...contributions[1], runId: "run-replaced", createdAt: 1_700_000_000_500 }, ...contributions,
      { ...contributions[1], runId: "run-old", createdAt: 1_699_999_999_900 },
    ]);
    await openDetail();
    expect(screen.getByText("image-one.png")).toBeDefined();
    expect(screen.queryByText("replaced.png")).toBeNull();
    expect(screen.queryByText("reply.md")).toBeNull();
    expect(screen.queryByText("未登记")).toBeNull();
    expect(screen.getByText("产物").parentElement?.textContent).toBe("产物2");
    expect(screen.getAllByText("历史记录")[0].parentElement?.textContent).toBe("历史记录1");
  });

  test("同一路径跨轮及跨对话登记时任务总数不重复，使用最新登记打开", async () => {
    const duplicate = { ...snapshot.runs[0].artifacts[1], artifactId: "newest", path: "/tmp//task-detail-latest.md" };
    mockDetail({ ...snapshot, runs: [snapshot.runs[0], { ...snapshot.runs[1], artifacts: [duplicate] }] });
    await openDetail();
    expect(screen.getByText("产物").parentElement?.textContent).toBe("产物2");
    const second = screen.getByRole("button", { name: "打开对话：补充任务详情验收状态" }).closest("article")!;
    fireEvent.click(within(second).getByRole("button", { name: "打开文件：task-detail-latest.md" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_artifact_open", { runId: "run-2", artifactId: "newest", openMode: "default" }));
  });

  test("图片由登记标识读取，关闭预览只关闭上层并恢复焦点", async () => {
    mockDetail({ ...snapshot, runs: [{ ...snapshot.runs[0], artifacts: [imageArtifact] }, snapshot.runs[1]] });
    const { onOpenConversation } = await openDetail();
    const trigger = screen.getByRole("button", { name: "预览图片：image-one.png" });
    await waitFor(() => expect((trigger as HTMLButtonElement).disabled).toBe(false));
    expect(invoke).toHaveBeenCalledWith("ai_artifact_read_image", { runId: "run-1", artifactId: "image-one" });
    trigger.focus();
    fireEvent.click(trigger);
    expect(screen.getByRole("dialog", { name: "查看图片 image-one.png" })).toBeDefined();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "查看图片 image-one.png" })).toBeNull();
    expect(screen.getByRole("complementary", { name: "任务详情" })).toBeDefined();
    expect(document.activeElement).toBe(trigger);
    expect(onOpenConversation).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "定位文件：image-one.png" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_artifact_open", { runId: "run-1", artifactId: "image-one", openMode: "finder" }));
  });

  test("超过四份产物可展开，文件打开失败明确显示", async () => {
    const artifacts = Array.from({ length: 6 }, (_, i) => ({ artifactId: `file-${i}`, artifactType: "document", label: `文件${i}`, path: `/tmp/file-${i}.md` }));
    mockDetail({ ...snapshot, runs: [{ ...snapshot.runs[0], artifacts }, snapshot.runs[1]] });
    const { onOpenConversation } = await openDetail();
    expect(screen.queryByText("file-0.md")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "查看全部 6 份产物" }));
    expect(screen.getByText("file-0.md")).toBeDefined();
    invoke.mockRejectedValueOnce(new Error("ai_artifact_unavailable"));
    fireEvent.click(screen.getByRole("button", { name: "打开文件：file-0.md" }));
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", "无法打开：产物不存在或已失效");
    expect(onOpenConversation).not.toHaveBeenCalled();
  });

  test("图片失效有明确原因和手动重试，不静默打开其他文件", async () => {
    const read = vi.fn().mockRejectedValueOnce(new Error("ai_artifact_unavailable")).mockResolvedValue(new Uint8Array([137, 80, 78, 71]).buffer);
    mockDetail({ ...snapshot, runs: [{ ...snapshot.runs[0], artifacts: [imageArtifact] }, snapshot.runs[1]] }, contributions, read);
    await openDetail();
    expect(await screen.findByRole("status")).toHaveProperty("textContent", "产物不存在或已失效重试");
    expect((screen.getByRole("button", { name: "预览图片：image-one.png" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() => expect((screen.getByRole("button", { name: "预览图片：image-one.png" }) as HTMLButtonElement).disabled).toBe(false));
    expect(read).toHaveBeenCalledTimes(2);
    expect(invoke.mock.calls.some(([command]) => command === "ai_artifact_open")).toBe(false);
  });

  test("同一对话的不同任务图片不被读取", async () => {
    mockDetail({ ...snapshot, runs: [...snapshot.runs, { ...snapshot.runs[0], runId: "foreign", artifacts: [imageArtifact] }] });
    await openDetail();
    expect(screen.queryByText("image-one.png")).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "ai_artifact_read_image")).toBe(false);
  });
});

test("工作项读取失败就地重试，成功后清除错误并显示任务", async () => {
  const onMessage = vi.fn();
  invoke.mockRejectedValueOnce(new Error("读取超时"));
  render(<MyWorkPage workspaces={[]} eventSequence={0} onMessage={onMessage} onOpenSource={vi.fn()} onOpenRun={vi.fn()} onOpenConversation={vi.fn()}/>);
  expect(await screen.findByText("工作项读取失败")).toBeTruthy();
  expect(onMessage).not.toHaveBeenCalled();
  expect(screen.queryByText("暂无需要处理的事项")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  expect(await screen.findByText("聚合工作台任务详情")).toBeTruthy();
  expect(screen.queryByText("工作项读取失败")).toBeNull();
});


test("首页直接显示完整看板，保留所有状态，并可继续最近对话", async () => {
  const open = vi.fn();
  const start = vi.fn();
  invoke.mockResolvedValue({ items: [], total: 0 });
  render(<MyWorkPage workspaces={[]} eventSequence={0} onMessage={() => {}} onOpenConversation={open} onOpenRun={() => {}} onOpenSource={() => {}} onStartConversation={start} recentConversations={[{ conversationId: "recent-1", title: "昨日方案", updatedAt: 1, projectName: "桌面体验", running: false }]}/>);
  fireEvent.click(screen.getByRole("button", { name: /昨日方案/ }));
  expect(open).toHaveBeenCalledWith("recent-1");
  fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
  expect(start).toHaveBeenCalledOnce();
  await screen.findByRole("heading", { name: "待处理" });
  expect(screen.queryByRole("group", { name: "工作视图" })).toBeNull();
  expect(screen.queryByRole("button", { name: "当前事项" })).toBeNull();
  expect(screen.queryByRole("button", { name: "完整看板" })).toBeNull();
  const columns = Array.from(document.querySelectorAll(".my-work-column"));
  expect(columns.map((column) => column.querySelector("h3")?.textContent)).toEqual(["待处理", "待决策", "进行中", "待验收"]);
  expect(columns.map((column) => column.querySelector("header > strong")?.textContent)).toEqual(["0", "0", "0", "0"]);
  expect(screen.getAllByText("暂无需要处理的事项")).toHaveLength(4);
});


test("新建任务直接交给对话入口，不显示任务合同表单", async () => {
  const start = vi.fn();
  render(<MyWorkPage workspaces={[]} eventSequence={0} onMessage={vi.fn()} onOpenSource={vi.fn()} onOpenRun={vi.fn()} onOpenConversation={vi.fn()} onStartTask={start}/>);
  await screen.findByText("聚合工作台任务详情");
  fireEvent.click(screen.getByRole("button", { name: "新建任务" }));
  expect(start).toHaveBeenCalledWith(null);
  expect(screen.queryByLabelText("任务目标")).toBeNull();
  expect(invoke.mock.calls.some(([, args]) => args?.method === "businessTask/create")).toBe(false);
});
