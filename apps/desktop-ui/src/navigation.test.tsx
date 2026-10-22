import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { loadLocalProfile } from "./local-profile";

// Regression tests for the project navigation / restore rules that three review rounds found
// broken. They drive the real App against a mocked Tauri IPC boundary, because every one of
// those defects lived in the wiring between components rather than in an isolated function.
const invoke = vi.hoisted(() => vi.fn());
const eventListen = vi.hoisted(() => vi.fn());
const appWindow = vi.hoisted(() => ({
  close: vi.fn().mockResolvedValue(undefined),
  isMaximized: vi.fn().mockResolvedValue(false),
  minimize: vi.fn().mockResolvedValue(undefined),
  onResized: vi.fn().mockResolvedValue(() => {}),
  toggleMaximize: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("./host", () => ({
  invoke,
  listen: eventListen,
  hostWindow: appWindow,
}));
vi.mock("./local-profile", () => ({
  loadLocalProfile: vi.fn().mockResolvedValue({profileId:"a".repeat(32),displayName:"本地资料",schemaVersion:2,ready:true}),
}));

const navigationSession = vi.hoisted(() => ({sessionId:"navigation-session", revision:0, values:{} as Record<string, unknown>}));
vi.mock("./conversation-session-api", () => ({
  loadConversationSession: async () => structuredClone(navigationSession),
  patchConversationSession: async (_sessionId: string, changes: Array<{key:string;value:unknown}>) => {
    for (const change of changes) navigationSession.values[change.key] = change.value;
    navigationSession.revision += 1;
    return structuredClone(navigationSession);
  },
}));

const projectDetailStorageKey = "cilai.workbench.last-project-detail";
const workspaceStorageKey = "cilai.workbench.selected-workspace";
const selectedFileStorageKeyPrefix = "cilai.workbench.selected-file:";

const workspace = {
  workspaceId: "ws-1",
  workspaceKind: "local",
  displayName: "主工作区",
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

const project = {
  projectId: "proj-1",
  displayName: "示例项目",
  goal: "验证导航恢复",
  status: "active",
  primaryWorkspaceId: "ws-1",
  stateRevision: 1,
  updatedAt: 0,
};

function phase(phaseId: string, name: string, status: string, isMain: boolean) {
  return {
    phaseId,
    planId: "plan-1",
    projectId: "proj-1",
    name,
    goal: `${name}目标`,
    acceptanceCriteria: "验收标准",
    requiredDeliverableId: null,
    requiredDeliverableTitle: null,
    requiredDeliverableVersionId: null,
    sortOrder: 0,
    predecessorPhaseIds: [],
    isMain,
    status,
    stateRevision: 1,
    tasks: [],
  };
}

const plan = {
  planId: "plan-1",
  projectId: "proj-1",
  versionNo: 1,
  status: "active",
  summary: "计划摘要",
  stateRevision: 1,
  createdAt: 0,
  activatedAt: 0,
  phases: [phase("phase-1", "需求阶段", "in_progress", true), phase("phase-2", "设计阶段", "planned", false)],
};

/** Records every app_server_request method so tests can assert on refetch behaviour. */
let calledMethods: string[] = [];
let serverPhase: "running" | "starting" | "failed" = "running";
let runtimeInitializationError: string | null = null;
const defaultLocalProfileLoader = vi.mocked(loadLocalProfile).getMockImplementation()!;
let hostPlatform = "macos";

const emptyAiSnapshot = {
  stateRevision: 0,
  runtimes: [], models: [], agents: [], conversations: [], messages: [], runs: [],
  permissionRequests: [], inboxReadItemIds: [], automationRules: [], skills: [],
};

function appServerStatus() {
  const failed = serverPhase === "failed";
  return {
    phase: serverPhase, pid: failed ? null : 1, serverInstanceId: failed ? null : "inst-1", protocolVersion: failed ? null : "1", lastExitCode: failed ? 13 : null,
    errorReason: failed ? "unexpected_exit_code_13" : null,
    runtimeInitializationError,
    stderrTail: failed ? ["App Server failure: applied migration 29 does not match embedded name and checksum"] : [],
    unsolicitedMessageCount: 0, localIpcAvailable: !failed,
    connectionMode: failed ? null : "owned", serverLifecycleMode: failed ? null : "owned",
  };
}

function rpcResult(method: string): unknown {
  switch (method) {
    case "workspace/list": return { workspaces: [workspace] };
    case "workspace/files": return { files: [], workspaceRevision: 1, nextCursor: null, hasMore: false };
    case "workspace/listDirectory": return { workspaceId: "ws-1", relativePath: null, entries: [], issueCount: 0 };
    case "conflict/list": return { conflicts: [] };
    case "event/replay": return { events: [], nextSequence: 0, hasMore: false };
    case "project/get": return { project, workspaceIds: ["ws-1"] };
    case "project/list": return { projects: [project] };
    case "projectProgress/get": return { projectId: "proj-1", plans: [plan], tasks: plan.phases.flatMap((phase) => phase.tasks) };
    case "deliverable/list": return { deliverables: [] };
    case "attentionCase/list": return { cases: [] };
    case "projectPortfolio/list": return { items: [] };
    case "myWork/get": return { items: [], generatedAt: 0 };
    case "capabilityCreation/list": return { projects: [] };
    default: return {};
  }
}

beforeEach(() => {
  vi.mocked(loadLocalProfile).mockReset().mockImplementation(defaultLocalProfileLoader);
  navigationSession.values = {}; navigationSession.revision = 0;
  calledMethods = [];
  serverPhase = "running";
  runtimeInitializationError = null;
  hostPlatform = "macos";
  window.localStorage.clear();
  window.localStorage.setItem(workspaceStorageKey, "ws-1");
  eventListen.mockReset().mockResolvedValue(vi.fn());
  invoke.mockImplementation((command: string, args?: { method?: string }) => {
    if (command === "app_server_status") return Promise.resolve(appServerStatus());
    if (command === "app_server_restart") return Promise.resolve(appServerStatus());
    if (command === "host_platform") return Promise.resolve(hostPlatform);
    if (command === "ai_runtime_snapshot") return Promise.resolve(emptyAiSnapshot);
    if (["model_connection_list", "model_connection_change_list", "model_credential_cleanup_list", "model_purpose_list"].includes(command)) return Promise.resolve([]);
    if (command === "local_user_profile") return Promise.resolve({ displayName: "测试用户", username: "tester", accountMode: "本地账户" });
    if (command === "enterprise_capability_catalog") return Promise.resolve({ items: [] });
    if (command === "application_catalog_list") {
      calledMethods.push("applicationCatalog/list");
      return Promise.resolve({ catalogRevision: "catalog-1", items: [], localStates: [] });
    }
    if (command === "app_server_request") {
      const method = args?.method ?? "";
      calledMethods.push(method);
      return Promise.resolve(rpcResult(method));
    }
    return Promise.resolve(null);
  });
});

afterEach(() => {
  cleanup();
});

async function renderApp() {
  const { App } = await import("./main");
  return render(<App/>);
}

test("切换页面保留已读取的工作项、筛选和滚动位置", async () => {
  await renderApp();
  fireEvent.click(await screen.findByRole("button", { name: "工作台" }));
  const search = await screen.findByRole("searchbox", {name:"搜索个人工作项"});
  await waitFor(() => expect(screen.queryByText("正在加载任务…")).toBeNull());
  fireEvent.change(search, {target:{value:"保留筛选"}});
  const board = document.querySelector<HTMLElement>(".my-work-page")!;
  board.scrollTop = 150;
  fireEvent.click(screen.getByRole("button", {name:"项目"}));
  await screen.findByRole("searchbox", {name:"搜索项目"}, {timeout:5000});
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => args?.method === "myWork/get" ? new Promise(() => {}) : original(command,args));
  fireEvent.click(screen.getByRole("button", {name:"工作台"}));
  const restored = await screen.findByRole("searchbox", {name:"搜索个人工作项"});
  expect(restored).toBe(search);
  expect(restored).toHaveProperty("value","保留筛选");
  expect(document.querySelector(".my-work-page")).toBe(board);
  expect(board.scrollTop).toBe(150);
  expect(screen.queryByText("正在加载任务…")).toBeNull();
},15000);

test("Windows 对话与模型设置共享沙箱状态，切回不重复启动检查", async () => {
  hostPlatform = "windows";
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command,args) => {
    if (command === "ai_windows_sandbox_status") return Promise.resolve({supported:true,status:"ready",errorCode:null,errorMessage:null});
    if (command === "powershell_policy_check") return Promise.resolve({status:"configured",engines:[],changes:[],restoreChanges:[]});
    if (command === "ai_model_catalog_read") return Promise.resolve({profileId:"legacy",catalog:{generation:0,revision:1,status:{configured:false,models:[]},checkedAt:Date.now(),revalidateAfter:Date.now()+300000,error:null}});
    return original(command,args);
  });
  await renderApp();
  fireEvent.click(await screen.findByRole("button",{name:"对话"}));
  await waitFor(() => expect(invoke.mock.calls.filter(([command])=>command==="ai_windows_sandbox_status")).toHaveLength(1));
  fireEvent.click(screen.getByRole("button",{name:"工作台"}));
  await screen.findByRole("searchbox",{name:"搜索个人工作项"});
  fireEvent.click(screen.getByRole("button",{name:"对话"}));
  await act(async () => { await Promise.resolve(); });
  expect(invoke.mock.calls.filter(([command])=>command==="ai_windows_sandbox_status")).toHaveLength(1);
  fireEvent.click(screen.getByRole("button",{name:"用户：本地资料"}));
  fireEvent.click(await screen.findByRole("menuitem",{name:"设置"},{timeout:5000}));
  fireEvent.click(await screen.findByRole("button",{name:"模型设置"},{timeout:5000}));
  await screen.findByText("已初始化",{},{timeout:5000});
  fireEvent.click(screen.getByRole("button",{name:"诊断"}));
  fireEvent.click(screen.getByRole("button",{name:"模型设置"}));
  await screen.findByText("已初始化",{},{timeout:5000});
  expect(invoke.mock.calls.filter(([command])=>command==="ai_windows_sandbox_status")).toHaveLength(1);
},15000);

/** Waits until the restored project detail page is on screen. */
async function waitForProjectDetail() {
  // 冷启动还需加载项目页面，给并行原生构建时的懒加载留出等待时间。
  await waitFor(() => expect(screen.getByRole("heading", { name: "示例项目" })).toBeDefined(), { timeout: 5000 });
}

test.each(["完成", "取消", "切换页面", "迟到错误"])("首次打开未索引占位文件：%s", async (action) => {
  const original = invoke.getMockImplementation()!;
  let ready = false; let indexed = false;
  const item = { localFileId: "placeholder", relativePath: "占位.txt", fileType: "file", presenceState: "present", stateRevision: 1 };
  const reads: string[] = [];
  let failPending!: (error: Error) => void;
  invoke.mockImplementation((command, args) => {
    const method = args?.method;
    if (command === "app_server_request" && method === "workspace/listDirectory") return Promise.resolve({ workspaceId: "ws-1", relativePath: null, entries: [{ relativePath: "占位.txt", name: "占位.txt", fileType: "file", sizeBytes: 72, modifiedAt: 1 }], issueCount: 0 });
    if (command === "app_server_request" && method === "workspace/files") return Promise.resolve({ files: indexed ? [item] : [], workspaceRevision: 1, nextCursor: null, hasMore: false });
    if (command === "app_server_request" && method === "file/hydrate") {
      reads.push(args.params.cancel ? "cancel" : ready ? "ready" : "waiting");
      if (action === "迟到错误") return new Promise((_resolve, reject) => { failPending = reject; });
      return Promise.resolve({ state: args.params.cancel ? "cancelled" : ready ? "ready" : "downloading", operationId: "download-1" });
    }
    if (command === "app_server_request" && method === "workspace/indexPath") {
      expect(ready).toBe(true); reads.push("index"); indexed = true;
      return Promise.resolve({ localFileId: "placeholder" });
    }
    if (command === "preview_open") { reads.push("preview"); return Promise.resolve({ availability: "unsupported", format: "unsupported", source: "local" }); }
    return original(command, args);
  });
  await renderApp();
  fireEvent.click(await screen.findByRole("button", { name: "文件" }));
  // 首次文件页还要懒加载与读取目录，等待可观察的完成状态。
  fireEvent.click(await screen.findByText("占位.txt", {}, {timeout: 5000}));
  if (action === "迟到错误") {
    await waitFor(() => expect(reads).toEqual(["waiting"]));
    fireEvent.click(screen.getByRole("button", { name: "工作台" }));
    await act(async () => { failPending(new Error("file_hydration_download_failed")); });
    expect(reads).not.toContain("index");
    expect(screen.queryByText("操作未完成")).toBeNull();
    return;
  }
  await screen.findByText("正在下载");
  expect(reads).toEqual(["waiting"]);
  if (action === "完成") {
    ready = true;
    await waitFor(() => expect(reads).toContain("preview"));
    expect(reads.indexOf("ready")).toBeLessThan(reads.indexOf("index"));
    expect(reads.indexOf("index")).toBeLessThan(reads.indexOf("preview"));
  } else {
    fireEvent.click(screen.getByRole("button", { name: action === "取消" ? "取消" : "工作台" }));
    await waitFor(() => expect(reads).toContain("cancel"));
    expect(reads).not.toContain("index");
    expect(reads).not.toContain("preview");
    expect(screen.queryByText("操作未完成")).toBeNull();
  }
}, 15000);

test.each([["todo", "继续处理"], ["decision", "确认决策"], ["review", "查看成果"]])("工作台 %s 主操作进入准确对话并显示原消息", async (focusGroup, action) => {
  const targetConversation = {
    conversationId: "work-target", title: "任务原始对话", conversationType: "project", mode: "discuss", agentId: null, archived: false,
    context: { workspaceId: "ws-1", workspaceName: "主工作区", projectId: "proj-1", projectName: "示例项目", phaseId: null, phaseName: null, taskId: "task-1", taskName: "当前任务", deliverableId: null, deliverableName: null, localFileId: null, relativePath: null, fileRevision: null },
    workClassification: null, createdAt: 1, updatedAt: 2,
  };
  const navigationSnapshot = {
    ...emptyAiSnapshot,
    conversations: [targetConversation, { ...targetConversation, conversationId: "work-other", title: "项目内其他对话", updatedAt: 3 }],
    messages: [{ messageId: "work-message", conversationId: "work-target", role: "user", content: "这是当前任务的原始消息", attachments: [], runId: null, createdAt: 1 }],
    runs: [{ runId: "work-run", conversationId: "work-target", agentId: "agent-1", inputMessageId: "work-message", retryOfRunId: null, provider: "codex", model: null, status: "completed", startedAt: 1, completedAt: 2, output: "", errorReason: null, artifacts: [], events: [], logTail: [], usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 } }],
  };
  const originalInvoke = invoke.getMockImplementation()!;
  invoke.mockImplementation((command: string, args?: { method?: string }) => {
    if (command === "ai_runtime_snapshot") return Promise.resolve(navigationSnapshot);
    if (command === "app_server_request" && args?.method === "myWork/get") return Promise.resolve({ items: [{
      workItemId: "task:task-1", sourceType: "business_task", sourceId: "task-1", projectId: "proj-1", projectName: "示例项目", primaryWorkspaceId: "ws-1", phaseId: null, phaseName: null, taskId: "task-1", deliverableId: null, deliverableTitle: null, attentionCaseId: null, title: "当前任务", actionType: "task_execute", focusGroup, responsibilityType: "collaboration", responsibilityLabel: "AI 对话协作", priority: "medium", dueAt: null, sourceRevision: 1, updatedAt: 2,
    }], total: 1 });
    if (command === "app_server_request" && args?.method === "businessTask/get") return Promise.resolve({ task: { businessTaskId: "task-1" }, contributions: [{ runId: "work-run", conversationId: "work-target", conversationStatus: "active", stage: focusGroup, createdAt: 2 }] });
    return originalInvoke(command, args);
  });
  await renderApp();
  fireEvent.click(await screen.findByRole("button", { name: action }));
  expect(await screen.findByText("这是当前任务的原始消息")).toBeDefined();
  await waitFor(() => expect(window.localStorage.getItem("cilai.workbench.last-page")).toBe("ai"));
  expect(screen.queryByRole("button", { name: "关闭任务详情" })).toBeNull();
  expect(screen.queryByLabelText("处理决策")).toBeNull();
});

test("Windows 本地资料启动后可以最小化、最大化和关闭窗口", async () => {
  hostPlatform = "windows";
  vi.mocked(loadLocalProfile).mockResolvedValueOnce({profileId:"a".repeat(32),displayName:"本地资料",schemaVersion:2,ready:true});
  await renderApp();
  expect(await screen.findByRole("navigation", { name: "主导航" })).toBeDefined();
  fireEvent.click(await screen.findByRole("button", { name: "最小化" }));
  fireEvent.click(screen.getByRole("button", { name: "最大化" }));
  fireEvent.click(screen.getByRole("button", { name: "关闭" }));
  expect(appWindow.minimize).toHaveBeenCalled();
  expect(appWindow.toggleMaximize).toHaveBeenCalled();
  expect(appWindow.close).toHaveBeenCalled();
});

test("Windows 本地服务启动失败时仍保留窗口控制", async () => {
  hostPlatform = "windows";
  serverPhase = "failed";
  vi.mocked(loadLocalProfile).mockRejectedValueOnce(new Error("ai_runtime_not_initialized"));
  await renderApp();
  expect(await screen.findByRole("heading", { name: "本地服务启动失败" })).toBeDefined();
  expect(await screen.findByRole("button", { name: "最小化" })).toBeDefined();
  expect(screen.getByRole("button", { name: "最大化" })).toBeDefined();
  expect(screen.getByRole("button", { name: "关闭" })).toBeDefined();
});

test("服务发现失败保留具体校验原因和重启入口", async () => {
  hostPlatform = "windows";
  serverPhase = "failed";
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command: string, args?: { method?: string }) => command === "app_server_status"
    ? Promise.resolve({ ...appServerStatus(), lastExitCode: null, errorReason: "instance_discovery_failed", stderrTail: ["App Server failure: instance_discovery_failed: instance record does not match the requested environment and data directory"] })
    : original(command, args));
  vi.mocked(loadLocalProfile).mockRejectedValueOnce(new Error("App Server is not ready"));
  await renderApp();
  expect(await screen.findByRole("heading", { name: "本地服务启动失败" })).toBeDefined();
  expect(screen.getByText("instance_discovery_failed: instance record does not match the requested environment and data directory")).toBeDefined();
  fireEvent.click(screen.getByRole("button", { name: "重新启动" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("app_server_restart"));
});

test("本地服务失败时显示启动根因", async () => {
  serverPhase = "failed";
  vi.mocked(loadLocalProfile).mockRejectedValueOnce(new Error("ai_runtime_not_initialized"));

  await renderApp();

  expect(await screen.findByRole("heading", { name: "本地服务启动失败" })).toBeDefined();
  expect(screen.getByText("applied migration 29 does not match embedded name and checksum")).toBeDefined();
  expect(screen.getByText("退出码 13")).toBeDefined();
  expect(screen.queryByText("暂时无法完成登录")).toBeNull();
  expect(screen.queryByText(/ai_runtime_not_initialized/)).toBeNull();
});

test.each([
  "database migration history is newer than this App Server",
  "cilai_home_migration_failed:open_migration_lock:access denied",
])("启动断连不覆盖本地服务实际失败原因：%s", async (reason) => {
  serverPhase = "failed";
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command: string, args?: { method?: string }) => command === "app_server_status"
    ? Promise.resolve({ ...appServerStatus(), stderrTail: [`App Server failure: ${reason}`, "App Server failure: initialize_failed: App Server connection closed"] })
    : original(command, args));
  vi.mocked(loadLocalProfile).mockRejectedValueOnce(new Error("App Server is not ready"));
  await renderApp();
  expect(await screen.findByRole("heading", { name: "本地服务启动失败" })).toBeDefined();
  expect(screen.getByText(reason)).toBeDefined();
  expect(screen.queryByText("initialize_failed: App Server connection closed")).toBeNull();
});

test("历史初始化失败保留具体原因，阻止未就绪时读取资料并允许重启", async () => {
  hostPlatform = "windows";
  runtimeInitializationError = "备份历史会话失败：App Server request timed out: aiHistory/execute";
  await renderApp();
  expect(await screen.findByRole("heading", { name: "历史会话恢复失败" })).toBeDefined();
  expect(screen.getByText(runtimeInitializationError)).toBeDefined();
  expect(screen.queryByText("暂时无法完成登录")).toBeNull();
  expect(loadLocalProfile).not.toHaveBeenCalled();
  expect(await screen.findByRole("button", { name: "最小化" })).toBeDefined();
  fireEvent.click(screen.getByRole("button", { name: "重新启动" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("app_server_restart"));
});

test("重启完成历史初始化后重新读取资料，不保留之前的初始化错误", async () => {
  runtimeInitializationError = "读取历史会话失败";
  vi.mocked(loadLocalProfile).mockResolvedValueOnce({profileId:"a".repeat(32),displayName:"本地资料",schemaVersion:2,ready:true});
  await renderApp();
  await screen.findByRole("heading", { name: "历史会话恢复失败" });
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command: string, args?: { method?: string }) => {
    if (command === "app_server_restart") {
      runtimeInitializationError = null;
      return Promise.resolve(appServerStatus());
    }
    return original(command, args);
  });
  fireEvent.click(screen.getByRole("button", { name: "重新启动" }));
  expect(await screen.findByRole("navigation", { name: "主导航" })).toBeDefined();
  await waitFor(() => expect(loadLocalProfile).toHaveBeenCalledTimes(1));
  expect(screen.queryByText("读取历史会话失败")).toBeNull();
});

test("已进入工作台后服务重启且资料清单损坏时撤下旧界面并显示错误", async () => {
  await renderApp();
  await screen.findByRole("navigation", { name: "主导航" });
  vi.mocked(loadLocalProfile).mockRejectedValueOnce(new Error("local_profile_manifest_invalid"));
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command,args) => command === "app_server_status" ? Promise.resolve({...appServerStatus(),serverInstanceId:"restarted-personal"}) : original(command,args));
  expect(await screen.findByRole("heading", {name:"无法打开本地资料"}, {timeout:4000})).toBeDefined();
  expect(screen.getByRole("alert").textContent).toContain("local_profile_manifest_invalid");
  expect(screen.queryByRole("navigation", {name:"主导航"})).toBeNull();
});

test("历史正文校验失败不引导修复权限，仍保留错误与日志入口", async () => {
  hostPlatform = "windows";
  runtimeInitializationError = "native_history_content_mismatch:message-test";
  await renderApp();
  expect(await screen.findByRole("heading", { name: "历史会话恢复失败" })).toBeDefined();
  expect(screen.getByText(runtimeInitializationError)).toBeDefined();
  expect(screen.queryByRole("button", { name: "检查并修复启动权限" })).toBeNull();
  expect(screen.getByRole("button", { name: "导出诊断" })).toBeDefined();
  expect(screen.getByText(/历史消息内容校验未通过/)).toBeDefined();
});

test("历史线程索引缺失不引导修复权限，保留重启与日志入口", async () => {
  hostPlatform = "windows";
  runtimeInitializationError = "codex_thread_request_failed:thread/turns/list:thread not loaded: thread-test";
  await renderApp();
  expect(await screen.findByRole("heading", { name: "历史会话恢复失败" })).toBeDefined();
  expect(screen.queryByRole("button", { name: "检查并修复启动权限" })).toBeNull();
  expect(screen.getByRole("button", { name: "重新启动" })).toBeDefined();
  expect(screen.getByRole("button", { name: "导出诊断" })).toBeDefined();
});

test("历史读取被系统权限拒绝时仍可检查启动权限", async () => {
  hostPlatform = "windows";
  runtimeInitializationError = "读取历史会话失败：Access is denied (os error 5)";
  await renderApp();
  expect(await screen.findByRole("heading", { name: "历史会话恢复失败" })).toBeDefined();
  expect(screen.getByRole("button", { name: "检查并修复启动权限" })).toBeDefined();
});

describe("读不出来与这里是空的", () => {
  test("工作区读取失败时说明是读取失败，而不是劝用户新建一个", async () => {
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_status") return Promise.resolve(appServerStatus());
      if (command === "ai_runtime_snapshot") return Promise.resolve(emptyAiSnapshot);
      if (command === "app_server_request") {
        const method = args?.method ?? "";
        calledMethods.push(method);
        if (method === "workspace/list") return Promise.reject(new Error("本地服务没有响应"));
        return Promise.resolve(rpcResult(method));
      }
      return Promise.resolve(null);
    });

    const { container } = await renderApp();
    fireEvent.click(await screen.findByRole("button", { name: "文件" }));

    const failure = await waitFor(() => {
      const node = container.querySelector(".load-failure");
      expect(node).not.toBeNull();
      return node as HTMLElement;
    });
    expect(failure.textContent).toContain("读取失败");
    expect(failure.textContent).toContain("本地服务没有响应");
    // 用户的工作区好端端在，界面不该在这时候劝他新建一个。
    expect(container.querySelector(".empty-state")).toBeNull();
    expect(within(failure).getByRole("button", { name: "重试" })).toBeTruthy();
  });

  test("删除本地文件前先校验目标工作区写权限", async () => {
    const item = {
      localFileId: "file-1",
      relativePath: "方案.md",
      fileType: "file",
      presenceState: "present",
      localChangeState: "clean",
      confirmationState: "not_required",
      conflictState: "none",
      stateRevision: 1,
    };
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_status") return Promise.resolve(appServerStatus());
      if (command === "host_platform") return Promise.resolve(hostPlatform);
      if (command === "ai_runtime_snapshot") return Promise.resolve(emptyAiSnapshot);
      if (command === "local_user_profile") return Promise.resolve({ displayName: "测试用户", username: "tester", accountMode: "本地账户" });
      if (command === "enterprise_capability_catalog") return Promise.resolve({ items: [] });
      if (command === "application_catalog_list") return Promise.resolve({ catalogRevision: "catalog-1", items: [], localStates: [] });
      if (command === "app_server_request") {
        const method = args?.method ?? "";
        calledMethods.push(method);
        if (method === "workspace/files") return Promise.resolve({ files: [item], workspaceRevision: 1, nextCursor: null, hasMore: false });
        if (method === "workspace/listDirectory") return Promise.resolve({ workspaceId: "ws-1", relativePath: null, entries: [{ relativePath: "方案.md", name: "方案.md", fileType: "file", sizeBytes: 1, modifiedAt: 1 }], issueCount: 0 });
        return Promise.resolve(rpcResult(method));
      }
      return Promise.resolve(null);
    });

    await renderApp();
    await waitFor(() => expect(screen.getByText("方案.md")).toBeTruthy());
    fireEvent.contextMenu(screen.getByText("方案.md"));
    fireEvent.click(screen.getByRole("menuitem", { name: "删除文件" }));
    fireEvent.click(within(screen.getByRole("dialog", { name: "删除这个文件？" })).getByRole("button", { name: "删除文件" }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("workspace_delete_item", {
      workspaceId: "ws-1",
      localRoot: "/tmp/ws-1",
      relativePath: "方案.md",
    }));
    const targetValidation = invoke.mock.calls.findIndex(([command, args]) => command === "app_server_request" && args?.method === "workspace/targetValidate");
    const deletion = invoke.mock.calls.findIndex(([command]) => command === "workspace_delete_item");
    expect(calledMethods).not.toContain("conflict/list");
    expect(targetValidation).toBeGreaterThanOrEqual(0);
    expect(deletion).toBeGreaterThan(targetValidation);
    expect(invoke.mock.calls[targetValidation]?.[1]).toEqual({
      method: "workspace/targetValidate",
      params: { workspaceId: "ws-1", relativePath: "方案.md", requireWrite: true },
    });
  });
});

describe("提示分档", () => {
  test("失败提示走全局弹窗，而不是页面顶部的横幅", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_status") return Promise.resolve(appServerStatus());
      if (command === "ai_runtime_snapshot") return Promise.resolve(emptyAiSnapshot);
      if (command === "app_server_request") {
        const method = args?.method ?? "";
        calledMethods.push(method);
        // 读取工作区清单失败，是启动时最短的一条真实失败路径。
        if (method === "workspace/list") return Promise.reject(new Error("工作区读取失败"));
        return Promise.resolve(rpcResult(method));
      }
      return Promise.resolve(null);
    });

    const { container } = await renderApp();

    // 横幅贴在页面顶部，用户正盯着刚点的按钮，很容易整条错过：失败必须挡住界面。
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog.textContent).toContain("工作区读取失败");
    expect(container.querySelector(".notice")).toBeNull();
  });

  test("成功提示仍然只是角落里的浮层，不打断操作", async () => {
    const { container } = await renderApp();
    await waitFor(() => expect(calledMethods).toContain("workspace/list"));

    container.querySelector<HTMLButtonElement>('.explorer-actions button[aria-label="重新扫描"]')!.click();

    await waitFor(() => expect(container.querySelector(".notice")).not.toBeNull());
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  test("成功提示到点自己退场，不需要用户再点一次关闭", async () => {
    // 这条提示原先挂到下一条提示把它顶掉为止：一次扫描完成能在界面上留一整天。
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const { container } = await renderApp();
      await waitFor(() => expect(calledMethods).toContain("workspace/list"));

      container.querySelector<HTMLButtonElement>('.explorer-actions button[aria-label="重新扫描"]')!.click();
      await waitFor(() => expect(container.querySelector(".notice")).not.toBeNull());

      await act(async () => { await vi.advanceTimersByTimeAsync(4_000); });
      expect(container.querySelector(".notice")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  test("对话页给外壳打上标记，浮层提示得以避开右下角的发送按钮", async () => {
    // 浮层默认停在右下角，而对话页的右下角正是发送按钮：实测两者完全重叠，
    // 提示停留的这几秒里整页的主动作都点不到。位置差异由样式按这个标记给出。
    const { container } = await renderApp();
    await waitFor(() => expect(calledMethods).toContain("workspace/list"));
    expect(container.querySelector(".shell.ai-chat-shell")).toBeNull();

    container.querySelector<HTMLButtonElement>('.activity-nav button[aria-label="对话"]')!.click();
    await waitFor(() => expect(container.querySelector(".shell.ai-chat-shell")).not.toBeNull());

    container.querySelector<HTMLButtonElement>('.activity-nav button[aria-label="自动化"]')!.click();
    await waitFor(() => expect(container.querySelector(".shell.ai-chat-shell")).toBeNull());
  });
});

describe("对话页新增项目", () => {
  test("项目下拉的新增入口打开统一项目弹窗并保持在对话页", async () => {
    const { container } = await renderApp();
    await waitFor(() => expect(calledMethods).toContain("projectPortfolio/list"));

    fireEvent.click(await screen.findByRole("button", { name: "对话" }));
    fireEvent.click(await screen.findByLabelText("新建 AI 对话"));
    const selector = await screen.findByRole("button", { name: "选择对话项目：未分类" });
    fireEvent.click(selector);
    fireEvent.click(screen.getByRole("menuitem", { name: /新增项目/ }));

    const dialog = await screen.findByRole("dialog", { name: "新建项目" });
    expect(within(dialog).getByRole("heading", { name: "新建项目" })).toBeTruthy();
    expect(within(dialog).getByLabelText("项目名称")).toBeTruthy();
    expect(container.querySelector(".shell.ai-chat-shell")).not.toBeNull();
  });
});

test("文件页新增项目可以先规划再关联本地文件夹", async () => {
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command, args) => args?.method === "project/create" ? Promise.resolve({ project: { ...project, primaryWorkspaceId: null } }) : original(command,args));
  await renderApp();
  fireEvent.click(await screen.findByRole("button", { name: "文件" }));
  fireEvent.click(await screen.findByRole("button", { name: /本地 · 主工作区/ }));
  fireEvent.click(screen.getByRole("button", { name: "添加工作空间" }));
  fireEvent.click(await screen.findByRole("button", { name: "新建项目" }));
  const dialog = await screen.findByRole("dialog", { name: "新建项目" });
  fireEvent.change(within(dialog).getByLabelText("项目名称"), { target: { value: "新项目" } });
  fireEvent.change(within(dialog).getByLabelText("项目说明"), { target: { value: "整理材料" } });
  expect(within(dialog).queryByRole("button", { name: /云端/ })).toBeNull();
  fireEvent.click(within(dialog).getByRole("button", { name: "创建项目" }));
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "新建项目" })).toBeNull());
  expect(invoke.mock.calls.find(([,args]) => args?.method === "project/create")?.[1].params.workspaceIds).toEqual([]);
  expect(invoke.mock.calls.some(([,args]) => args?.method?.startsWith("cloudWorkspace/"))).toBe(false);
});

describe("应用重启恢复项目与阶段上下文", () => {
  test("启动时恢复到上次的项目推进视图并保留选中的阶段", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-2" }));

    await renderApp();
    await waitForProjectDetail();

    // The stored phase wins over the plan's default main phase.
    await waitFor(() => {
      const selected = document.querySelector(".phase-timeline button.selected");
      expect(selected?.textContent).toContain("设计阶段");
    });
  });

  test("重启后回到离开时所在的页面，而不是一律回到推进视图", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));
    window.localStorage.setItem("cilai.workbench.last-page", "mywork");

    const { container } = await renderApp();

    // 项目上下文恢复了（恢复记录还在），但页面停在用户离开时那个跨项目页面上。
    await waitFor(() => expect(container.querySelector(".my-work-toolbar")).not.toBeNull());
    expect(window.localStorage.getItem(projectDetailStorageKey)).not.toBeNull();
    expect(container.querySelector(".phase-timeline")).toBeNull();
  });

  test("没有恢复记录时停留在默认页面，不进入任何项目", async () => {
    await renderApp();

    await waitFor(() => expect(calledMethods).toContain("workspace/list"));
    expect(calledMethods).not.toContain("project/get");
  });

  test("旧版记录页恢复值不再打开已删除的模块", async () => {
    window.localStorage.setItem("cilai.workbench.last-page", "activity");

    await renderApp();

    await screen.findByRole("heading", { name: "工作台" });
    await waitFor(() => expect(window.localStorage.getItem("cilai.workbench.last-page")).toBe("mywork"));
    expect(screen.queryByRole("heading", { name: "变更历史" })).toBeNull();
  });

  test("恢复记录损坏时静默清除，不进入项目也不中断启动", async () => {
    window.localStorage.setItem(projectDetailStorageKey, "not-json");

    await renderApp();

    await waitFor(() => expect(calledMethods).toContain("workspace/list"));
    await waitFor(() => expect(window.localStorage.getItem(projectDetailStorageKey)).toBeNull());
    expect(calledMethods).not.toContain("project/get");
  });

  test("恢复的项目已失效时静默清除恢复记录", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "gone", phaseId: null }));
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_status") return Promise.resolve(appServerStatus());
      if (command === "ai_runtime_snapshot") return Promise.resolve(emptyAiSnapshot);
      if (command === "app_server_request") {
        const method = args?.method ?? "";
        calledMethods.push(method);
        if (method === "project/get") return Promise.reject(new Error("project_not_found"));
        return Promise.resolve(rpcResult(method));
      }
      return Promise.resolve(null);
    });

    await renderApp();

    await waitFor(() => expect(window.localStorage.getItem(projectDetailStorageKey)).toBeNull());
  });
});

describe("一级菜单选中态", () => {
  // Every page that remains in primary navigation must keep exactly that entry selected.
  // Management pages opened from the personal menu do not invent a primary-navigation owner.
  // 「运行」已不在一级导航：它只在有运行时才有内容，失败会作为关注事项汇入工作台，
  // 需要看原始运行记录时从工作台的事故条目进去。
  const pagesReachableFromNav = ["工作台", "对话", "项目", "文件", "自动化", "技能", "应用"];

  test("主导航以应用名称开头并按固定顺序展示菜单", async () => {
    const { container } = await renderApp();
    await waitFor(() => expect(calledMethods).toContain("workspace/list"));

    const header = container.querySelector<HTMLElement>(".activity-nav-header");
    expect(header).not.toBeNull();
    expect(within(header!).getByText("词来")).toBeTruthy();
    expect(within(header!).queryByText("词来工作台")).toBeNull();
    const brandMark = header!.querySelector<HTMLImageElement>(".activity-brand-mark");
    expect(brandMark).not.toBeNull();
    expect(brandMark!.getAttribute("src")).toMatch(/^(?:data:image\/svg\+xml|.*\.svg)/);
    expect(within(header!).getByRole("button", { name: "全局搜索" })).toBeTruthy();
    expect(container.querySelector(".activity-nav-group-label")).toBeNull();
    expect(Array.from(container.querySelectorAll('.activity-nav[aria-label="主导航"] > button'), (button) => button.getAttribute("aria-label"))).toEqual(pagesReachableFromNav);
    const panelToggle = screen.getByRole("button", { name: "展开主导航" });
    expect(panelToggle.querySelector("rect")?.getAttribute("width")).toBe("16");
    expect(panelToggle.querySelector("rect")?.getAttribute("height")).toBe("12");
    expect(panelToggle.querySelector("path")?.getAttribute("d")).toBe("M7 8.5v7");
    fireEvent.click(panelToggle);
    const collapseToggle = screen.getByRole("button", { name: "收起主导航" });
    expect(collapseToggle.querySelector("rect")?.getAttribute("width")).toBe("16");
    expect(collapseToggle.querySelector("rect")?.getAttribute("height")).toBe("12");
    expect(collapseToggle.querySelector("path")?.getAttribute("d")).toBe("M9 8.5v7");
  });

  test("macOS 显示并响应命令键搜索快捷键", async () => {
    await renderApp();
    const search = await screen.findByRole("button", { name: "全局搜索" }) as HTMLButtonElement;
    await waitFor(() => expect(search.title).toBe("全局搜索 (⌘K)"));

    fireEvent.keyDown(window, { key: "k", metaKey: true });

    expect(await screen.findByRole("heading", { name: "全局搜索" })).toBeTruthy();
    expect(screen.getByText("⌘K")).toBeTruthy();
  });

  test("Windows 显示并响应 Ctrl+K，不响应命令键", async () => {
    hostPlatform = "windows";
    await renderApp();
    const search = await screen.findByRole("button", { name: "全局搜索" }) as HTMLButtonElement;
    await waitFor(() => expect(search.title).toBe("全局搜索 (Ctrl+K)"));

    fireEvent.keyDown(window, { key: "k", metaKey: true });
    expect(search.getAttribute("aria-current")).toBeNull();
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });

    expect(await screen.findByRole("heading", { name: "全局搜索" })).toBeTruthy();
    expect(screen.getByText("Ctrl+K")).toBeTruthy();
  });

  test("未知平台不展示或响应系统快捷键", async () => {
    hostPlatform = "other";
    await renderApp();
    const search = await screen.findByRole("button", { name: "全局搜索" }) as HTMLButtonElement;
    await waitFor(() => expect(search.title).toBe("全局搜索"));

    fireEvent.keyDown(window, { key: "k", metaKey: true });
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });

    expect(search.getAttribute("aria-current")).toBeNull();
    expect(screen.queryByText("⌘K")).toBeNull();
    expect(screen.queryByText("Ctrl+K")).toBeNull();
  });

  test("应用入口位于主导航并可打开资料菜单", async () => {
    const { container } = await renderApp();
    await waitFor(() => expect(calledMethods).toContain("workspace/list"));

    const applicationNavigation = container.querySelector<HTMLElement>('.activity-nav[aria-label="主导航"]');
    const user = container.querySelector<HTMLElement>(".activity-user");
    expect(applicationNavigation).not.toBeNull();
    expect(user).not.toBeNull();

    within(applicationNavigation!).getByRole("button", { name: "应用" }).click();
    await waitFor(() => expect(calledMethods).toContain("capabilityCreation/list"));
    expect(screen.getByRole("heading", { name: "应用" })).toBeTruthy();
    expect(await screen.findByText("还没有应用")).toBeTruthy();
    expect(within(applicationNavigation!).getByRole("button", { name: "应用" }).classList.contains("active")).toBe(true);

    // 账户菜单在应用页也走正常布局：应用画面是界面里的元素，弹窗由层级决定先后，
    // 不再需要原生浮层那条绕路。
    fireEvent.click(screen.getByRole("button", { name: "用户：本地资料" }));
    await waitFor(() => expect(document.querySelector(".activity-user-menu")).not.toBeNull());
    fireEvent.click(screen.getByRole("button", { name: "用户：本地资料" }));
    await waitFor(() => expect(document.querySelector(".activity-user-menu")).toBeNull());
  });

  for (const label of pagesReachableFromNav) {
    test(`「${label}」打开后自身保持选中`, async () => {
      const { container } = await renderApp();
      await waitFor(() => expect(calledMethods).toContain("workspace/list"));

      const navButton = container.querySelector<HTMLButtonElement>(`.activity-nav button[aria-label="${label}"]`);
      expect(navButton).not.toBeNull();
      navButton!.click();

      await waitFor(() => {
        const active = container.querySelectorAll(".activity-nav button.active");
        expect(active.length).toBe(1);
        expect(active[0]?.getAttribute("aria-label")).toBe(label);
      });
    });
  }

  test("文件变更入口和变更历史页面均不再对用户提供", async () => {
    const { container } = await renderApp();
    await waitFor(() => expect(calledMethods).toContain("workspace/list"));

    expect(container.querySelector('.activity-nav button[aria-label="智能体"]')).toBeNull();
    expect(container.querySelector('.activity-nav button[aria-label="变更历史"]')).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "用户：本地资料" }));
    const menu = await screen.findByRole("menu", { name: "用户菜单" });
    expect(within(menu).queryByRole("menuitem", { name: "文件变更" })).toBeNull();
  });

  test("在项目里点「文件」后高亮跟到「文件」，不再留在「项目」上", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));

    const { container } = await renderApp();
    await waitForProjectDetail();

    container.querySelector<HTMLButtonElement>('.activity-nav button[aria-label="文件"]')!.click();

    await waitFor(() => {
      const active = container.querySelectorAll(".activity-nav button.active");
      expect(active.length).toBe(1);
      expect(active[0]?.getAttribute("aria-label")).toBe("文件");
    });
  });

  test("项目详情作为二级页面时「项目」保持选中", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));

    const { container } = await renderApp();
    await waitForProjectDetail();

    await waitFor(() => {
      const active = container.querySelectorAll(".activity-nav button.active");
      expect(active.length).toBe(1);
      expect(active[0]?.getAttribute("aria-label")).toBe("项目");
    });
  });
});

describe("项目上下文的保留与退出", () => {
  // Navigation now preserves the project: these pages either work inside it or sit above it,
  // and the breadcrumb keeps showing what the user would return to. Only 返回 ends it.
  const topLevelEntries = ["工作台", "对话", "项目", "文件", "自动化"];

  for (const label of topLevelEntries) {
    test(`点击一级导航「${label}」后保留项目上下文`, async () => {
      window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));

      const { container } = await renderApp();
      await waitForProjectDetail();

      const navButton = container.querySelector<HTMLButtonElement>(`.activity-nav button[aria-label="${label}"]`);
      expect(navButton).not.toBeNull();
      navButton!.click();

      // 项目上下文不再有常驻横条，恢复记录本身就是它没被这次导航丢掉的证据。
      await waitFor(() => expect(window.localStorage.getItem(projectDetailStorageKey)).not.toBeNull());
    });
  }

  test("点击标题行「返回」后清除恢复记录", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));

    const { container } = await renderApp();
    await waitForProjectDetail();
    await waitFor(() => expect(screen.queryByRole("button",{name:"返回项目列表"})).not.toBeNull());

    fireEvent.click(screen.getByRole("button",{name:"返回项目列表"}));

    await waitFor(() => expect(window.localStorage.getItem(projectDetailStorageKey)).toBeNull());
    expect(screen.queryByRole("button",{name:"返回项目列表"})).toBeNull();
  });

  test("点击全局搜索后保留项目上下文", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));

    const { container } = await renderApp();
    await waitForProjectDetail();

    container.querySelector<HTMLButtonElement>(".activity-nav-search")!.click();

    await waitFor(() => expect(window.localStorage.getItem(projectDetailStorageKey)).not.toBeNull());
    expect(screen.getByRole("heading", { name: "全局搜索" })).toBeTruthy();
    expect(screen.queryByText("GLOBAL SOURCE SEARCH")).toBeNull();
    expect(screen.queryByText("搜索本地可访问的项目、阶段、任务、交付物、决策和文件，并进入唯一来源对象。")).toBeNull();
    expect(container.querySelector(".global-search-body")).not.toBeNull();
  });

  test("从项目内离开去文件页不清除恢复记录", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));

    const { container } = await renderApp();
    await waitForProjectDetail();

    // 页面内的推进/文件切换已移除，同一条不变式改由侧栏入口验证：
    // 只有显式返回项目列表才清除恢复记录，普通导航不清除。
    const filesEntry = [...container.querySelectorAll<HTMLButtonElement>(".activity-nav button")]
      .find((button) => (button.getAttribute("aria-label") ?? button.textContent ?? "").includes("文件"));
    expect(filesEntry).toBeDefined();
    filesEntry!.click();

    await waitFor(() => expect(container.querySelector(".explorer-sidebar")).not.toBeNull());
    expect(container.querySelector(".app-header")).toBeNull();
    expect(window.localStorage.getItem(projectDetailStorageKey)).not.toBeNull();
  });

  test("项目身份解析进行中时的显式导航保住当前页面", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));
    let releaseProjectGet: (() => void) | null = null;
    invoke.mockImplementation((command: string, args?: { method?: string }) => {
      if (command === "app_server_status") return Promise.resolve(appServerStatus());
      if (command === "ai_runtime_snapshot") return Promise.resolve(emptyAiSnapshot);
      if (command === "app_server_request") {
        const method = args?.method ?? "";
        calledMethods.push(method);
        // Holds project/get open so the test can navigate while identity resolution is pending.
        if (method === "project/get") {
          return new Promise((resolve) => { releaseProjectGet = () => resolve(rpcResult(method)); });
        }
        return Promise.resolve(rpcResult(method));
      }
      return Promise.resolve(null);
    });

    const { container } = await renderApp();
    await waitFor(() => expect(calledMethods).toContain("project/get"));

    container.querySelector<HTMLButtonElement>('.activity-nav button[aria-label="工作台"]')!.click();
    await waitFor(() => expect(container.querySelector(".my-work-toolbar")).not.toBeNull());

    // The context is still restored (the breadcrumb needs it), but the page the user chose stands.
    await act(async () => { releaseProjectGet!(); });

    expect(container.querySelector(".phase-timeline")).toBeNull();
  });

  test("本地资料未就绪前保持加载，就绪后恢复原有项目", async () => {
    serverPhase = "starting";
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));
    await renderApp();
    expect(screen.queryByRole("navigation", {name:"主导航"})).toBeNull();
    expect(loadLocalProfile).not.toHaveBeenCalled();
    serverPhase = "running";
    await waitFor(() => expect(calledMethods).toContain("project/get"), { timeout: 5_000 });
    expect(await screen.findByRole("navigation", {name:"主导航"})).toBeTruthy();
    expect(invoke.mock.calls.some(([command]) => ["enterprise_login","enterprise_session_status"].includes(command))).toBe(false);
  });
});

describe("文件页选择恢复", () => {
  test("切换菜单和重启界面后恢复工作空间及其最后打开的文件", async () => {
    const rememberedWorkspace = { ...workspace, workspaceId: "ws-2", displayName: "记忆工作空间", localRoot: "/tmp/ws-2" };
    const workspaceFiles = {
      "ws-1": [{
        localFileId: "ws-1-file", relativePath: "默认.md", fileType: "file", presenceState: "present",
        localChangeState: "clean", confirmationState: "not_required", conflictState: "none", stateRevision: 1,
      }],
      "ws-2": [
        {
          localFileId: "remembered-first", relativePath: "先打开.md", fileType: "file", presenceState: "present",
          localChangeState: "clean", confirmationState: "not_required", conflictState: "none", stateRevision: 1,
        },
        {
          localFileId: "remembered-last", relativePath: "后打开.md", fileType: "file", presenceState: "present",
          localChangeState: "clean", confirmationState: "not_required", conflictState: "none", stateRevision: 1,
        },
      ],
    } as const;
    window.localStorage.setItem(workspaceStorageKey, rememberedWorkspace.workspaceId);
    window.localStorage.setItem(`${selectedFileStorageKeyPrefix}${rememberedWorkspace.workspaceId}`, "remembered-last");
    invoke.mockImplementation((command: string, args?: { method?: string; params?: { workspaceId?: string } }) => {
      if (command === "app_server_status") return Promise.resolve(appServerStatus());
      if (command === "host_platform") return Promise.resolve(hostPlatform);
      if (command === "ai_runtime_snapshot") return Promise.resolve(emptyAiSnapshot);
      if (command === "local_user_profile") return Promise.resolve({ displayName: "测试用户", username: "tester", accountMode: "本地账户" });
      if (command === "enterprise_capability_catalog") return Promise.resolve({ items: [] });
      if (command === "application_catalog_list") return Promise.resolve({ catalogRevision: "catalog-1", items: [], localStates: [] });
      if (command === "app_server_request") {
        const method = args?.method ?? "";
        calledMethods.push(method);
        if (method === "workspace/list") return Promise.resolve({ workspaces: [workspace, rememberedWorkspace] });
        if (method === "workspace/listDirectory") {
          const workspaceId = args?.params?.workspaceId as keyof typeof workspaceFiles;
          return Promise.resolve({ workspaceId, relativePath: null, entries: (workspaceFiles[workspaceId] ?? []).map((file) => ({ relativePath: file.relativePath, name: file.relativePath, fileType: "file", sizeBytes: 1, modifiedAt: 1 })), issueCount: 0 });
        }
        if (method === "workspace/files") {
          const workspaceId = args?.params?.workspaceId as keyof typeof workspaceFiles;
          return Promise.resolve({ files: workspaceFiles[workspaceId] ?? [], workspaceRevision: 1, nextCursor: null, hasMore: false });
        }
        return Promise.resolve(rpcResult(method));
      }
      return Promise.resolve(null);
    });

    const firstRun = await renderApp();
    fireEvent.click(await screen.findByRole("button", { name: "文件" }));
    await waitFor(() => {
      expect(firstRun.container.querySelector(".workspace-switcher-trigger small")?.textContent).toBe("本地 · 记忆工作空间");
      expect(firstRun.container.querySelector(".document-view-context strong")?.textContent).toBe("后打开.md");
    });

    fireEvent.click(screen.getByText("先打开.md"));
    await waitFor(() => expect(window.localStorage.getItem(`${selectedFileStorageKeyPrefix}${rememberedWorkspace.workspaceId}`)).toBe("remembered-first"));
    fireEvent.click(screen.getByRole("button", { name: "工作台" }));
    fireEvent.click(screen.getByRole("button", { name: "文件" }));
    expect(firstRun.container.querySelector(".document-view-context strong")?.textContent).toBe("先打开.md");

    firstRun.unmount();
    const secondRun = await renderApp();
    fireEvent.click(await screen.findByRole("button", { name: "文件" }));
    await waitFor(() => {
      expect(secondRun.container.querySelector(".workspace-switcher-trigger small")?.textContent).toBe("本地 · 记忆工作空间");
      expect(secondRun.container.querySelector(".document-view-context strong")?.textContent).toBe("先打开.md");
    });
  });
});

describe("文件列表宽度调整", () => {
  test("拖动分隔线连续调整宽度并保存", async () => {
    const { container } = await renderApp();
    await waitFor(() => expect(calledMethods).toContain("workspace/list"));
    container.querySelector<HTMLButtonElement>('.activity-nav button[aria-label="文件"]')!.click();

    const separator = await screen.findByRole("separator", { name: "调整文件列表宽度" });
    const shell = container.querySelector<HTMLElement>(".shell")!;
    expect(separator.getAttribute("aria-valuemin")).toBe("240");
    expect(separator.getAttribute("aria-valuemax")).toBe("520");
    expect(shell.style.getPropertyValue("--explorer-sidebar-width")).toBe("300px");

    fireEvent(separator, new MouseEvent("pointerdown", { bubbles: true, button: 0, clientX: 300 }));
    fireEvent(window, new MouseEvent("pointermove", { bubbles: true, clientX: 420 }));
    expect(separator.getAttribute("aria-valuenow")).toBe("420");
    expect(shell.style.getPropertyValue("--explorer-sidebar-width")).toBe("420px");
    fireEvent(window, new MouseEvent("pointerup", { bubbles: true }));
    expect(window.localStorage.getItem("cilai.workbench.explorer-width")).toBe("420");
  });

  test("恢复已保存宽度并支持键盘边界与双击复位", async () => {
    window.localStorage.setItem("cilai.workbench.explorer-width", "400");
    const { container } = await renderApp();
    await waitFor(() => expect(calledMethods).toContain("workspace/list"));
    container.querySelector<HTMLButtonElement>('.activity-nav button[aria-label="文件"]')!.click();

    const separator = await screen.findByRole("separator", { name: "调整文件列表宽度" });
    expect(separator.getAttribute("aria-valuenow")).toBe("400");
    fireEvent.keyDown(separator, { key: "Home" });
    expect(separator.getAttribute("aria-valuenow")).toBe("240");
    fireEvent.keyDown(separator, { key: "ArrowLeft" });
    expect(separator.getAttribute("aria-valuenow")).toBe("240");
    fireEvent.keyDown(separator, { key: "End" });
    expect(separator.getAttribute("aria-valuenow")).toBe("520");
    fireEvent.doubleClick(separator);
    expect(separator.getAttribute("aria-valuenow")).toBe("300");
    expect(window.localStorage.getItem("cilai.workbench.explorer-width")).toBe("300");
  });
});

describe("阶段切换不触发整页重新加载", () => {
  test("点击另一个阶段不会重新拉取项目数据", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));

    const { container } = await renderApp();
    await waitForProjectDetail();
    await waitFor(() => expect(container.querySelectorAll(".phase-timeline button").length).toBe(2));

    const countReloads = () => calledMethods.filter((method) =>
      method === "project/list" || method === "projectProgress/get" || method === "deliverable/list" || method === "attentionCase/list").length;
    const before = countReloads();

    const designPhase = [...container.querySelectorAll<HTMLButtonElement>(".phase-timeline button")]
      .find((button) => button.textContent?.includes("设计阶段"));
    expect(designPhase).toBeDefined();
    designPhase!.click();

    // Waiting on the persisted phase rather than on the .selected class matters: the class flips
    // synchronously, while the write-back into the shared project context — the thing that used
    // to re-trigger the whole load cascade — only lands a commit later. Asserting on the class
    // alone lets the regression slip through.
    await waitFor(() => {
      const stored = window.localStorage.getItem(projectDetailStorageKey);
      expect(stored && JSON.parse(stored).phaseId).toBe("phase-2");
    });
    expect(container.querySelector(".phase-timeline button.selected")?.textContent).toContain("设计阶段");
    expect(countReloads()).toBe(before);
  });

  test("阶段切换会写入恢复记录，供下次启动使用", async () => {
    window.localStorage.setItem(projectDetailStorageKey, JSON.stringify({ projectId: "proj-1", phaseId: "phase-1" }));

    const { container } = await renderApp();
    await waitForProjectDetail();
    await waitFor(() => expect(container.querySelectorAll(".phase-timeline button").length).toBe(2));

    const designPhase = [...container.querySelectorAll<HTMLButtonElement>(".phase-timeline button")]
      .find((button) => button.textContent?.includes("设计阶段"));
    designPhase!.click();

    await waitFor(() => {
      const stored = window.localStorage.getItem(projectDetailStorageKey);
      expect(stored && JSON.parse(stored).phaseId).toBe("phase-2");
    });
  });
});


test("文件栏新建按钮创建当前目录文件夹，云端入口只放在工作空间流程", async () => {
  await renderApp();
  fireEvent.click(await screen.findByRole("button", { name: "文件" }));
  await waitFor(() => expect((screen.getByRole("button", { name: "新建文件夹" }) as HTMLButtonElement).disabled).toBe(false));
  expect(screen.queryByRole("button", { name: "添加云端工作空间" })).toBeNull();
  expect(screen.queryByRole("button", { name: "连接本地目录" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "新建文件夹" }));
  expect(screen.getByRole("dialog", { name: "新建文件夹" })).toBeTruthy();
  expect(screen.getByText("位置：主工作区")).toBeTruthy();
  expect(invoke.mock.calls.some(([command]) => command === "workspace_pick_directory")).toBe(false);
});


test("归档成功提示查看定位详情，跨页撤销后会话恢复", async () => {
  const conversation = {
    conversationId: "archive-target", title: "归档交互测试", conversationType: "direct", mode: "discuss", agentId: null,
    context: { workspaceId: null, workspaceName: null, workspaceRoot: "", projectId: null, projectName: null, phaseId: null, phaseName: null, taskId: null, taskName: null, deliverableId: null, deliverableName: null, localFileId: null, relativePath: null, fileRevision: null },
    workClassification: null, archived: false, createdAt: 1, updatedAt: 1,
  };
  let state = { ...emptyAiSnapshot, stateRevision: 1, conversations: [conversation], messages: [{ messageId: "archive-message", conversationId: conversation.conversationId, role: "user", content: "归档前的完整正文", runId: null, createdAt: 1, attachments: [] }] };
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command: string, args?: { request?: { archived: boolean }; method?: string }) => {
    if (command === "ai_runtime_snapshot") return Promise.resolve(state);
    if (command === "ai_conversation_set_archived") {
      state = { ...state, stateRevision: state.stateRevision + 1, conversations: [{ ...state.conversations[0], archived: args!.request!.archived }] };
      return Promise.resolve({ stateRevision: state.stateRevision, conversations: state.conversations, changedConversationIds: [conversation.conversationId], failures: [] });
    }
    return original(command, args);
  });
  window.localStorage.setItem("cilai.workbench.last-page", "ai");
  window.localStorage.setItem("cilai.workbench.ai-last-viewed-conversation", conversation.conversationId);
  await renderApp();
  const archive = await screen.findByRole("button", { name: "归档会话：归档交互测试" });
  fireEvent.click(archive);
  await screen.findByText("已归档对话");
  const notice = document.querySelector(".archive-notice")! as HTMLElement;
  fireEvent.click(within(notice).getByRole("button", { name: "查看" }));
  await screen.findByRole("heading", { name: "归档交互测试" });
  expect(screen.getByText("归档前的完整正文")).toBeTruthy();
  fireEvent.click(within(notice).getByRole("button", { name: "撤销" }));
  await waitFor(() => expect(document.querySelector(".archive-notice")).toBeNull());
  expect(state.conversations[0].archived).toBe(false);
  expect(screen.queryByRole("heading", { name: "归档交互测试" })).toBeNull();
  expect(screen.getByText("已撤销归档")).toBeTruthy();
});
