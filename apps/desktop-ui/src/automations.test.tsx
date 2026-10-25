import type { ComponentProps } from "react";
import { ModelCatalogProvider } from "./use-model-catalog";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { aiRuntimeChangedEvent, type AiAutomationRule, type AiRuntimeSnapshot } from "./ai-runtime";
import { AutomationsPage as AutomationsPageImplementation, filterAutomationRules, nextRunDistance } from "./automations";

function AutomationsPage(props: ComponentProps<typeof AutomationsPageImplementation>) { return <ModelCatalogProvider profileId="test-profile"><AutomationsPageImplementation {...props}/></ModelCatalogProvider>; }

const invoke = vi.hoisted(() => vi.fn());
const modelApi = vi.hoisted(() => ({ models: [] as unknown[], fail: null as string | null }));
vi.mock("./host", () => ({
  listen: async (event: string, callback: (value: unknown) => void) => { const handler = (value: Event) => callback((value as CustomEvent).detail); window.addEventListener(event, handler); return () => window.removeEventListener(event, handler); },
  invoke: async (command: string, args: unknown) => {
    if (command === "ai_model_catalog_read") {
      await Promise.resolve();
      if (modelApi.fail) throw new Error(modelApi.fail);
      return { profileId: "test-profile", catalog: { generation: 0, revision: 1, checkedAt: Date.now(), revalidateAfter: Date.now() + 300_000, error: null, status: { configured: true, endpoint: "https://models.example.test/v1", models: modelApi.models } } };
    }
    const result = await invoke(command, args);
    if (command === "ai_runtime_query" && result) { modelApi.models = result.models; return structuredClone(result); }
    return result;
  },
}));

const workspace = {
  workspaceId: "ws-1",
  workspaceKind: "local",
  displayName: "项目工作区",
  localRoot: "/tmp/ws-1",
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
};

function rule(overrides: Partial<AiAutomationRule> & Pick<AiAutomationRule, "automationRuleId" | "name">): AiAutomationRule {
  return {
    triggerType: "interval",
    intervalMinutes: 60,
    scheduleKind: "daily",
    timeOfDayMinutes: 9 * 60,
    weekday: 1,
    weekdays: [1],
    notifyOnEveryRun: false,
    conditionSummary: "每天 09:00，在当前工作区内执行",
    actionPrompt: "梳理工作区最近变化",
    workspaceId: workspace.workspaceId,
    workspaceName: workspace.displayName,
    workspaceRoot: workspace.localRoot,
    projectId: "project-1",
    projectName: "示例项目",
    agentId: "agent-1",
    riskLevel: "low",
    ruleVersion: 2,
    enabled: true,
    nextRunAt: Date.now() + 3_600_000,
    lastRunId: null,
    lastRunAt: null,
    lastError: null,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

const rules = [
  rule({ automationRuleId: "daily", name: "每日项目摘要", actionPrompt: "汇总每天的项目变化" }),
  rule({ automationRuleId: "weekly", name: "周五进展回顾", scheduleKind: "weekly", weekday: 5, weekdays: [5], timeOfDayMinutes: 16 * 60 }),
  rule({ automationRuleId: "paused", name: "材料检查", enabled: false }),
  rule({ automationRuleId: "attention", name: "异常巡检", lastError: "上次运行失败" }),
];

const snapshot = {
  stateRevision: 0,
  localAgentsEnabled: true,
  runtimes: [],
  models: [],
  agents: [{ agentId: "agent-1", name: "项目助手", description: "", instructions: "", provider: "codex", model: null, skillIds: [], toolPermissions: [], directoryScope: "workspace-read", networkAccess: false, concurrencyLimit: 1, createdAt: 1, updatedAt: 1 }],
  conversations: [],
  messages: [],
  runs: [{
    runId: "run-1", conversationId: "conversation-1", agentId: "agent-1", inputMessageId: "message-1", retryOfRunId: null,
    provider: "codex", model: null, status: "completed", pid: null, startedAt: 1, completedAt: 2, output: "完成", errorReason: null,
    sessionId: null, skillIds: [], toolPermissions: [], permissionRequestId: null, automationRuleId: "daily", automationRuleVersion: 2,
    sandboxMode: "read-only", events: [], artifacts: [], usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 }, logTail: [],
  }],
  permissionRequests: [],
  inboxReadItemIds: [],
  automationRules: rules,
  skills: [],
} as AiRuntimeSnapshot;

beforeEach(() => {
  modelApi.models = [];
  modelApi.fail = null;
  invoke.mockReset();
  invoke.mockImplementation((command: string) => {
    if (command === "ai_runtime_query") return Promise.resolve(snapshot);
    if (command === "ai_automation_trigger_history") return Promise.resolve({items:[],nextBeforeSequence:null});
    throw new Error(`unexpected command ${command}`);
  });
});

afterEach(cleanup);

describe("自动化列表筛选", () => {
  it("按名称、指令、工作区和排期搜索", () => {
    expect(filterAutomationRules(rules, "每天的项目", "all").map((item) => item.automationRuleId)).toEqual(["daily"]);
    expect(filterAutomationRules(rules, "每周五", "all").map((item) => item.automationRuleId)).toEqual(["weekly"]);
    expect(filterAutomationRules(rules, "项目工作区", "all")).toHaveLength(4);
  });

  it("状态互斥区分已启用、已暂停和需处理", () => {
    expect(filterAutomationRules(rules, "", "enabled").map((item) => item.automationRuleId)).toEqual(["daily", "weekly"]);
    expect(filterAutomationRules(rules, "", "paused").map((item) => item.automationRuleId)).toEqual(["paused"]);
    expect(filterAutomationRules(rules, "", "attention").map((item) => item.automationRuleId)).toEqual(["attention"]);
  });
});

describe("下次运行距离", () => {
  it("按分钟、小时和天显示未来时间", () => {
    const now = 1_000_000;
    expect(nextRunDistance(now + 20 * 60_000, now)).toBe("20 分钟后");
    expect(nextRunDistance(now + 3 * 3_600_000, now)).toBe("3 小时后");
    expect(nextRunDistance(now + 3 * 86_400_000, now)).toBe("3 天后");
  });

  it("未排期或已到时间时显示明确状态", () => {
    expect(nextRunDistance(null, 1_000_000)).toBe("未排期");
    expect(nextRunDistance(900_000, 1_000_000)).toBe("等待调度");
  });
});

describe("已安排任务页面", () => {
  it("删除需要确认，取消不改变任务，也不触发展开或编辑", async () => {
    render(<AutomationsPage project={null} workspace={workspace} onMessage={() => {}} onOpenRun={() => {}}/>);
    const remove = await screen.findByRole("button", { name: "删除每日项目摘要" });
    const edit = screen.getByRole("button", { name: "编辑每日项目摘要" });
    expect(remove.compareDocumentPosition(edit) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    fireEvent.click(remove);
    const dialog = within(screen.getByRole("dialog", { name: "删除“每日项目摘要”？" }));
    expect(dialog.getByText(/已有对话和运行记录保留/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "展开每日项目摘要" }).getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("dialog", { name: "编辑自动化" })).toBeNull();
    fireEvent.click(dialog.getByRole("button", { name: "取消" }));
    expect(screen.getByText("每日项目摘要")).toBeTruthy();
    expect(invoke.mock.calls.some(([command]) => command === "ai_automation_rule_delete")).toBe(false);
  });

  it.each(["每日项目摘要", "材料检查", "异常巡检"])("删除%s后更新列表和数量，其他任务仍保留", async (name) => {
    const onMessage = vi.fn();
    let current = snapshot;
    invoke.mockImplementation((command: string, args?: { request: { automationRuleId: string } }) => {
      if (command === "ai_runtime_query") return Promise.resolve(current);
      if (command === "ai_automation_rule_delete") {
        current = { ...current, automationRules: current.automationRules.filter((item) => item.automationRuleId !== args?.request.automationRuleId) };
        return Promise.resolve(null);
      }
      throw new Error(command);
    });
    const view = render(<AutomationsPage project={null} workspace={workspace} onMessage={onMessage} onOpenRun={() => {}}/>);
    fireEvent.click(await screen.findByRole("button", { name: `展开${name}` }));
    fireEvent.click(screen.getByRole("button", { name: `删除${name}` }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "删除" }));
    await waitFor(() => expect(screen.queryByText(name)).toBeNull());
    expect(screen.queryByRole("button", { name: "立即运行" })).toBeNull();
    expect(screen.getByText(/^3 条任务/)).toBeTruthy();
    expect(screen.getByText("周五进展回顾")).toBeTruthy();
    expect(current.runs).toEqual(snapshot.runs);
    expect(onMessage).toHaveBeenCalledWith("自动化已删除。");
    expect(invoke).toHaveBeenCalledWith("ai_automation_rule_delete", { request: { automationRuleId: rules.find((item) => item.name === name)?.automationRuleId } });
    view.unmount();
    render(<AutomationsPage project={null} workspace={workspace} onMessage={onMessage} onOpenRun={() => {}}/>);
    await screen.findByText("周五进展回顾");
    expect(screen.queryByText(name)).toBeNull();
  });

  it("提交期间阻止重复操作，失败保留原任务并允许重试", async () => {
    const onMessage = vi.fn();
    let rejectDelete!: (reason: Error) => void;
    const deletion = new Promise<void>((_resolve, reject) => { rejectDelete = reject; });
    invoke.mockImplementation((command: string) => {
      if (command === "ai_runtime_query") return Promise.resolve(snapshot);
      if (command === "ai_automation_rule_delete") return deletion;
      throw new Error(command);
    });
    render(<AutomationsPage project={null} workspace={workspace} onMessage={onMessage} onOpenRun={() => {}}/>);
    fireEvent.click(await screen.findByRole("button", { name: "展开每日项目摘要" }));
    fireEvent.click(screen.getByRole("button", { name: "删除每日项目摘要" }));
    const confirm = within(screen.getByRole("dialog")).getByRole("button", { name: "删除" });
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    expect(confirm).toHaveProperty("disabled", true);
    expect(screen.getByRole("button", { name: "编辑每日项目摘要" })).toHaveProperty("disabled", true);
    expect(screen.getByRole("button", { name: "立即运行" })).toHaveProperty("disabled", true);
    expect(invoke.mock.calls.filter(([command]) => command === "ai_automation_rule_delete")).toHaveLength(1);
    await act(async () => { rejectDelete(new Error("磁盘无法写入")); });
    expect(onMessage).toHaveBeenCalledWith("删除失败：磁盘无法写入", "error");
    expect(screen.getByText("每日项目摘要")).toBeTruthy();
    expect(screen.getByText(/^4 条任务/)).toBeTruthy();
    expect(within(screen.getByRole("dialog")).getByRole("button", { name: "删除" })).toHaveProperty("disabled", false);
    invoke.mockImplementation((command: string) => command === "ai_automation_rule_delete" ? Promise.resolve(null) : Promise.resolve(snapshot));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "删除" }));
    await waitFor(() => expect(screen.queryByText("每日项目摘要")).toBeNull());
  });

  it("无可用项目或智能体时仍能删除最后一条任务，保留搜索和筛选", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "ai_runtime_query") return Promise.resolve({ ...snapshot, localAgentsEnabled: false, agents: [], automationRules: [rules[2]] });
      if (command === "ai_automation_rule_delete") return Promise.resolve(null);
      throw new Error(command);
    });
    render(<AutomationsPage project={null} projects={[]} workspace={null} onMessage={() => {}} onOpenRun={() => {}}/>);
    await screen.findByText("材料检查");
    expect(screen.getByRole("button", { name: "新建自动化" })).toHaveProperty("disabled", true);
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "材料" } });
    fireEvent.click(screen.getByRole("button", { name: "已暂停" }));
    fireEvent.click(screen.getByRole("button", { name: "删除材料检查" }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "删除" }));
    await screen.findByText("还没有已安排任务");
    expect(screen.getByRole("searchbox")).toHaveProperty("value", "材料");
    expect(screen.getByRole("button", { name: "已暂停" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByText("0 条任务 · 0 条已启用")).toBeTruthy();
  });

  it("删除成功后忽略迟到的旧快照，任务不会重新出现", async () => {
    let resolveSnapshot!: (value: AiRuntimeSnapshot) => void;
    const stale = new Promise<AiRuntimeSnapshot>((resolve) => { resolveSnapshot = resolve; });
    let reads = 0;
    invoke.mockImplementation((command: string) => {
      if (command === "ai_runtime_query") return ++reads === 1 ? Promise.resolve(snapshot) : stale;
      if (command === "ai_automation_rule_delete") return Promise.resolve(null);
      throw new Error(command);
    });
    render(<AutomationsPage project={null} workspace={workspace} onMessage={() => {}} onOpenRun={() => {}}/>);
    await screen.findByText("每日项目摘要");
    act(() => { window.dispatchEvent(new Event(aiRuntimeChangedEvent)); });
    fireEvent.click(screen.getByRole("button", { name: "删除每日项目摘要" }));
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "删除" }));
    await waitFor(() => expect(screen.queryByText("每日项目摘要")).toBeNull());
    await act(async () => { resolveSnapshot(snapshot); });
    expect(screen.queryByText("每日项目摘要")).toBeNull();
    expect(screen.getByText(/^3 条任务/)).toBeTruthy();
  });

  it("编辑任务保留其所属项目，并将已有对话和独立模型一起保存", async () => {
    const project = { projectId: "project-1", displayName: "示例项目", primaryWorkspaceId: "ws-1", primaryWorkspaceName: "项目工作区", primaryWorkspaceRoot: "/tmp/ws-1", workspaceIds: ["ws-1"] };
    const conversations = [
      { conversationId: "same", title: "项目内对话", archived: false, context: { projectId: "project-1", workspaceId: "ws-1", workspaceName: "项目工作区", workspaceRoot: "/tmp/ws-1" } },
      { conversationId: "other", title: "其他项目对话", archived: false, context: { projectId: "project-2" } },
      { conversationId: "archived", title: "已归档对话", archived: true, context: { projectId: "project-1" } },
    ];
    invoke.mockImplementation((command: string, args: { request?: object }) => {
      if (command === "ai_runtime_query") return Promise.resolve({ ...snapshot, conversations, models: [{ modelId: "model-1", label: "Qwen 模型" }] });
      if (command === "ai_automation_rule_save") return Promise.resolve({ ...rules[0], ...args.request });
      throw new Error(command);
    });
    render(<AutomationsPage projects={[project]} project={{ projectId: "project-2", displayName: "当前其他项目" }} workspace={workspace} onMessage={() => {}} onOpenRun={() => {}}/>);
    fireEvent.click(await screen.findByRole("button", { name: "编辑每日项目摘要" }));
    expect(screen.getByLabelText("执行项目").textContent).toBe("示例项目");
    fireEvent.click(screen.getByLabelText("对话方式"));
    expect(screen.getByRole("option", { name: "新对话" })).toBeTruthy();
    fireEvent.click(screen.getByRole("option", { name: "已有对话" }));
    fireEvent.click(screen.getByLabelText("选择对话"));
    expect(screen.queryByRole("option", { name: "其他项目对话" })).toBeNull();
    expect(screen.queryByRole("option", { name: "已归档对话" })).toBeNull();
    fireEvent.click(screen.getByRole("option", { name: "项目内对话" }));
    fireEvent.click(screen.getByLabelText("模型"));
    fireEvent.click(screen.getByRole("option", { name: "Qwen 模型" }));
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_automation_rule_save", { request: expect.objectContaining({ projectId: "project-1", conversationId: "same", modelId: "model-1", workspaceRoot: "/tmp/ws-1" }) }));
    const payload = invoke.mock.calls.find(([command]) => command === "ai_automation_rule_save")?.[1].request;
    expect(payload).not.toHaveProperty("executionLocation");
    expect(payload).not.toHaveProperty("executionMode");
  });

  it("展示搜索、状态筛选、规则摘要和建议自动化", async () => {
    render(<AutomationsPage project={null} workspace={workspace} onMessage={() => {}} onOpenRun={() => {}}/>);
    expect(await screen.findByRole("heading", { name: "自动化" })).toBeTruthy();
    expect(screen.getByRole("searchbox", { name: "搜索已安排任务" })).toBeTruthy();
    expect(screen.getByRole("navigation", { name: "自动化状态筛选" })).toBeTruthy();
    expect(screen.getByText("4 条任务 · 3 条已启用 · 1 条需处理")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "建议自动化" })).toBeTruthy();
  });

  it("搜索与筛选会立即收窄任务列表", async () => {
    render(<AutomationsPage project={null} workspace={workspace} onMessage={() => {}} onOpenRun={() => {}}/>);
    await screen.findByText("每日项目摘要");
    fireEvent.change(screen.getByRole("searchbox", { name: "搜索已安排任务" }), { target: { value: "周五" } });
    expect(screen.getByText("周五进展回顾")).toBeTruthy();
    expect(screen.queryByText("每日项目摘要")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "清除搜索" }));
    fireEvent.click(screen.getByRole("button", { name: "已暂停" }));
    expect(screen.getByText("材料检查")).toBeTruthy();
    expect(screen.queryByText("周五进展回顾")).toBeNull();
  });

  it("展开任务后展示对话、运行入口和运行历史，编辑位于展开按钮左侧", async () => {
    const onOpenRun = vi.fn();
    render(<AutomationsPage project={null} workspace={workspace} onMessage={() => {}} onOpenRun={onOpenRun}/>);
    const list = within(await screen.findByRole("region", { name: "已安排任务列表" }));
    fireEvent.click(list.getByRole("button", { name: /^每日项目摘要/ }));
    expect(screen.getByText("汇总每天的项目变化")).toBeTruthy();
    expect(screen.getByText("新对话")).toBeTruthy();
    expect(screen.queryByText("规则版本")).toBeNull();
    const edit = list.getByRole("button", { name: "编辑每日项目摘要" });
    const toggle = list.getByRole("button", { name: "收起每日项目摘要" });
    expect(edit.compareDocumentPosition(toggle) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(list.queryByRole("button", { name: "编辑" })).toBeNull();
    fireEvent.click(edit);
    expect(screen.getByRole("dialog", { name: "编辑自动化" })).toBeTruthy();
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "关闭" }));
    expect(screen.getByRole("button", { name: "立即运行" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /运行已完成/ }));
    expect(onOpenRun).toHaveBeenCalledWith("run-1");
    fireEvent.click(toggle);
    expect(screen.queryByText("汇总每天的项目变化")).toBeNull();
  });

  it("建议项打开带预填内容的新建弹窗", async () => {
    render(<AutomationsPage project={null} workspace={workspace} onMessage={() => {}} onOpenRun={() => {}}/>);
    await screen.findByText("每日项目摘要");
    fireEvent.click(screen.getByRole("button", { name: /每周进展回顾/ }));
    const dialog = within(screen.getByRole("dialog", { name: "新建自动化" }));
    expect(dialog.getByLabelText("自动化名称")).toHaveProperty("value", "每周进展回顾");
    expect(dialog.getByLabelText("执行时间")).toHaveProperty("value", "16:00");
  });

  it("未连接工作区时仍给出明确边界提示", async () => {
    invoke.mockResolvedValue({ ...snapshot, automationRules: [] });
    render(<AutomationsPage project={null} workspace={null} onMessage={() => {}} onOpenRun={() => {}}/>);
    expect(await screen.findByText("连接项目文件夹后可新建自动化。")).toBeTruthy();
    expect(screen.getByRole("button", { name: "新建自动化" })).toHaveProperty("disabled", true);
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_runtime_query")).toBe(true));
  });
});


it("从完成工作创建自动化先核对草稿，保持停用且不自动保存或执行", async () => {
  const consumed = vi.fn();
  const intent = { token: "reuse-1", title: "复核周报", conversationId: "conversation-1", projectId: "project-1", agentId: "agent-1", modelId: null, input: "整理每周变更", output: "本周已完成验收" };
  render(<AutomationsPage reuseIntent={intent} onReuseConsumed={consumed} projects={[{ projectId: "project-1", displayName: "示例项目", primaryWorkspaceId: workspace.workspaceId, primaryWorkspaceName: workspace.displayName, primaryWorkspaceRoot: workspace.localRoot, workspaceIds: [workspace.workspaceId] }]} project={null} workspace={workspace} onMessage={() => {}} onOpenRun={() => {}}/>);
  const dialog = within(await screen.findByRole("dialog", { name: "新建自动化" }));
  expect((dialog.getByLabelText("自动化名称") as HTMLInputElement).value).toBe("复核周报");
  expect((dialog.getByLabelText("交给智能体执行的指令") as HTMLTextAreaElement).value).toContain("整理每周变更");
  expect(dialog.getByRole("switch", { name: "是否启用", checked: false })).toBeTruthy();
  expect(dialog.getByText(/本机 · 项目工作区/)).toBeTruthy();
  expect(dialog.getByText("停用规则或退出工作台")).toBeTruthy();
  expect(dialog.queryByText(/企业/)).toBeNull();
  expect(consumed).toHaveBeenCalledOnce();
  expect(invoke.mock.calls.every(([command]) => command === "ai_runtime_query")).toBe(true);
});

it("通知打开指定规则并清除筛选，不运行、保存或改为编辑状态",async()=>{
  const onMessage=vi.fn(),consumed=vi.fn(),onOpenRun=vi.fn();
  const view=render(<AutomationsPage project={null} workspace={workspace} onMessage={onMessage} onOpenRun={onOpenRun}/>);
  await screen.findByText("每日项目摘要");
  fireEvent.change(screen.getByRole("searchbox",{name:"搜索已安排任务"}),{target:{value:"周五"}});
  view.rerender(<AutomationsPage project={null} workspace={workspace} onMessage={onMessage} onOpenRun={onOpenRun}
    openIntent={{ruleId:"daily",token:"notification-one"}} onOpenIntentConsumed={consumed}/>);
  await screen.findByText("汇总每天的项目变化");
  expect((screen.getByRole("searchbox",{name:"搜索已安排任务"}) as HTMLInputElement).value).toBe("");
  expect(screen.getByRole("button",{name:"收起每日项目摘要"}).getAttribute("aria-expanded")).toBe("true");
  expect(screen.queryByRole("dialog")).toBeNull();expect(onOpenRun).not.toHaveBeenCalled();
  expect(invoke.mock.calls.every(call=>["ai_runtime_query","ai_automation_trigger_history"].includes(call[0]))).toBe(true);
  expect(invoke.mock.calls.filter(call=>call[0]==="ai_automation_trigger_history").every(call=>JSON.stringify(call[1])===JSON.stringify({request:{ruleId:"daily",limit:20,beforeSequence:null}}))).toBe(true);
  expect(consumed).toHaveBeenCalledWith("notification-one");
  view.rerender(<AutomationsPage project={null} workspace={workspace} onMessage={onMessage} onOpenRun={onOpenRun}
    openIntent={{ruleId:"removed",token:"notification-two"}} onOpenIntentConsumed={consumed}/>);
  await waitFor(()=>expect(onMessage).toHaveBeenCalledWith("自动化已不可用。","error"));
  expect(invoke.mock.calls.every(call=>["ai_runtime_query","ai_automation_trigger_history"].includes(call[0]))).toBe(true);
  expect(invoke.mock.calls.filter(call=>call[0]==="ai_automation_trigger_history").every(call=>JSON.stringify(call[1])===JSON.stringify({request:{ruleId:"daily",limit:20,beforeSequence:null}}))).toBe(true);
});
