import { ModelCatalogProvider } from "./use-model-catalog";
import { WindowsSandboxProvider } from "./use-windows-sandbox";
import { ConversationSessionProvider, flushConversationSessions } from "./conversation-session";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { AiContext, AiConversation, AiReasoningEffort, AiRuntimeSnapshot } from "./ai-runtime";
import type { SkillConversationCatalogItem } from "./generated/protocol";
import { type ComponentProps, useState } from "react";
import { ProjectCreatePanel } from "./project-create";
import type { MessageSink } from "./app-message";

// AI 工作台的可读性回归测试：会话以讨论主题而不是文件选择作为身份，
// 文件仍作为可移除上下文保留完整事实。展示层容易在后续改动中被无意还原，
// 所以既锁纯函数也锁渲染结果。
const sessionApi = vi.hoisted(() => ({ snapshot: { sessionId: "test-session", revision: 0, values: {} as Record<string, unknown> }, failNextPatch: null as Error | null }));
vi.mock("./conversation-session-api", () => ({
  loadConversationSession: async () => structuredClone(sessionApi.snapshot),
  patchConversationSession: async (_id: string, changes: Array<{ key: string; value: unknown }>) => {
    if (sessionApi.failNextPatch) { const error = sessionApi.failNextPatch; sessionApi.failNextPatch = null; throw error; }
    changes.forEach((change) => { sessionApi.snapshot.values[change.key] = change.value; });
    sessionApi.snapshot.revision += 1;
    return structuredClone(sessionApi.snapshot);
  },
}));
const invoke = vi.hoisted(() => vi.fn());
const modelApi = vi.hoisted(() => ({ models: [] as unknown[], fail: null as string | null }));
const writeText = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
const renderMermaid = vi.hoisted(() => vi.fn().mockResolvedValue({ svg: '<svg data-testid="conversation-mermaid"></svg>' }));
vi.mock("./host", () => ({
  listen: async (event: string, callback: (value: unknown) => void) => { const handler = (value: Event) => callback((value as CustomEvent).detail); window.addEventListener(event, handler); return () => window.removeEventListener(event, handler); },
  invoke: async (command: string, args: unknown) => {
    if (command === "workspace_permissions_check") return { status: "ready" };
    if (command === "ai_model_catalog_read") {
      await Promise.resolve();
      if (modelApi.fail) throw new Error(modelApi.fail);
      return { profileId: "test-profile", catalog: { generation: 0, revision: 1, checkedAt: Date.now(), revalidateAfter: Date.now() + 300_000, error: null, status: { configured: true, endpoint: "https://models.example.test/v1", models: modelApi.models } } };
    }
    const result = await invoke(command, args);
    if (command === "ai_runtime_snapshot" && result) { modelApi.models = result.models; return structuredClone(result); }
    return result;
  },
  writeClipboard: writeText,
}));
vi.mock("mermaid", () => ({ default: { initialize: vi.fn(), render: renderMermaid } }));

const { AiWorkbenchPage: AiWorkbenchPageImplementation, RunElapsedTime, adaptiveControlWidth, assistantMessageBody, conversationDisplayTitle, conversationFileArtifacts, conversationSummary, conversationTitleFromFirstMessage, conversationTopic, elideMiddle, fileNameFromPath, imageGenerationProgress, markdownWorkspaceFile, modelApprovalOptionsForHost, runDurationLabel, runFailureSummary, visibleRunProgress, workspaceContext } = await import("./ai-workbench");
const { readAiAttachment } = await import("./ai-runtime");

function AiWorkbenchPage(props: Omit<ComponentProps<typeof AiWorkbenchPageImplementation>, "hostPlatform"> & { hostPlatform?: string | null }) {
  const { hostPlatform = "macos", ...pageProps } = props;
  return <ModelCatalogProvider profileId="test-profile"><WindowsSandboxProvider scopeKey="test-profile"><ConversationSessionProvider hostPlatform={hostPlatform} onMessage={pageProps.onMessage}><AiWorkbenchPageImplementation {...pageProps} hostPlatform={hostPlatform}/></ConversationSessionProvider></WindowsSandboxProvider></ModelCatalogProvider>;
}

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
const secondWorkspace = { ...workspace, workspaceId: "ws-2", displayName: "第二工作区", localRoot: "/tmp/ws-2" };
const projects = [
  { projectId: "proj-1", displayName: "主项目", primaryWorkspaceId: workspace.workspaceId, primaryWorkspaceName: workspace.displayName, primaryWorkspaceRoot: workspace.localRoot, workspaceIds: [workspace.workspaceId] },
  { projectId: "proj-2", displayName: "第二项目", primaryWorkspaceId: secondWorkspace.workspaceId, primaryWorkspaceName: secondWorkspace.displayName, primaryWorkspaceRoot: secondWorkspace.localRoot, workspaceIds: [secondWorkspace.workspaceId] },
];
const defaultProjectIndex = new Map([
  [workspace.workspaceId, { projectId: projects[0].projectId, displayName: projects[0].displayName }],
  [secondWorkspace.workspaceId, { projectId: projects[1].projectId, displayName: projects[1].displayName }],
]);

const relativePath = "00项目总览/项目目录结构与产品分类.md";
const selectedFile = {
  localFileId: "file-1", relativePath, fileType: "file", presenceState: "present", displayState: null,
  baselineRelation: null, localChangeState: null, confirmationState: null, conflictState: null, stateRevision: 3,
  checksumAlgorithm: null, checksumValue: null, baselineVersionId: null,
};

/**
 * 打开某条排队消息的更多菜单并点"编辑消息"。
 *
 * 编辑不再直接摆在行上——它要把内容搬回输入框，是另起一次编辑，与"插进当前回复""不要了"
 * 不是一类操作，收在更多菜单里。菜单经传送门挂在文档根部，因此菜单项要从 screen 上取，
 * 在排队区里是找不到的。
 */
function openQueuedEdit(queue: HTMLElement, position = 1) {
  fireEvent.click(within(queue).getByRole("button", { name: `排队消息 ${position} 的更多操作` }));
  fireEvent.click(screen.getByRole("menuitem", { name: /编辑消息/ }));
}

function setComposerText(label: string, value: string) {
  const input = screen.getByLabelText(label);
  if (input instanceof HTMLTextAreaElement) {
    fireEvent.change(input, { target: { value } });
  } else {
    input.textContent = value;
    fireEvent.input(input);
  }
  return input;
}

function conversationContext(overrides: Partial<AiContext> = {}): AiContext {
  return {
    workspaceId: workspace.workspaceId,
    workspaceName: workspace.displayName,
    workspaceRoot: workspace.localRoot,
    projectId: null,
    projectName: null,
    phaseId: null,
    phaseName: null,
    taskId: null,
    taskName: null,
    deliverableId: null,
    deliverableName: null,
    localFileId: "file-1",
    relativePath,
    fileRevision: 3,
    ...overrides,
  };
}

function snapshot(context: AiContext, executionSupported = true, agentModel: string | null = null, runtimeCapabilities: string[] = []) {
  const models: AiRuntimeSnapshot["models"] = [
    { modelId: "Qwen3.6-35B-A3B-NVFP4", label: "Qwen3.6 35B A3B NVFP4", provider: "codex", description: "256K 上下文窗口", source: "personal", contextWindowTokens: 262_144, inputModalities: ["text", "image"] },
    { modelId: "Qwen3.8-27B-NVFP4", label: "Qwen3.8 27B NVFP4", provider: "codex", description: "512K 上下文窗口", source: "personal", contextWindowTokens: 524_288, inputModalities: ["text", "image"] },
  ];
  return {
    stateRevision: 0,
    models,
    agents: [{ agentId: "agent-1", name: "默认 Agent", description: "", instructions: "", provider: "codex", model: agentModel, skillIds: ["skill-summary"], toolPermissions: [], directoryScope: "workspace-read", networkAccess: false, concurrencyLimit: 1, createdAt: 0, updatedAt: 0 }],
    conversations: [{ conversationId: "conv-1", title: `讨论 ${relativePath}`, conversationType: "file" as const, mode: "discuss" as const, agentId: "agent-1", context, workClassification: null, archived: false, createdAt: 0, updatedAt: 0 }],
    messages: [],
    runs: [],
    permissionRequests: [],
    inboxReadItemIds: [],
    automationRules: [],
  };
}

const existingMessage: AiRuntimeSnapshot["messages"][number] = {
  messageId: "message-existing",
  conversationId: "conv-1",
  role: "user",
  content: "已有一条消息",
  attachments: [],
  runId: null,
  createdAt: 1,
};

test("模型资格读取被拒绝时历史仍可读，重试成功后恢复模型选择", async () => {
  modelApi.fail = "model_connection_disabled";
  await renderWorkbench(conversationContext(), { messages: [existingMessage], localAgentsEnabled: false });
  expect((await screen.findAllByText("已有一条消息")).length).toBeGreaterThan(0);
  expect(await screen.findByText("此连接已停用。")).toBeTruthy();
  expect(screen.queryByText("还没有对话")).toBeNull();
  expect(screen.queryByRole("button", { name: /正在加载模型/ })).toBeNull();
  expect(screen.queryByRole("button", { name: "计划模式" })).toBeNull();
  modelApi.fail = null;
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  await waitFor(() => expect(screen.queryByText("此连接已停用。")).toBeNull());
  fireEvent.click(screen.getByRole("button", { name: /选择模型：/ }));
  expect(await screen.findByRole("menuitemradio", { name: /Qwen3.8 27B/ })).toBeTruthy();
  expect(screen.getAllByText("已有一条消息").length).toBeGreaterThan(0);
});

function startResult(request: Record<string, unknown>) {
  const conversation = {
    conversationId: "conv-new",
    title: request.title,
    conversationType: request.conversationType,
    mode: request.mode,
    agentId: request.agentId,
    approvalMode: request.permissionMode,
    context: request.context,
    workClassification: null,
    archived: false,
    createdAt: 2,
    updatedAt: 2,
  } as unknown as AiConversation;
  const run = {
    runId: "run-new",
    conversationId: conversation.conversationId,
    agentId: request.agentId,
    inputMessageId: request.clientMessageId ?? "message-new",
    executionTarget: request.executionTarget,
    model: request.modelOverride,
    reasoningEffort: request.reasoningEffort,
    approvalMode: request.permissionMode,
    sandboxMode: request.permissionMode === "full-access"
      ? "danger-full-access"
      : request.permissionMode === "read-only"
        ? "read-only"
        : "workspace-write",
    status: "queued",
    startedAt: 2,
    events: [],
    usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 },
  } as unknown as AiRuntimeSnapshot["runs"][number];
  const message = {
    messageId: request.clientMessageId ?? "message-new",
    conversationId: conversation.conversationId,
    role: "user",
    content: request.content,
    attachments: [],
    runId: run.runId,
    createdAt: 2,
  } as unknown as AiRuntimeSnapshot["messages"][number];
  return { conversation, run, message };
}

async function renderWorkbench(context: AiContext = conversationContext(), overrides: Partial<{
  launchRequest: ComponentProps<typeof AiWorkbenchPageImplementation>["launchRequest"];
  projectIndex: Map<string, { projectId: string; displayName: string }>;
  projects: typeof projects;
  localAgentsEnabled: boolean;
  executionSupported: boolean;
  agentModel: string | null;
  runtimeCapabilities: string[];
  conversations: AiRuntimeSnapshot["conversations"];
  messages: AiRuntimeSnapshot["messages"];
  runs: AiRuntimeSnapshot["runs"];
  permissionRequests: AiRuntimeSnapshot["permissionRequests"];
  onMessage: MessageSink;
  onCreateProject: () => void;
  onOpenFile: (localFileId: string, workspaceId: string) => void;
  onOpenWorkspacePath: (relativePath: string, workspaceId: string) => Promise<boolean>;
  onSelectProject: (projectId: string) => void;
  files: (typeof selectedFile)[];
  hostPlatform: string | null;
  rememberConversation: boolean;
  workspaceOpenError: unknown;
  conversationSkills: SkillConversationCatalogItem[];
  generatedSkillResult: { skill: { skillId: string; name: string; description: string; automatic: boolean; stateRevision: number } };
  generatedSkillFailOnce: boolean;
  skillInstallations: import("./generated/protocol").SkillRunInstallation[];
  rememberModel: boolean;
}> = {}) {
  if (overrides.rememberConversation !== false) window.localStorage.setItem("cilai.workbench.ai-last-viewed-conversation", "conv-1");
  // 本地智能体下线后发送必须选模型；默认按员工上次选择的模型进入，需要验证未选择状态的用例显式关闭
  if (overrides.rememberModel !== false && !window.localStorage.getItem("cilai.workbench.ai-last-execution-selection")) {
    window.localStorage.setItem("cilai.workbench.ai-last-execution-selection", JSON.stringify({ agentId: "agent-1", modelId: "Qwen3.8-27B-NVFP4", reasoningEffort: null }));
  }
  let generatedSkillFailed = false;
  invoke.mockImplementation(async (command: string, args?: { request?: Record<string, unknown>; attachmentId?: string; method?: string }) => {
    if (command === "ai_runtime_snapshot") return { ...snapshot(context, overrides.executionSupported ?? true, overrides.agentModel ?? null, overrides.runtimeCapabilities ?? []), localAgentsEnabled: overrides.localAgentsEnabled ?? true, conversations: overrides.conversations ?? snapshot(context).conversations, messages: overrides.messages ?? [], runs: overrides.runs ?? [], permissionRequests: overrides.permissionRequests ?? [] };
    if (command === "ai_runtime_activity_snapshot") return { stateRevision: 0, conversations: overrides.conversations ?? snapshot(context).conversations, messages: overrides.messages ?? [], runs: overrides.runs ?? [], permissionRequests: overrides.permissionRequests ?? [], missingRunIds: [] };
    if (command === "app_server_request" && args?.method === "skill/runList") return { installations: overrides.skillInstallations ?? [] };
    if (command === "app_server_request" && args?.method === "skill/conversationCatalog") return { skills: overrides.conversationSkills ?? [] };
    if (command === "app_server_request" && (args?.method === "skill/save" || args?.method === "skill/importDirectory")) {
      if (overrides.generatedSkillFailOnce && !generatedSkillFailed) {
        generatedSkillFailed = true;
        throw new Error('{"reason":"skill_file_read_failed"}');
      }
      if (overrides.generatedSkillResult) return overrides.generatedSkillResult;
    }
    if (command === "application_catalog_list") return { catalogRevision: "catalog-empty", items: [] };
    if (command === "ai_windows_sandbox_status") return { supported: true, status: "ready", errorCode: null, errorMessage: null };
    if (command === "ai_conversation_set_archived") return { stateRevision: 1, conversations: [{ ...snapshot(context).conversations[0], archived: true }], changedConversationIds: ["conv-1"], failures: [] };
    if (command === "ai_conversation_update") return snapshot(context).conversations[0];
    if (command === "ai_message_submit") return {};
    if (command === "ai_message_regenerate") return overrides.runs?.at(-1) ?? {};
    if (command === "ai_message_steer") return overrides.runs?.[0] ?? {};
    if (command === "ai_permission_request_resolve") return overrides.permissionRequests?.[0] ?? {};
    if (command === "ai_attachment_create") return { attachmentId: "attachment-test", fileName: args?.request?.fileName, mimeType: args?.request?.mimeType, mediaType: String(args?.request?.mimeType).startsWith("video/") ? "video" : "image", sizeBytes: args?.request?.sizeBytes, localPath: "" };
    if (command === "ai_attachment_append") return {};
    if (command === "ai_attachment_discard") return undefined;
    if (command === "ai_attachment_read") return new Uint8Array([137, 80, 78, 71]).buffer;
    if (command === "ai_artifact_read_image") return new Uint8Array([137, 80, 78, 71]).buffer;
    if (command === "ai_artifact_open") return undefined;
    if (command === "ai_run_event_detail") return { runId: "run-process", itemId: "command-1", phase: "reply", kind: "command", status: "completed", toolName: "命令执行", operation: "列出目录", target: ".", durationMs: 126, parameters: '[{"type":"listFiles","path":"."}]', output: "report.md", exitCode: 0, error: null, truncated: false };
    if (command === "workspace_open_item") {
      if (overrides.workspaceOpenError) throw overrides.workspaceOpenError;
      return undefined;
    }
    if (command === "workspace_save_copy") return true;
    if (command === "workspace_copy_item_content") return undefined;
    throw new Error(`unexpected command ${command}`);
  });
  render(<AiWorkbenchPage workspace={workspace} projects={overrides.projects ?? projects} files={overrides.files ?? [selectedFile]} selectedFile={null} launchRequest={overrides.launchRequest ?? null} preferredConversationId={null} hostPlatform={overrides.hostPlatform} onMessage={overrides.onMessage ?? (() => { /* 消息条不参与本次断言 */ })} onOpenAgents={() => { /* 智能体页跳转不参与本次断言 */ }} onOpenFile={overrides.onOpenFile} onOpenWorkspacePath={overrides.onOpenWorkspacePath} canCreateProject onCreateProject={overrides.onCreateProject ?? (() => { /* 默认不检查新建 */ })} onSelectProject={overrides.onSelectProject ?? (() => { /* 默认不检查切换 */ })} projectIndex={overrides.projectIndex ?? new Map()}/>);
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_runtime_snapshot")).toBe(true));
  await act(async () => { await Promise.resolve(); });
  if (context.workspaceId === workspace.workspaceId) {
    await waitFor(() => expect(document.querySelector(".ai-conversation-group-header")).not.toBeNull());
  }
}

async function renderEmptyWorkbench(
  agents = snapshot(conversationContext()).agents,
  onOpenAgents: () => void = () => { /* 默认不检查页面跳转 */ },
  runs: AiRuntimeSnapshot["runs"] = [],
  hostPlatform: string | null = "macos",
  sandboxStatus: "ready" | "notConfigured" | "updateRequired" | "error" = "ready",
  models: AiRuntimeSnapshot["models"] = snapshot(conversationContext()).models,
  tenantId: string | null = null,
) {
  modelApi.models = models;
  let emptySnapshot = { ...snapshot(conversationContext()), agents, models, conversations: [], messages: [], runs } as unknown as AiRuntimeSnapshot;
  invoke.mockImplementation(async (command: string) => {
    if (command === "ai_runtime_snapshot") return emptySnapshot;
    if (command === "ai_windows_sandbox_status") return { supported: true, status: sandboxStatus, errorCode: null, errorMessage: null };
    if (command === "ai_conversation_update") return emptySnapshot.conversations[0];
    if (command === "ai_message_submit") return {};
    throw new Error(`unexpected command ${command}`);
  });
  render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[selectedFile]} selectedFile={selectedFile} launchRequest={null} preferredConversationId={null} profileId={tenantId} hostPlatform={hostPlatform} onMessage={() => { /* 消息条不参与本次断言 */ }} onOpenAgents={onOpenAgents} canCreateProject onCreateProject={() => { /* 默认不检查新建 */ }} onSelectProject={() => { /* 默认不检查切换 */ }} projectIndex={defaultProjectIndex}/>);
  await waitFor(() => expect(screen.getByRole("heading", { name: "Hi，今天有什么安排？" })).toBeTruthy());
  // 首页标题先于异步模型目录出现；等待选择器就绪后才能操作或断言恢复的选择。
  await waitFor(() => {
    const picker = screen.getByRole("button", { name: /^选择模型：/ });
    expect(picker.textContent).not.toContain("正在加载模型");
    expect(picker.hasAttribute("disabled")).toBe(false);
  });
  invoke.mockImplementation(async (command: string, args?: { request: Record<string, unknown> }) => {
    if (command === "ai_runtime_snapshot") return emptySnapshot;
    if (command === "ai_windows_sandbox_status") return { supported: true, status: sandboxStatus, errorCode: null, errorMessage: null };
    if (command === "ai_conversation_submit") {
      const started = startResult(args!.request);
      emptySnapshot = { ...emptySnapshot, conversations: [started.conversation], messages: [started.message], runs: [started.run] };
      return { status: "recorded", conversation: started.conversation, run: started.run };
    }
    if (command === "ai_conversation_update") return emptySnapshot.conversations[0];
    if (command === "ai_message_submit") return {};
    throw new Error(command);
  });
  fireEvent.click(screen.getByLabelText("新建 AI 对话"));
  fireEvent.click(screen.getByRole("button", { name: "选择对话项目：未分类" }));
  fireEvent.click(screen.getByRole("menuitemradio", { name: /主项目/ }));
}

function selectDefaultStarterAgent() {
  fireEvent.click(screen.getByRole("button", { name: /^选择模型：/ }));
  fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3\.8 27B NVFP4/ }));
}

beforeEach(() => {
  modelApi.models = snapshot(conversationContext()).models;
  modelApi.fail = null;
  sessionApi.snapshot = { sessionId: "test-session", revision: 0, values: {} };
  sessionApi.failNextPatch = null;
  window.localStorage.clear();
  HTMLElement.prototype.scrollTo = vi.fn();
  Object.defineProperty(Blob.prototype, "arrayBuffer", { configurable: true, value: async function arrayBuffer(this: Blob) { return new Uint8Array(this.size).buffer; } });
  Object.defineProperty(URL, "createObjectURL", { configurable: true, value: vi.fn(() => "blob:attachment-preview") });
  Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: vi.fn() });
});

afterEach(() => {
  cleanup();
  invoke.mockReset();
  writeText.mockClear();
  vi.unstubAllGlobals();
});

test("Windows 与平台未确认阶段不提供完全访问模式", () => {
  expect(modelApprovalOptionsForHost("windows").map((option) => option.value)).toEqual(["request", "auto-review"]);
  expect(modelApprovalOptionsForHost(null).map((option) => option.value)).toEqual(["request", "auto-review"]);
  expect(modelApprovalOptionsForHost(undefined).map((option) => option.value)).toEqual(["request", "auto-review"]);
  expect(modelApprovalOptionsForHost("other").map((option) => option.value)).toEqual(["request", "auto-review"]);
  expect(modelApprovalOptionsForHost("macos").map((option) => option.value)).toEqual(["request", "auto-review", "full-access"]);
});

test("Windows 沙箱失败摘要给出可执行处理方向", () => {
  expect(runFailureSummary("Windows 沙箱尚未初始化。请在设置中完成初始化后重试。")).toBe("请先初始化 Windows 安全沙箱");
  expect(runFailureSummary("Windows 沙箱需要更新。请在设置中重新初始化后重试。")).toBe("请先更新 Windows 安全沙箱");
  expect(runFailureSummary("Windows 沙箱运行组件不完整。请重新安装词来工作台后重试。")).toBe("Windows 安全沙箱组件不完整");
  expect(runFailureSummary("无法确认 Windows 沙箱状态。请在设置中重试初始化。")).toBe("请检查 Windows 安全沙箱");
});

test("模型公共错误摘要区分服务、容量和租户额度", () => {
  expect(runFailureSummary('模型 Harness 执行失败：codex_harness_turn_failed:{"error":{"code":"MODEL_REQUEST_REJECTED"}}')).toBe("模型服务拒绝了请求");
  expect(runFailureSummary("模型服务拒绝了本次请求（HTTP 400，参数：tool_choice），请检查接口与参数兼容性。\n错误代码：MODEL_REQUEST_REJECTED")).toBe("模型服务拒绝了请求");
  expect(runFailureSummary("模型服务余额或额度不足，请查看该服务的用量与账单。\n错误代码：MODEL_REQUEST_REJECTED")).toBe("模型额度不足");
  expect(runFailureSummary("模型服务暂时不可用\n错误代码：AI-SVC-001")).toBe("模型服务暂时不可用");
  expect(runFailureSummary("模型服务繁忙\n错误代码：AI-SVC-002")).toBe("模型服务繁忙");
  expect(runFailureSummary("模型额度不足\n错误代码：AI-QUOTA-001")).toBe("模型额度不足");
  expect(runFailureSummary("企业额度账尚未就绪\n错误代码：AI-QUOTA-002")).toBe("企业额度账尚未就绪");
  expect(runFailureSummary("无权使用此模型\n错误代码：AI-AUTH-002")).toBe("无权使用此模型");
  expect(runFailureSummary("本次运行凭据已失效\n错误代码：AI-AUTH-001")).toBe("本次运行凭据已失效");
  expect(runFailureSummary("工作台登录已失效，请重新登录后继续。\n错误代码：AI-AUTH-001")).toBe("请重新登录");
  expect(runFailureSummary("输入内容超过模型限制：当前对话内容超过所选模型可处理的范围。请新建对话继续，或换用上下文更大的模型；如只是本次输入过长，缩短后重试。\n错误代码：AI-REQ-003")).toBe("输入内容超过模型限制");
  expect(runFailureSummary("消息接收状态待核实：已暂停发送，避免重复执行。\n错误代码：message_delivery_uncertain")).toBe("消息接收状态待核实");
  expect(runFailureSummary("对话恢复失败：对话绑定的模型线程记录无效，已保留原记录。\n错误代码：thread_binding_invalid")).toBe("对话恢复失败");
  expect(runFailureSummary("无权执行此操作：当前账号没有执行此操作的权限，请联系企业管理员。\n错误代码：AI-AUTH-003")).toBe("无权执行此操作");
  // 网关拒绝的文案里恰好含“权限”二字，不能当成需要员工确认的权限请求
  expect(runFailureSummary("模型 Harness 执行失败：codex_harness_turn_failed:unexpected status 403 Forbidden: 当前账号未获得此模型的使用权限，请联系企业管理员。, url: http://127.0.0.1:63925/v1/responses, request id: 543d127a")).toBe("运行未能完成");
});

describe("对话页默认位置", () => {
  function renderConversationHistory() {
    const context = conversationContext();
    const conversations = [
      { ...snapshot(context).conversations[0], conversationId: "conv-old", title: "上次查看的对话", updatedAt: 1 },
      { ...snapshot(context).conversations[0], conversationId: "conv-newer", title: "最近更新的对话", updatedAt: 2 },
    ];
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return { ...snapshot(context), conversations };
      throw new Error(`unexpected command ${command}`);
    });
    render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[]} selectedFile={null} launchRequest={null} preferredConversationId={null}
      onMessage={() => {}} onOpenAgents={() => {}} canCreateProject onCreateProject={() => {}} onSelectProject={() => {}} projectIndex={defaultProjectIndex}/>);
  }

  test("优先恢复用户上次实际查看的对话，而不是最近更新的对话", async () => {
    window.localStorage.setItem("cilai.workbench.ai-last-viewed-conversation", "conv-old");
    renderConversationHistory();

    await waitFor(() => expect(document.querySelector(".ai-conversation-open[aria-current='true'] strong")?.textContent).toBe("上次查看的对话"));
    fireEvent.click(document.querySelector(".ai-conversation-open[title='最近更新的对话']") as HTMLButtonElement);
    await waitFor(() => expect(window.localStorage.getItem("cilai.workbench.ai-last-viewed-conversation")).toBe("conv-newer"));
  });

  test("没有查看记录时进入不选项目的新对话", async () => {
    renderConversationHistory();

    expect(await screen.findByRole("heading", { name: "Hi，今天有什么安排？" })).toBeTruthy();
    expect(await screen.findByRole("button", { name: "选择对话项目：未分类" })).toBeTruthy();
    expect(document.querySelector(".ai-conversation-open[aria-current='true']")).toBeNull();
  });
});

describe("fileNameFromPath", () => {
  test("多级路径只保留最后一段文件名", () => {
    expect(fileNameFromPath(relativePath)).toBe("项目目录结构与产品分类.md");
  });

  test("没有斜杠时原样返回", () => {
    expect(fileNameFromPath("产品分类.md")).toBe("产品分类.md");
  });

  test("以斜杠结尾时返回最后一个非空段", () => {
    expect(fileNameFromPath("00项目总览/02需求/")).toBe("02需求");
  });

  test("空字符串返回空字符串", () => {
    expect(fileNameFromPath("")).toBe("");
  });

  test("只有分隔符时原样返回，不会退化成空值", () => {
    expect(fileNameFromPath("/")).toBe("/");
    expect(fileNameFromPath("///")).toBe("///");
  });

  test("绝对路径同样只保留文件名", () => {
    expect(fileNameFromPath("/tmp/ws-1/notes.md")).toBe("notes.md");
  });
});

describe("附件二进制读取", () => {
  test("将 IPC 字节数组规范化为可生成图片 Blob 的二进制", async () => {
    invoke.mockResolvedValue([137, 80, 78, 71]);

    const bytes = await readAiAttachment("attachment-image");

    expect(bytes).toBeInstanceOf(ArrayBuffer);
    expect(Array.from(new Uint8Array(bytes))).toEqual([137, 80, 78, 71]);
  });
});

describe("assistantMessageBody", () => {
  test("正文后的当前归类数据不显示，普通 JSON 示例保持原样", () => {
    const nonce = "classification-current";
    const envelope = JSON.stringify({ schemaVersion: 1, nonce, reply: "图片已生成", decision: "keep" });
    expect(assistantMessageBody(`图片已生成\n\n${envelope}`, nonce)).toBe("图片已生成");
    expect(assistantMessageBody(`图片已生成\n\n\u0060\u0060\u0060json\n${envelope}`, nonce)).toBe("图片已生成");
    expect(assistantMessageBody(envelope, nonce)).toBe("图片已生成");
    const example = `JSON 示例\n${envelope.replace(nonce, "example")}`;
    expect(assistantMessageBody(example, nonce)).toBe(example);
    expect(assistantMessageBody(`JSON 示例\n${envelope}`)).toBe(`JSON 示例\n${envelope}`);
  });
  test("旧版模型响应只显示用户正文", () => {
    const legacyResponse = JSON.stringify({
      classification: { action: "unclassified", taskAction: "none" },
      nonce: "classification-old",
      reply: "这是完整正文。",
      schemaVersion: 1,
    });
    expect(assistantMessageBody(legacyResponse)).toBe("这是完整正文。");
  });

  test("普通正文和用户要求展示的其他 JSON 保持原样", () => {
    expect(assistantMessageBody("普通 Markdown 回复")).toBe("普通 Markdown 回复");
    expect(assistantMessageBody('{"reply":"示例","kind":"user-data"}')).toBe('{"reply":"示例","kind":"user-data"}');
  });

  test("AI 回复渲染 GFM 表格和 Mermaid 图表", async () => {
    const markdown = "| 模块 | 状态 |\n| --- | --- |\n| 词来 | 完成 |\n\n- [x] 已检查\n\n```mermaid\ngraph LR\nA-->B\n```";
    const assistantMessage: AiRuntimeSnapshot["messages"][number] = {
      messageId: "message-markdown",
      conversationId: "conv-1",
      role: "assistant",
      content: markdown,
      attachments: [],
      runId: null,
      createdAt: 2,
    };

    await renderWorkbench(conversationContext(), { messages: [assistantMessage] });

    expect(screen.getByRole("table")).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "模块" })).toBeTruthy();
    expect((screen.getByRole("checkbox") as HTMLInputElement).checked).toBe(true);
    await waitFor(() => expect(screen.getByTestId("conversation-mermaid")).toBeTruthy());
    expect(renderMermaid).toHaveBeenCalledWith(expect.stringMatching(/^conversation-mermaid-/), "graph LR\nA-->B");
  });
});

describe("conversationDisplayTitle", () => {
  test("标题内的路径压缩为文件名，前缀词保留", () => {
    expect(conversationDisplayTitle(`讨论 ${relativePath}`)).toBe("讨论 项目目录结构与产品分类.md");
  });

  test("不含路径的标题原样返回", () => {
    expect(conversationDisplayTitle("示例项目 · 需求阶段 编排")).toBe("示例项目 · 需求阶段 编排");
  });

  test("空标题返回空字符串", () => {
    expect(conversationDisplayTitle("")).toBe("");
  });

  test("以斜杠结尾的目录标题保留最后一段目录名", () => {
    expect(conversationDisplayTitle("讨论 00项目总览/02需求/")).toBe("讨论 02需求");
  });

  test("一条标题里的多个路径分别压缩", () => {
    expect(conversationDisplayTitle("对比 a/b/left.md 与 c/d/right.md")).toBe("对比 left.md 与 right.md");
  });

  test("保留原有空白结构，不做额外裁剪", () => {
    expect(conversationDisplayTitle("讨论  a/b.md ")).toBe("讨论  b.md ");
  });
});

describe("conversationTitleFromFirstMessage", () => {
  test("使用首个非空行命名并去掉 Markdown 标记", () => {
    expect(conversationTitleFromFirstMessage("\n  ## 梳理本周项目安排  \n补充说明")).toBe("梳理本周项目安排");
  });

  test("只有附件时使用附件名称命名", () => {
    expect(conversationTitleFromFirstMessage("", "需求说明.md")).toBe("需求说明.md");
  });
});

describe("adaptiveControlWidth", () => {
  test("按当前模型或智能体名称宽度伸缩并限制上限", () => {
    expect(adaptiveControlWidth("gpt-5.4", 8, 28)).toBe("8ch");
    expect(adaptiveControlWidth("长模型名称", 8, 28)).toBe("11ch");
    expect(adaptiveControlWidth("x".repeat(80), 8, 28)).toBe("28ch");
  });
});

describe("运行中消息", () => {
  test("多个并行运行分批刷新，不遗漏超出单次上限的运行", async () => {
    const runs: AiRuntimeSnapshot["runs"] = Array.from({ length: 33 }, (_, index) => ({
      runId: `run-${index}`, conversationId: "conv-1", agentId: "agent-1", inputMessageId: "message-existing",
      retryOfRunId: null, provider: "codex", model: null, executionTarget: "agent",
      status: "running", pid: null, startedAt: Date.now(), completedAt: null, output: null,
      partialOutput: null, errorReason: null, sessionId: null, skillIds: [], toolPermissions: [],
      permissionRequestId: null, automationRuleId: null, automationRuleVersion: null,
      sandboxMode: "read-only", events: [], artifacts: [],
      usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 }, logTail: [],
    }));
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs });
    invoke.mockClear();
    await waitFor(() => {
      const batches = invoke.mock.calls
        .filter(([command]) => command === "ai_runtime_activity_snapshot")
        .map(([, args]) => args.runIds.length);
      expect(batches).toContain(32);
      expect(batches).toContain(1);
    });
  });

  test("运行期间只读取活跃运行，结束时再同步完整对话", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      runId: "run-activity", conversationId: "conv-1", agentId: "agent-1", inputMessageId: "message-existing",
      retryOfRunId: null, provider: "codex", model: "Qwen3.8-27B-NVFP4", executionTarget: "model",
      status: "running", pid: 123, startedAt: Date.now(), completedAt: null, output: null,
      partialOutput: "第一段", errorReason: null, sessionId: null, skillIds: [], toolPermissions: [],
      permissionRequestId: null, automationRuleId: null, automationRuleVersion: null,
      sandboxMode: "read-only", events: [], artifacts: [],
      usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 }, logTail: [],
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [activeRun] });
    invoke.mockClear();
    let completed = false;
    const finished = { ...activeRun, status: "completed" as const, completedAt: Date.now(), partialOutput: null, output: "完成" };
    const reply = { messageId: "activity-reply", conversationId: "conv-1", role: "assistant" as const, content: "完成", attachments: [], runId: activeRun.runId, createdAt: Date.now() };
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_activity_snapshot") return { stateRevision: 0, conversations: [], messages: completed ? [reply] : [], runs: [completed ? finished : activeRun], permissionRequests: [], missingRunIds: [] };
      if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages: [existingMessage, reply], runs: [finished] };
      throw new Error(`unexpected command ${command}`);
    });
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_runtime_activity_snapshot")).toBe(true));
    expect(invoke.mock.calls.some(([command]) => command === "ai_runtime_snapshot")).toBe(false);
    completed = true;
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_runtime_snapshot")).toBe(true));
    expect(screen.getAllByText("完成").length).toBeGreaterThan(0);
  });

  test("使用最新可读事件并隐藏 Harness 实现术语", () => {
    const run = {
      status: "running",
      events: [
        { sequence: 1, eventType: "harness.started", summary: "模型 Harness 已启动", createdAt: 1 },
        { sequence: 2, eventType: "item.started", summary: "模型开始分析", createdAt: 2 },
      ],
    } as AiRuntimeSnapshot["runs"][number];
    expect(visibleRunProgress(run)).toBe("正在分析…");
  });

  test("尚无可读事件时显示中性状态", () => {
    const run = { status: "running", events: [] } as unknown as AiRuntimeSnapshot["runs"][number];
    expect(visibleRunProgress(run)).toBe("正在处理…");
  });

  test("通用阶段和具体活动均按真实事件展示", () => {
    const genericRun = {
      status: "running",
      events: [{ sequence: 1, eventType: "item.completed", summary: "已分析上下文与约束", createdAt: 1 }],
    } as AiRuntimeSnapshot["runs"][number];
    const commandRun = {
      status: "running",
      events: [{ sequence: 1, eventType: "command.started", summary: "开始执行工作区命令", createdAt: 1 }],
    } as AiRuntimeSnapshot["runs"][number];

    expect(visibleRunProgress(genericRun)).toBe("已分析上下文与约束");
    expect(visibleRunProgress(commandRun)).toBe("正在执行工作区命令…");
  });
});

describe("运行用时", () => {
  test("按分钟和小时稳定格式化", () => {
    expect(runDurationLabel(5_900)).toBe("00:05");
    expect(runDurationLabel(65_000)).toBe("01:05");
    expect(runDurationLabel(3_665_000)).toBe("01:01:05");
  });

  test("运行中按秒更新，结束后固定为最终用时", () => {
    const startedAt = 1_700_000_000_000;
    vi.useFakeTimers();
    vi.setSystemTime(startedAt + 5_000);
    try {
      const view = render(<RunElapsedTime completedAt={null} startedAt={startedAt}/>);
      expect(screen.getByLabelText("运行中 00:05").textContent).toBe("运行中 00:05");

      act(() => vi.advanceTimersByTime(2_000));
      expect(screen.getByLabelText("运行中 00:07").textContent).toBe("运行中 00:07");

      view.rerender(<RunElapsedTime completedAt={startedAt + 96_000} startedAt={startedAt}/>);
      expect(screen.getByLabelText("用时 01:36").textContent).toBe("用时 01:36");
      act(() => vi.advanceTimersByTime(2_000));
      expect(screen.getByLabelText("用时 01:36").textContent).toBe("用时 01:36");
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("对话记录身份", () => {
  const conversation: AiConversation = {
    conversationId: "conv-1", title: `讨论 ${relativePath}`, conversationType: "file", mode: "discuss",
    agentId: "agent-1", context: conversationContext(), workClassification: null, archived: false, createdAt: 0, updatedAt: 0,
  };
  const messages = [
    { messageId: "m1", conversationId: "conv-1", role: "user" as const, content: "请梳理这个项目的目录分类和维护边界", attachments: [], runId: "run-1", createdAt: 1 },
    { messageId: "m2", conversationId: "conv-1", role: "assistant" as const, content: "建议先区分产品、需求、设计和代码四类目录。", attachments: [], runId: "run-1", createdAt: 2 },
  ];

  test("文件会话有消息后使用首条问题作为主题", () => {
    expect(conversationTopic(conversation, messages)).toBe("请梳理这个项目的目录分类和维护边界");
  });

  test("尚未发送消息的文件会话不使用文件名充当主题", () => {
    expect(conversationTopic(conversation, [])).toBe("文件讨论");
    expect(conversationSummary(conversation, [])).toContain("引用 项目目录结构与产品分类.md");
  });

  test("文件上下文已经缺失的历史文件标题仍按文件讨论展示", () => {
    const legacy = { ...conversation, context: conversationContext({ localFileId: null, relativePath: null, fileRevision: null }) };
    expect(conversationTopic(legacy, [])).toBe("文件讨论");
  });

  test("列表摘要使用最近一条消息，文件名不再承担会话身份", () => {
    expect(conversationSummary(conversation, messages)).toBe("建议先区分产品、需求、设计和代码四类目录。");
  });

  test("列表摘要不显示历史回复中的旧协议外壳", () => {
    const legacyMessages = [...messages, {
      messageId: "m3",
      conversationId: "conv-1",
      role: "assistant" as const,
      content: JSON.stringify({ schemaVersion: 1, nonce: "old", reply: "这是实际回复正文", classification: {} }),
      attachments: [],
      runId: "run-2",
      createdAt: 3,
    }];
    expect(conversationSummary(conversation, legacyMessages)).toBe("这是实际回复正文");
  });
});

describe("AI 工作台渲染", () => {
  const failedRun: AiRuntimeSnapshot["runs"][number] = {
    runId: "run-failed", conversationId: "conv-1", agentId: "agent-1", inputMessageId: "message-existing",
    retryOfRunId: null, provider: "codex", model: "Qwen3.6-35B-A3B-NVFP4", executionTarget: "model",
    status: "failed", pid: null, startedAt: 1_700_000_000_000, completedAt: 1_700_000_005_000, output: null,
    errorReason: "credential unavailable",
    sessionId: null, skillIds: [], toolPermissions: [], permissionRequestId: null, automationRuleId: null,
    automationRuleVersion: null, sandboxMode: "read-only", events: [], artifacts: [],
    usage: { inputTokens: 11_127, cachedInputTokens: 0, outputTokens: 103 }, logTail: [],
  };

describe("应用转交消息", () => {
  const startedAt = Date.UTC(2026, 9, 15, 18, 54);
  const completedAt = startedAt + 5_000;
  const question = "当前区域的平均值是多少？\n保留原始换行。";
  const answer = "**平均值**为 113.125 μg/m³。";
  const source = {
    applicationId: "air", applicationName: "空气质量热力图", runId: "app-run-original",
    userMessageId: "imported-user", assistantMessageId: "imported-assistant",
    runSnapshot: { modelId: "retired-model", modelLabel: "原始模型", startedAt, completedAt, events: [] as AiRuntimeSnapshot["runs"][number]["events"] },
  };
  const importedMessages: AiRuntimeSnapshot["messages"] = [
    { ...existingMessage, messageId: source.userMessageId, content: question, createdAt: startedAt },
    { ...existingMessage, messageId: source.assistantMessageId, role: "assistant", content: answer, createdAt: completedAt },
  ];
  const conversations = [{ ...snapshot(conversationContext()).conversations[0], applicationHandoff: source }];

  test.each(["macos", "windows"])("%s 按主对话结构保留原模型、时间和问答，来源独立显示", async (hostPlatform) => {
    await renderWorkbench(conversationContext(), { hostPlatform, conversations, messages: importedMessages });
    const user = screen.getByLabelText("用户消息");
    const reply = screen.getByLabelText("AI 消息");
    const origin = screen.getByLabelText("转交来源");
    expect(user.className).toBe("ai-message user");
    expect(reply.className).toBe("ai-message assistant");
    expect(origin.textContent).toBe("来源：空气质量热力图");
    expect(user.contains(origin)).toBe(false);
    expect(user.querySelector(":scope > div")?.textContent).toBe(question);
    expect(reply.querySelector("header > span")?.textContent).toBe("原始模型");
    expect(user.querySelector("time")?.dateTime).toBe(new Date(startedAt).toISOString());
    expect(reply.querySelector("time")?.dateTime).toBe(new Date(completedAt).toISOString());
    expect(within(reply).getByLabelText("用时 00:05")).toBeTruthy();
    expect(reply.querySelector("strong")?.textContent).toBe("平均值");
    expect(within(reply).queryByRole("button", { name: /执行过程/ })).toBeNull();
    fireEvent.click(within(user).getByRole("button", { name: "复制对话内容" }));
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(question));
    fireEvent.click(within(reply).getByRole("button", { name: "复制对话内容" }));
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(answer));
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
  });

  test("只展示真实执行记录，转交摘要不发起主对话详情请求", async () => {
    const events: typeof source.runSnapshot.events = [
      { sequence: 1, eventType: "item.completed", itemId: "read-1", phase: "reply", kind: "mcp", status: "completed", summary: "已读取观测数据", detailAvailable: true, createdAt: startedAt + 1_000 },
      { sequence: 2, eventType: "item.completed", phase: "reply", kind: "message", status: "completed", summary: "已生成回复", createdAt: completedAt },
    ];
    const withEvents = [{ ...conversations[0], applicationHandoff: { ...source, runSnapshot: { ...source.runSnapshot, events } } }];
    await renderWorkbench(conversationContext(), { conversations: withEvents, messages: importedMessages });
    const reply = screen.getByLabelText("AI 消息");
    fireEvent.click(within(reply).getByRole("button", { name: "展开执行过程：执行过程" }));
    expect(within(reply).getByText("已读取观测数据")).toBeTruthy();
    expect(within(reply).queryByRole("button", { name: /已读取观测数据/ })).toBeNull();
    expect(within(reply).queryByText("已生成回复")).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "ai_run_event_detail")).toBe(false);
  });

  test("后续续聊保持自身模型和执行记录，不覆盖原回答来源", async () => {
    const laterRun = { ...failedRun, runId: "later-run", executionTarget: "model" as const, model: "later-model", modelLabel: "续聊模型", status: "completed" as const, errorReason: null, output: "后续回复" };
    const laterMessages: AiRuntimeSnapshot["messages"] = [
      ...importedMessages,
      { ...existingMessage, messageId: "later-user", runId: laterRun.runId, content: "继续分析", createdAt: completedAt + 1_000 },
      { ...existingMessage, messageId: "later-assistant", runId: laterRun.runId, role: "assistant", content: "后续回复", createdAt: completedAt + 2_000 },
    ];
    await renderWorkbench(conversationContext(), { conversations, messages: laterMessages, runs: [laterRun] });
    const replies = screen.getAllByLabelText("AI 消息");
    expect(replies[0].querySelector("header > span")?.textContent).toBe("原始模型");
    expect(replies[1].querySelector("header > span")?.textContent).toBe("续聊模型");
    expect(screen.getAllByLabelText("转交来源")).toHaveLength(1);
  });

  test("旧转交去除正文来源前缀，缺少原模型和时间时明确显示未记录", async () => {
    const legacy = [{ ...conversations[0], applicationHandoff: { applicationId: "air", applicationName: "空气质量热力图", runId: "legacy-run" } }];
    const messages = importedMessages.map((message) => message.role === "user" ? { ...message, content: `来自「空气质量热力图」的页面 AI 对话\n\n${question}` } : message);
    await renderWorkbench(conversationContext(), { conversations: legacy, messages });
    expect(screen.getByLabelText("用户消息").querySelector(":scope > div")?.textContent).toBe(question);
    expect(within(screen.getByLabelText("AI 消息")).getByText("原模型未记录")).toBeTruthy();
    expect(screen.getAllByText("原时间未记录")).toHaveLength(2);
    expect(screen.getByLabelText("AI 消息").querySelector("time")).toBeNull();
    expect(screen.queryByRole("button", { name: /执行过程/ })).toBeNull();
  });
});


  test.each(["macos", "windows"].flatMap(hostPlatform => (["running", "failed", "completed"] as const).map(status => ({ hostPlatform, status }))))("$hostPlatform 的 $status 回复只显示正文，历史失败保留详情", async ({ hostPlatform, status }) => {
    const nonce = "classification-visible-test";
    const raw = `图片已生成\n\n${JSON.stringify({ schemaVersion: 1, nonce, reply: "图片已生成", decision: "keep" })}`;
    const run = { ...failedRun, classificationNonce: nonce, status, partialOutput: raw,
      completedAt: status === "running" ? null : failedRun.completedAt, errorReason: "AI 回复保存失败：本地服务连接中断" };
    const nextRun = { ...failedRun, runId: "run-next", inputMessageId: "user-next", status: "running" as const, completedAt: null, errorReason: null };
    const messages = [{ ...existingMessage, runId: run.runId }, ...(status === "completed" ? [{ ...existingMessage, messageId: "reply-schema", role: "assistant" as const, content: raw, runId: run.runId, createdAt: 2 }] : []),
      ...(status === "failed" ? [{ ...existingMessage, messageId: "user-next", runId: nextRun.runId, content: "新的问题", createdAt: 3 }] : [])];
    await renderWorkbench(conversationContext(), { hostPlatform, messages, runs: status === "failed" ? [run, nextRun] : [run] });
    expect(within(screen.getByLabelText(status === "running" ? "正在输出的回复" : status === "failed" ? "未完成的 AI 消息" : "AI 消息")).getByText("图片已生成")).toBeTruthy();
    expect(document.body.textContent).not.toContain("schemaVersion");
    expect(document.body.textContent).not.toContain(nonce);
    if (status === "failed") {
      expect(within(screen.getByLabelText("未完成的 AI 消息")).getByText("AI 回复保存失败：本地服务连接中断")).toBeTruthy();
      expect(screen.getAllByText("新的问题").length).toBeGreaterThan(0);
    }
  });

  test("执行过程使用对话内活动流并按需读取脱敏详情", async () => {
    const run: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-process",
      status: "running",
      completedAt: null,
      errorReason: null,
      events: [
        { sequence: 1, eventType: "turn.started", phase: "reply", kind: "status", status: "started", summary: "模型开始处理本轮请求", createdAt: 1 },
        { sequence: 2, eventType: "item.completed", itemId: "reasoning-1", phase: "reply", kind: "reasoning", status: "completed", summary: "### 技术架构\n\n- **后端**：FastAPI\n- **前端**：React", createdAt: 2 },
        { sequence: 3, eventType: "item.completed", itemId: "command-1", phase: "reply", kind: "command", status: "completed", summary: "完成执行工作区命令", durationMs: 126, detailAvailable: true, createdAt: 3 },
        { sequence: 4, eventType: "harness.classification_started", phase: "classification", kind: "status", status: "started", summary: "开始生成工作归类", createdAt: 4 },
      ],
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [run] });

    const processToggle = screen.getByRole("button", { name: /开始生成工作归类/ });
    expect(processToggle.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(processToggle);
    expect(screen.getByRole("heading", { name: "技术架构" })).toBeTruthy();
    expect(screen.getByText("后端").tagName).toBe("STRONG");
    expect(screen.getByText("前端").closest("li")).toBeTruthy();
    expect(screen.queryByText(/### 技术架构/)).toBeNull();
    expect(screen.getByText("已执行工作区操作")).toBeTruthy();
    expect(screen.queryByText("正在理解你的请求")).toBeNull();
    expect(screen.queryByText("工作归类")).toBeNull();

    const stepToggle = screen.getByRole("button", { name: /已执行工作区操作/ });
    expect(stepToggle.textContent).not.toContain("126 ms");
    expect(invoke.mock.calls.some(([command]) => command === "ai_run_event_detail")).toBe(false);
    fireEvent.click(stepToggle);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_run_event_detail", { runId: "run-process", itemId: "command-1" }));
    expect(await screen.findByText("列出目录")).toBeTruthy();
    expect(screen.getByText("report.md")).toBeTruthy();
    expect(screen.queryByText("耗时")).toBeNull();
    expect(screen.queryByText("126 ms")).toBeNull();
    expect(screen.getByRole("button", { name: "复制已脱敏的步骤详情" })).toBeTruthy();
  });

  test("执行过程只显示实际工具调用并隐藏企业工具发现状态", async () => {
    const run: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-tool-usage",
      status: "running",
      completedAt: null,
      errorReason: null,
      events: [
        { sequence: 1, eventType: "application_discovery.ready", phase: "reply", kind: "status", status: "completed", summary: "企业应用目录可检索：3 个应用、31 个工具", createdAt: 1 },
        { sequence: 2, eventType: "application_discovery.failed", phase: "reply", kind: "status", status: "failed", summary: "应用发现请求失败", createdAt: 2 },
        { sequence: 3, eventType: "enterprise_tool_snapshot.partial", phase: "reply", kind: "mcp", status: "failed", summary: "企业应用工具目录部分可用：30 个可检索，1 个不可用", createdAt: 3 },
        { sequence: 4, eventType: "tool_search.failed", itemId: "search-failed", phase: "reply", kind: "mcp", status: "failed", summary: "企业工具检索失败", createdAt: 4 },
        { sequence: 5, eventType: "tool_search.completed", itemId: "search-1", phase: "reply", kind: "mcp", status: "completed", summary: "已发现 2 个企业工具", createdAt: 5 },
        { sequence: 6, eventType: "item.completed", itemId: "tool-1", phase: "reply", kind: "mcp", status: "completed", summary: "已使用 prepare_document_update", createdAt: 6 },
      ],
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [run] });

    const processToggle = screen.getByRole("button", { name: "展开执行过程：已使用 prepare_document_update" });
    fireEvent.click(processToggle);
    expect(screen.getAllByText("已使用 prepare_document_update")).toHaveLength(2);
    expect(screen.queryByText("应用发现失败，本轮继续普通回复")).toBeNull();
    expect(screen.queryByText("部分企业工具未加载，本轮继续处理")).toBeNull();
    expect(screen.queryByText("企业工具查找失败，本轮继续处理")).toBeNull();
    expect(screen.queryByText(/可检索/)).toBeNull();
    expect(screen.queryByText(/已发现 2 个企业工具/)).toBeNull();
  });

  test("整理对话上下文作为独立步骤显示在运行状态和执行过程中", async () => {
    const compacting: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-context-compaction",
      status: "running",
      completedAt: null,
      errorReason: null,
      events: [
        { sequence: 1, eventType: "item.completed", itemId: "command-1", phase: "reply", kind: "command", status: "completed", summary: "已运行工作区命令", createdAt: 1 },
        { sequence: 2, eventType: "item.started", itemId: "compact-1", phase: "reply", kind: "context", status: "started", summary: "正在整理对话上下文", createdAt: 2 },
        { sequence: 3, eventType: "thread.tokenUsage.updated", phase: "reply", kind: "status", status: "completed", summary: "Token 使用量已更新", createdAt: 3 },
      ],
    };
    expect(visibleRunProgress(compacting)).toBe("正在整理对话上下文");

    const run: AiRuntimeSnapshot["runs"][number] = {
      ...compacting,
      status: "completed",
      completedAt: 4,
      events: [
        compacting.events[0],
        { ...compacting.events[1], eventType: "item.completed", status: "completed", summary: "已整理对话上下文，对话较长时建议新建对话继续" },
      ],
    };
    const reply = { ...existingMessage, messageId: "reply-context-compaction", role: "assistant" as const, content: "整理后的回复", runId: run.runId, createdAt: 5 };
    await renderWorkbench(conversationContext(), { messages: [existingMessage, reply], runs: [run] });

    fireEvent.click(within(screen.getByLabelText("AI 消息")).getByRole("button", { name: /展开执行过程/ }));
    expect(screen.getAllByText("已整理对话上下文，对话较长时建议新建对话继续").length).toBeGreaterThan(0);
    expect(screen.getAllByText("已运行工作区命令").length).toBeGreaterThan(0);
  });

  test("历史运行缺少结构化字段时仍识别已有执行活动并排除归类阶段", async () => {
    const run: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-legacy-process",
      status: "completed",
      errorReason: null,
      events: [
        { sequence: 1, eventType: "item.started", kind: "status", status: "completed", summary: "开始执行工作区命令", createdAt: 1 },
        { sequence: 2, eventType: "item.completed", kind: "status", status: "completed", summary: "完成执行工作区命令", createdAt: 2 },
        { sequence: 3, eventType: "harness.classification_started", kind: "status", status: "completed", summary: "开始生成工作归类", createdAt: 3 },
        { sequence: 4, eventType: "item.started", kind: "status", status: "completed", summary: "开始执行工作区命令", createdAt: 4 },
        { sequence: 5, eventType: "item.completed", kind: "status", status: "completed", summary: "完成执行工作区命令", createdAt: 5 },
      ],
    };
    const reply = { ...existingMessage, messageId: "reply-legacy-process", role: "assistant" as const, content: "历史回复", runId: run.runId, createdAt: 6 };
    await renderWorkbench(conversationContext(), { messages: [existingMessage, reply], runs: [run] });

    fireEvent.click(within(screen.getByLabelText("AI 消息")).getByRole("button", { name: /展开执行过程/ }));
    expect(screen.getAllByText("已执行工作区操作")).toHaveLength(1);
    expect(screen.queryByText("本次运行没有可展示的执行记录")).toBeNull();
    expect(screen.queryByText("开始生成工作归类")).toBeNull();
  });

  test("技能加载失败时保留模型回复并在执行过程显示非阻断提示", async () => {
    const run: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-skill-skipped",
      status: "completed",
      errorReason: null,
      events: [
        { sequence: 1, eventType: "run.queued", phase: "reply", kind: "status", status: "completed", summary: "已进入模型 Harness 执行队列", createdAt: 1 },
        { sequence: 2, eventType: "skill.preparation_failed", phase: "reply", kind: "status", status: "failed", summary: "技能加载失败，本轮未加载技能，对话已继续", createdAt: 2 },
      ],
    };
    const reply = { ...existingMessage, messageId: "reply-skill-skipped", role: "assistant" as const, content: "普通模型回复已完成", runId: run.runId, createdAt: 3 };
    await renderWorkbench(conversationContext(), { messages: [existingMessage, reply], runs: [run] });

    const message = screen.getByLabelText("AI 消息");
    expect(within(message).getByText("普通模型回复已完成")).toBeTruthy();
    fireEvent.click(within(message).getByRole("button", { name: /展开执行过程/ }));
    expect(within(message).getByText("技能加载失败，本轮未加载技能，对话已继续")).toBeTruthy();
    expect(screen.queryByText("运行未能完成")).toBeNull();
  });

  test("每条消息可以复制，只有最近一条用户消息可以编辑并重新生成", async () => {
    const firstUser = { ...existingMessage, messageId: "message-first", content: "第一条问题", runId: "run-first", createdAt: 1 };
    const firstReply = { ...existingMessage, messageId: "reply-first", role: "assistant" as const, content: "第一条回复", runId: "run-first", createdAt: 2 };
    const latestUser = { ...existingMessage, messageId: "message-latest", content: "最近一条问题", runId: "run-latest", createdAt: 3 };
    const latestReply = { ...existingMessage, messageId: "reply-latest", role: "assistant" as const, content: "最近一条回复", runId: "run-latest", createdAt: 4 };
    const firstRun = { ...failedRun, runId: "run-first", inputMessageId: "message-first", status: "completed" as const, errorReason: null, startedAt: 1, completedAt: 2 };
    const latestRun = { ...failedRun, runId: "run-latest", inputMessageId: "message-latest", status: "completed" as const, errorReason: null, startedAt: 3, completedAt: 4 };
    await renderWorkbench(conversationContext(), { messages: [firstUser, firstReply, latestUser, latestReply], runs: [firstRun, latestRun] });

    const userMessages = screen.getAllByLabelText("用户消息");
    expect(within(userMessages[0]).queryByRole("button", { name: "编辑最近一条消息并重新生成" })).toBeNull();
    const latestUserArticle = userMessages[1];
    expect(within(latestUserArticle).getByRole("button", { name: "编辑最近一条消息并重新生成" })).toBeTruthy();

    const latestAssistantArticle = screen.getAllByLabelText("AI 消息").at(-1)!;
    fireEvent.click(within(latestAssistantArticle).getByRole("button", { name: "复制对话内容" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("最近一条回复"));

    const composer = screen.getByRole("textbox", { name: "发送给 AI" }) as HTMLTextAreaElement;
    fireEvent.change(composer, { target: { value: "尚未发送的普通草稿" } });
    fireEvent.click(within(latestUserArticle).getByRole("button", { name: "编辑最近一条消息并重新生成" }));
    const editor = within(latestUserArticle).getByRole("textbox", { name: "编辑最近一条消息" });
    expect(screen.getByRole("textbox", { name: "发送给 AI" })).toBe(composer);
    expect(composer.value).toBe("尚未发送的普通草稿");
    expect(latestUserArticle.querySelector(":scope > div")).toBeNull();
    expect((editor as HTMLTextAreaElement).value).toBe("最近一条问题");
    expect((editor as HTMLTextAreaElement).rows).toBe(2);
    Object.defineProperty(editor, "scrollHeight", { configurable: true, value: 112 });
    fireEvent.change(editor, { target: { value: "修改后的问题" } });
    expect(latestUserArticle.querySelector(":scope > div")).toBeNull();
    expect(within(latestAssistantArticle).getByText("最近一条回复")).toBeTruthy();
    expect((editor as HTMLTextAreaElement).style.height).toBe("112px");
    expect(fireEvent.keyDown(editor, { key: "Enter", shiftKey: true })).toBe(true);
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_regenerate")).toBe(false);
    fireEvent.keyDown(editor, { key: "Enter" });

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_message_regenerate", expect.objectContaining({ request: expect.objectContaining({
      conversationId: "conv-1", messageId: "message-latest", content: "修改后的问题", executionTarget: "model",
    }) })));
  });

  test.each(["terminated", "failed", "completed"] as const)("运行为 %s 但线程尚未释放时不能编辑，释放后可重新生成", async (status) => {
    const overrides = {
      messages: [existingMessage],
      runs: [{ ...failedRun, inputMessageId: existingMessage.messageId, status, threadWriterActive: true }],
      onMessage: vi.fn(),
    };
    await renderWorkbench(conversationContext(), overrides);
    expect(screen.queryByRole("button", { name: "编辑最近一条消息并重新生成" })).toBeNull();

    overrides.runs[0] = { ...overrides.runs[0], threadWriterActive: false };
    await act(async () => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
    fireEvent.click(await screen.findByRole("button", { name: "编辑最近一条消息并重新生成" }));
    fireEvent.change(screen.getByRole("textbox", { name: "编辑最近一条消息" }), { target: { value: "停止后修改的问题" } });
    fireEvent.click(screen.getByRole("button", { name: "重新生成" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_message_regenerate", expect.objectContaining({
      request: expect.objectContaining({ content: "停止后修改的问题" }),
    })));
    expect(invoke.mock.calls.filter(([command]) => command === "ai_message_regenerate")).toHaveLength(1);
    expect(overrides.onMessage).not.toHaveBeenCalled();
  });

  test("点击停止后等待线程回收，期间不开放历史编辑", async () => {
    const overrides = {
      messages: [existingMessage],
      runs: [{ ...failedRun, inputMessageId: existingMessage.messageId, status: "running" as AiRuntimeSnapshot["runs"][number]["status"], threadWriterActive: true, completedAt: null as number | null }],
    };
    await renderWorkbench(conversationContext(), overrides);
    const originalInvoke = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "ai_run_control") {
        overrides.runs[0] = { ...overrides.runs[0], status: "terminating" };
        return overrides.runs[0];
      }
      return originalInvoke(command, args);
    });
    fireEvent.click(screen.getByRole("button", { name: "停止回复" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_run_control", expect.objectContaining({
      request: { runId: failedRun.runId, action: "terminate" },
    })));
    overrides.runs[0] = { ...overrides.runs[0], status: "terminated", completedAt: Date.now() };
    await act(async () => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
    expect(screen.queryByRole("button", { name: "编辑最近一条消息并重新生成" })).toBeNull();
    overrides.runs[0] = { ...overrides.runs[0], threadWriterActive: false };
    await act(async () => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
    expect(await screen.findByRole("button", { name: "编辑最近一条消息并重新生成" })).toBeTruthy();
  });

  test("编辑期间发现线程仍被占用时，按钮和回车均不能提交且草稿保留", async () => {
    const overrides = {
      messages: [existingMessage],
      runs: [{ ...failedRun, inputMessageId: existingMessage.messageId, threadWriterActive: false }],
    };
    await renderWorkbench(conversationContext(), overrides);
    fireEvent.click(screen.getByRole("button", { name: "编辑最近一条消息并重新生成" }));
    const editor = screen.getByRole("textbox", { name: "编辑最近一条消息" }) as HTMLTextAreaElement;
    fireEvent.change(editor, { target: { value: "保留编辑内容" } });
    overrides.runs[0] = { ...overrides.runs[0], threadWriterActive: true };
    await act(async () => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
    expect((screen.getByRole("button", { name: "重新生成" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "重新生成" }));
    fireEvent.keyDown(editor, { key: "Enter" });
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_regenerate")).toBe(false);
    expect(editor.value).toBe("保留编辑内容");

    overrides.runs[0] = { ...overrides.runs[0], threadWriterActive: false };
    await act(async () => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
    expect((screen.getByRole("button", { name: "重新生成" }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.keyDown(editor, { key: "Enter" });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_message_regenerate", expect.objectContaining({
      request: expect.objectContaining({ content: "保留编辑内容" }),
    })));
  });

  test("重新生成提交后立即显示改写内容和等待动效，并接上实际运行", async () => {
    const user = { ...existingMessage, messageId: "message-regenerate-pending", content: "原问题", runId: "run-regenerate-old", createdAt: 1 };
    const reply = { ...existingMessage, messageId: "reply-regenerate-old", role: "assistant" as const, content: "原回复", runId: "run-regenerate-old", createdAt: 2 };
    const oldRun = { ...failedRun, runId: "run-regenerate-old", inputMessageId: user.messageId, status: "completed" as const, errorReason: null, startedAt: 1, completedAt: 2 };
    const newRun = { ...failedRun, runId: "run-regenerate-new", inputMessageId: user.messageId, status: "running" as const, partialOutput: "新的实时回复", errorReason: null, startedAt: Date.now(), completedAt: null };
    await renderWorkbench(conversationContext(), { messages: [user, reply], runs: [oldRun] });
    let releaseRegenerate = () => { /* 由测试主体放行 */ };
    const regenerateGate = new Promise<void>((resolve) => { releaseRegenerate = resolve; });
    let persisted = false;
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return {
        ...snapshot(conversationContext()),
        messages: persisted ? [{ ...user, content: "改写后的问题", runId: newRun.runId }] : [user, reply],
        runs: persisted ? [{ ...oldRun, superseded: true }, newRun] : [oldRun],
      };
      if (command === "ai_message_regenerate") {
        await regenerateGate;
        persisted = true;
        return newRun;
      }
      throw new Error(`unexpected command ${command}`);
    });

    fireEvent.click(within(screen.getByLabelText("用户消息")).getByRole("button", { name: "编辑最近一条消息并重新生成" }));
    fireEvent.change(screen.getByRole("textbox", { name: "编辑最近一条消息" }), { target: { value: "改写后的问题" } });
    fireEvent.click(screen.getByRole("button", { name: "重新生成" }));

    expect(screen.queryByRole("textbox", { name: "编辑最近一条消息" })).toBeNull();
    expect(within(screen.getByLabelText("用户消息")).getByText("改写后的问题")).toBeTruthy();
    expect(screen.queryByLabelText("AI 消息")).toBeNull();
    expect(screen.getByRole("status", { name: "回复准备中" }).querySelectorAll(".ai-pending-reply-dots i")).toHaveLength(3);
    expect(screen.getByLabelText(/^运行中 00:/)).toBeTruthy();

    releaseRegenerate();
    await waitFor(() => expect(screen.getByLabelText("正在输出的回复").textContent).toContain("新的实时回复"));
    expect(screen.queryByRole("status", { name: "回复准备中" })).toBeNull();
    expect(screen.getAllByLabelText("用户消息")).toHaveLength(1);
  });

  test("重新生成在保存前失败时恢复编辑草稿与原回复", async () => {
    const user = { ...existingMessage, messageId: "message-regenerate-error", content: "原问题", runId: "run-regenerate-error", createdAt: 1 };
    const reply = { ...existingMessage, messageId: "reply-regenerate-error", role: "assistant" as const, content: "原回复", runId: user.runId, createdAt: 2 };
    const run = { ...failedRun, runId: user.runId, inputMessageId: user.messageId, status: "completed" as const, errorReason: null, startedAt: 1, completedAt: 2 };
    await renderWorkbench(conversationContext(), { messages: [user, reply], runs: [run] });
    let rejectRegenerate: (reason?: unknown) => void = () => { /* 由测试主体触发 */ };
    const regenerateGate = new Promise<never>((_, reject) => { rejectRegenerate = reject; });
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages: [user, reply], runs: [run] };
      if (command === "ai_message_regenerate") return regenerateGate;
      throw new Error(`unexpected command ${command}`);
    });

    fireEvent.click(within(screen.getByLabelText("用户消息")).getByRole("button", { name: "编辑最近一条消息并重新生成" }));
    fireEvent.change(screen.getByRole("textbox", { name: "编辑最近一条消息" }), { target: { value: "还未保存的改写" } });
    fireEvent.click(screen.getByRole("button", { name: "重新生成" }));
    expect(screen.getByRole("status", { name: "回复准备中" })).toBeTruthy();

    rejectRegenerate(new Error("企业会话已失效"));
    await waitFor(() => expect(screen.getByRole("textbox", { name: "编辑最近一条消息" })).toBeTruthy());
    expect((screen.getByRole("textbox", { name: "编辑最近一条消息" }) as HTMLTextAreaElement).value).toBe("还未保存的改写");
    expect(within(screen.getByLabelText("AI 消息")).getByText("原回复")).toBeTruthy();
    expect(screen.queryByRole("status", { name: "回复准备中" })).toBeNull();
  });

  test("重新生成已保存但运行启动失败时保留新问题且不退回编辑", async () => {
    const user = { ...existingMessage, messageId: "message-regenerate-start-error", content: "原问题", runId: "run-before-start-error", createdAt: 1 };
    const reply = { ...existingMessage, messageId: "reply-before-start-error", role: "assistant" as const, content: "原回复", runId: user.runId, createdAt: 2 };
    const oldRun = { ...failedRun, runId: user.runId, inputMessageId: user.messageId, status: "completed" as const, errorReason: null, startedAt: 1, completedAt: 2 };
    const failedNewRun = { ...failedRun, runId: "run-after-start-error", inputMessageId: user.messageId, status: "failed" as const, errorReason: "运行启动失败", startedAt: 3, completedAt: 4 };
    await renderWorkbench(conversationContext(), { messages: [user, reply], runs: [oldRun] });
    let rejectRegenerate: (reason?: unknown) => void = () => { /* 由测试主体触发 */ };
    const regenerateGate = new Promise<never>((_, reject) => { rejectRegenerate = reject; });
    let persisted = false;
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return {
        ...snapshot(conversationContext()),
        messages: persisted ? [{ ...user, content: "已保存的新问题", runId: failedNewRun.runId }] : [user, reply],
        runs: persisted ? [{ ...oldRun, superseded: true }, failedNewRun] : [oldRun],
      };
      if (command === "ai_message_regenerate") return regenerateGate;
      throw new Error(`unexpected command ${command}`);
    });

    fireEvent.click(within(screen.getByLabelText("用户消息")).getByRole("button", { name: "编辑最近一条消息并重新生成" }));
    fireEvent.change(screen.getByRole("textbox", { name: "编辑最近一条消息" }), { target: { value: "已保存的新问题" } });
    fireEvent.click(screen.getByRole("button", { name: "重新生成" }));
    expect(screen.getByRole("status", { name: "回复准备中" })).toBeTruthy();

    persisted = true;
    rejectRegenerate(new Error("运行启动失败"));
    await waitFor(() => expect(screen.queryByRole("status", { name: "回复准备中" })).toBeNull());
    expect(screen.queryByRole("textbox", { name: "编辑最近一条消息" })).toBeNull();
    expect(within(screen.getByLabelText("用户消息")).getByText("已保存的新问题")).toBeTruthy();
    expect(screen.queryByLabelText("AI 消息")).toBeNull();
  });

  test("进入编辑只显示编辑框，取消或按 Esc 恢复原问题和已完成回复", async () => {
    const user = { ...existingMessage, messageId: "message-edit-cancel", content: "原问题", runId: "run-edit-cancel", createdAt: 1 };
    const reply = { ...existingMessage, messageId: "reply-edit-cancel", role: "assistant" as const, content: "原回复", runId: "run-edit-cancel", createdAt: 2 };
    const run = { ...failedRun, runId: "run-edit-cancel", inputMessageId: user.messageId, status: "completed" as const, errorReason: null, startedAt: 1, completedAt: 2 };
    await renderWorkbench(conversationContext(), { messages: [user, reply], runs: [run] });
    const userArticle = screen.getByLabelText("用户消息");
    fireEvent.click(within(userArticle).getByRole("button", { name: "编辑最近一条消息并重新生成" }));
    expect(userArticle.querySelector(":scope > div")).toBeNull();
    expect(within(userArticle).getByRole("textbox", { name: "编辑最近一条消息" })).toBeTruthy();
    expect(within(screen.getByLabelText("AI 消息")).getByText("原回复")).toBeTruthy();
    expect(screen.getByRole("button", { name: "重新生成" })).toBeTruthy();
    fireEvent.change(within(userArticle).getByRole("textbox", { name: "编辑最近一条消息" }), { target: { value: "草稿修改" } });
    fireEvent.click(within(userArticle).getByRole("button", { name: "取消" }));
    expect(within(userArticle).getByText("原问题")).toBeTruthy();
    expect(within(screen.getByLabelText("AI 消息")).getByText("原回复")).toBeTruthy();
    fireEvent.click(within(userArticle).getByRole("button", { name: "编辑最近一条消息并重新生成" }));
    fireEvent.keyDown(within(userArticle).getByRole("textbox", { name: "编辑最近一条消息" }), { key: "Escape" });
    expect(within(userArticle).getByText("原问题")).toBeTruthy();
    expect(within(screen.getByLabelText("AI 消息")).getByText("原回复")).toBeTruthy();
    fireEvent.click(within(userArticle).getByRole("button", { name: "编辑最近一条消息并重新生成" }));
    fireEvent.focus(screen.getByRole("textbox", { name: "发送给 AI" }));
    expect(screen.queryByRole("textbox", { name: "编辑最近一条消息" })).toBeNull();
    expect(within(userArticle).getByText("原问题")).toBeTruthy();
    expect(within(screen.getByLabelText("AI 消息")).getByText("原回复")).toBeTruthy();
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_regenerate")).toBe(false);
  });

  test("重新生成运行中在原问题后只显示一次新回复", async () => {
    const user = { ...existingMessage, messageId: "message-edit-running", content: "新问题", runId: "run-edit-running", createdAt: 1 };
    const oldRun = { ...failedRun, runId: "run-old", inputMessageId: user.messageId, superseded: true, status: "terminated" as const, partialOutput: "旧回复片段", startedAt: 1, completedAt: 2 };
    const activeRun = { ...failedRun, runId: "run-edit-running", inputMessageId: user.messageId, retryOfRunId: oldRun.runId, regenerationDraft: { content: "新问题", sourceRunId: oldRun.runId }, status: "running" as const, partialOutput: "新回复片段", startedAt: 3, completedAt: null, errorReason: null };
    await renderWorkbench(conversationContext(), { messages: [user], runs: [oldRun, activeRun] });
    expect(within(screen.getByLabelText("用户消息")).getByText("新问题")).toBeTruthy();
    expect(screen.queryByText("旧回复片段")).toBeNull();
    expect(screen.getAllByText("新回复片段")).toHaveLength(1);
    expect(screen.queryByLabelText("未完成的 AI 消息")).toBeNull();
  });

  test("重新生成终止后只显示修改后的问题和可复制的未完成回复", async () => {
    const user = { ...existingMessage, messageId: "message-previous", content: "新问题", runId: "run-regeneration", createdAt: 1 };
    const previousRun = { ...failedRun, runId: "run-previous", inputMessageId: user.messageId, superseded: true, status: "completed" as const, errorReason: null, startedAt: 1, completedAt: 2 };
    const terminatedRun = { ...failedRun, runId: "run-regeneration", inputMessageId: user.messageId, retryOfRunId: previousRun.runId, regenerationDraft: { content: "新问题", sourceRunId: previousRun.runId }, status: "terminated" as const, partialOutput: "新回复片段", startedAt: 3, completedAt: 4 };
    await renderWorkbench(conversationContext(), { messages: [user], runs: [previousRun, terminatedRun] });
    expect(within(screen.getByLabelText("用户消息")).getByText("新问题")).toBeTruthy();
    expect(screen.queryByText("原问题")).toBeNull();
    expect(within(screen.getByLabelText("未完成的 AI 消息")).getByText("新回复片段")).toBeTruthy();
    expect(screen.getAllByLabelText("未完成的 AI 消息")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "复制未完成的回复" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("新回复片段"));
  });

  test("重新生成成功后只显示当前问题和回复", async () => {
    const user = { ...existingMessage, messageId: "message-revised", content: "新问题", runId: "run-revised", createdAt: 1 };
    const reply = { ...existingMessage, messageId: "reply-revised", role: "assistant" as const, content: "新回复", runId: "run-revised", createdAt: 2 };
    const run = { ...failedRun, runId: "run-revised", inputMessageId: user.messageId, status: "completed" as const, errorReason: null, startedAt: 1, completedAt: 2, previousRevisions: [{ runId: "run-previous", userContent: "原问题", assistantContents: ["原回复"], incomplete: false }] };
    await renderWorkbench(conversationContext(), { messages: [user, reply], runs: [run] });
    expect(within(screen.getByLabelText("用户消息")).getByText("新问题")).toBeTruthy();
    expect(within(screen.getByLabelText("AI 消息")).getByText("新回复")).toBeTruthy();
    expect(screen.queryByText("上一版", { selector: "summary" })).toBeNull();
    expect(screen.queryByText("原问题")).toBeNull();
    expect(screen.queryByText("原回复")).toBeNull();
  });

  test("AI 回复期间默认排队，并允许移回输入框修改或删除排队消息", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-active",
      model: "Qwen3.8-27B-NVFP4",
      status: "running",
      pid: 123,
      startedAt: Date.now(),
      completedAt: null,
      errorReason: null,
      sessionId: "thread-active",
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [activeRun] });

    const elapsed = screen.getByLabelText(/^运行中 \d{2}:\d{2}$/);
    expect(elapsed.parentElement?.tagName).toBe("HEADER");
    expect(elapsed.previousElementSibling?.textContent).toBe("Qwen3.8 27B NVFP4");
    const input = screen.getByLabelText("发送给 AI") as HTMLTextAreaElement;
    expect(input.disabled).toBe(false);
    expect(input.placeholder).toBe("输入问题、补充约束或下一步要求…");
    expect(screen.getByRole("button", { name: "停止回复" })).toBeTruthy();
    fireEvent.change(input, { target: { value: "请继续补充验收边界" } });
    expect(screen.queryByRole('button', { name: '停止回复' })).toBeNull();
    expect(screen.queryByRole('button', { name: '更多发送操作' })).toBeNull();
    const queuedSend = screen.getByRole('button', { name: '排队发送' });
    expect(queuedSend.parentElement?.querySelectorAll('.ai-send-button')).toHaveLength(1);
    fireEvent.change(input, { target: { value: '' } });
    expect(screen.getByRole('button', { name: '停止回复' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: '排队发送' })).toBeNull();
    fireEvent.change(input, { target: { value: '请继续补充验收边界' } });
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));

    const queue = screen.getByRole("region", { name: "待发送的后续指令" });
    expect(within(queue).getByText("请继续补充验收边界")).toBeTruthy();
    expect(input.value).toBe('');
    expect(screen.getByRole('button', { name: '停止回复' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: '排队发送' })).toBeNull();
    /*
     * 默认状态不写出来：每条排队项都是"等待当前回复完成"，条数越多越是重复。
     * 顺序由行的排列表达，序号也不再占版面，只留在无障碍标签里。
     */
    expect(within(queue).queryByText("等待当前回复完成")).toBeNull();
    expect(within(queue).queryByText("排队 1")).toBeNull();
    // 操作逐条常驻：当前回复一结束队首就发出去了，那时才去找按钮已经来不及
    expect(within(queue).getByRole("button", { name: "排队消息 1 的更多操作" })).toBeTruthy();
    expect(within(queue).getByRole("button", { name: "删除排队消息 1" })).toBeTruthy();
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_steer")).toBe(false);

    openQueuedEdit(queue);
    expect(screen.queryByRole("region", { name: "待发送的后续指令" })).toBeNull();
    expect(input.value).toBe("请继续补充验收边界");
    fireEvent.change(input, { target: { value: "请补充修改后的验收边界" } });
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
    const revisedQueue = screen.getByRole("region", { name: "待发送的后续指令" });
    expect(within(revisedQueue).getByText("请补充修改后的验收边界")).toBeTruthy();

    fireEvent.click(within(revisedQueue).getByRole("button", { name: "删除排队消息 1" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "待发送的后续指令" })).toBeNull());
  });

  test("AI 回复期间仍可为下一轮切换完整显示的模型和审批模式", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-active-selection",
      model: "Qwen3.8-27B-NVFP4",
      status: "running",
      pid: 123,
      startedAt: Date.now(),
      completedAt: null,
      errorReason: null,
      sessionId: "thread-active-selection",
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [activeRun] });

    const modelPicker = await screen.findByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" });
    expect(modelPicker).toHaveProperty("disabled", false);
    expect(modelPicker.querySelector("span")?.textContent).toBe("Qwen3.8 27B NVFP4");
    expect((modelPicker.querySelector("span") as HTMLElement).style.width).toBe("18ch");
    fireEvent.click(modelPicker);
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.6 35B A3B NVFP4/ }));

    const permissionPicker = screen.getByRole("button", { name: "选择审批模式：请求批准" });
    expect(permissionPicker).toHaveProperty("disabled", false);
    fireEvent.click(permissionPicker);
    fireEvent.click(screen.getByRole("menuitemradio", { name: /^完全访问权限/ }));
    expect(screen.getByRole("button", { name: "选择审批模式：完全访问权限" })).toBeTruthy();
    expect(screen.getAllByText("Qwen3.8 27B NVFP4").length).toBeGreaterThan(0);
  });

  test("流式输出时跟随底部，用户上拉后停止抢夺滚动位置并提供回到底部入口", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-streaming",
      model: "Qwen3.8-27B-NVFP4",
      status: "running",
      pid: 123,
      startedAt: Date.now(),
      completedAt: null,
      partialOutput: "第一段输出",
      errorReason: null,
      sessionId: "thread-streaming",
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [activeRun] });

    expect(screen.getByLabelText("正在输出的回复").textContent).toContain("第一段输出");
    expect(document.querySelector(".ai-streaming-caret")).toBeNull();
    const executionProcess = screen.getByRole("button", { name: /执行过程：/ }).closest(".ai-execution-process");
    expect(executionProcess?.getAttribute("data-active")).toBe("true");
    const list = document.querySelector<HTMLElement>(".ai-message-list")!;
    Object.defineProperty(list, "scrollHeight", { configurable: true, value: 1200 });
    Object.defineProperty(list, "clientHeight", { configurable: true, value: 500 });
    fireEvent.wheel(list);
    list.scrollTop = 260;
    fireEvent.scroll(list);
    const initialReturnButton = screen.getByRole("button", { name: "回到底部" });
    expect(initialReturnButton.textContent).toBe("");

    activeRun.partialOutput = "第一段输出\n第二段输出";
    act(() => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
    await waitFor(() => expect(screen.getByLabelText("正在输出的回复").textContent).toContain("第二段输出"));
    expect(list.scrollTop).toBe(260);
    const returnButton = screen.getByRole("button", { name: "回到底部，有新输出" });
    fireEvent.click(returnButton);
    expect(list.scrollTo).toHaveBeenCalledWith({ top: 1200, behavior: "auto" });
  });

  test("回复完成后执行过程文字恢复静态状态", async () => {
    const run = { ...failedRun, runId: "run-finished", status: "completed" as const, errorReason: null, output: "已完成" };
    const reply = { ...existingMessage, messageId: "reply-finished", role: "assistant" as const, runId: run.runId, content: "已完成" };
    await renderWorkbench(conversationContext(), { messages: [existingMessage, reply], runs: [run] });

    const toggle = within(screen.getByLabelText("AI 消息")).getByRole("button", { name: /执行过程/ });
    expect(toggle.closest(".ai-execution-process")?.hasAttribute("data-active")).toBe(false);
    expect(document.querySelector(".ai-streaming-caret")).toBeNull();
  });

  test("第三轮进入阅读区域或消息区底部时高亮第三条，回看第二轮时切回第二条", async () => {
    const messages: AiRuntimeSnapshot["messages"] = [
      ...[1, 2, 3].flatMap((number) => [
        { ...existingMessage, messageId: `question-${number}`, content: `第 ${number} 次发问`, createdAt: number * 2 },
        { ...existingMessage, messageId: `reply-${number}`, role: "assistant" as const, content: `第 ${number} 次回复`, createdAt: number * 2 + 1 },
      ]),
    ];
    await renderWorkbench(conversationContext(), { messages });

    const list = document.querySelector<HTMLElement>(".ai-message-list")!;
    const questions = [1, 2, 3].map((number) => document.querySelector<HTMLElement>(`[data-message-id="question-${number}"]`)!);
    const positions = [100, 700, 2180];
    Object.defineProperty(list, "scrollHeight", { configurable: true, value: 2400 });
    Object.defineProperty(list, "clientHeight", { configurable: true, value: 600 });
    vi.spyOn(list, "getBoundingClientRect").mockReturnValue({ top: 100 } as DOMRect);
    questions.forEach((question, index) => vi.spyOn(question, "getBoundingClientRect")
      .mockImplementation(() => ({ top: 100 + positions[index] - list.scrollTop }) as DOMRect));
    const activeQuestion = () => document.querySelector<HTMLButtonElement>('.conversation-navigator__tick[data-active="true"]')?.getAttribute("aria-label");

    list.scrollTop = 600;
    fireEvent.scroll(list);
    await waitFor(() => expect(activeQuestion()).toContain("第 2 次发问"));

    positions[2] = 1100;
    list.scrollTop = 900;
    fireEvent.scroll(list);
    await waitFor(() => expect(activeQuestion()).toContain("第 3 次发问"));

    positions[2] = 2180;
    list.scrollTop = 600;
    fireEvent.scroll(list);
    await waitFor(() => expect(activeQuestion()).toContain("第 2 次发问"));
    list.scrollTop = 1800;
    fireEvent.scroll(list);
    await waitFor(() => expect(activeQuestion()).toContain("第 3 次发问"));

    const scrollIntoView = vi.spyOn(Element.prototype, "scrollIntoView");
    fireEvent.click(within(screen.getByRole("navigation", { name: "跳转到我发出的内容" })).getByRole("button", { name: /第 1 次发问/ }));
    expect(list.scrollTo).toHaveBeenCalledWith({ top: 72, behavior: "smooth" });
    expect(scrollIntoView).not.toHaveBeenCalled();
  });

  test("回复完成时的布局滚动不会取消最新输出跟随", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-completing",
      model: "Qwen3.8-27B-NVFP4",
      status: "running",
      pid: 123,
      startedAt: Date.now(),
      completedAt: null,
      partialOutput: "联通正常",
      errorReason: null,
      sessionId: "thread-completing",
    };
    const messages: AiRuntimeSnapshot["messages"] = [existingMessage];
    const runs: AiRuntimeSnapshot["runs"] = [activeRun];
    await renderWorkbench(conversationContext(), { messages, runs });

    const list = document.querySelector<HTMLElement>(".ai-message-list")!;
    Object.defineProperty(list, "scrollHeight", { configurable: true, value: 1200 });
    Object.defineProperty(list, "clientHeight", { configurable: true, value: 500 });
    list.scrollTop = 260;
    fireEvent.scroll(list);
    expect(screen.queryByRole("button", { name: "回到底部" })).toBeNull();

    activeRun.status = "completed";
    activeRun.pid = null;
    activeRun.completedAt = Date.now();
    messages.push({
      messageId: "message-completed",
      conversationId: "conv-1",
      role: "assistant",
      content: "联通正常",
      attachments: [],
      runId: activeRun.runId,
      createdAt: Date.now(),
    });
    act(() => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));

    await screen.findByText("联通正常");
    await waitFor(() => expect(list.scrollTop).toBe(1200));
    expect(screen.queryByRole("button", { name: /回到底部/ })).toBeNull();
  });

  test("发送时输入框收起后保持消息区停在最新内容", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    const list = document.querySelector<HTMLElement>(".ai-message-list")!;
    Object.defineProperty(list, "scrollHeight", { configurable: true, value: 1200 });
    Object.defineProperty(list, "clientHeight", { configurable: true, value: 500 });
    list.scrollTop = 700;
    fireEvent.scroll(list);

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "检查发送后的滚动位置" } });
    list.scrollTop = 700;
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(list.scrollTop).toBe(1200));
  });

  test("异常结束后保留已经生成的回复并按 Markdown 展示", async () => {
    const interruptedRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-interrupted-output",
      status: "interrupted",
      partialOutput: "## 已完成部分\n\n- 第一项",
      completedAt: Date.now(),
      errorReason: "桌面端或运行监督器已重启，原进程状态无法继续确认。",
      artifacts: [{ artifactId: "artifact-partial", artifactType: "file", label: "部分产物", path: "产物/部分结果.md" }],
    };
    const input = { ...existingMessage, runId: interruptedRun.runId };
    await renderWorkbench(conversationContext(), { messages: [input], runs: [interruptedRun] });

    const partial = screen.getByLabelText("未完成的 AI 消息");
    expect(within(partial).getByRole("heading", { name: "已完成部分" })).toBeTruthy();
    expect(within(partial).getByText("第一项").closest("li")).toBeTruthy();
    expect(within(partial).getByText("运行被意外中断，已保留已生成内容")).toBeTruthy();
    expect(within(partial).getByRole("button", { name: "打开文件 部分结果.md" })).toBeTruthy();
    expect(within(partial).getByRole("button", { name: "复制未完成的回复" })).toBeTruthy();
  });

  test("对话继续后保留每一轮已经生成的未完成回复", async () => {
    const olderRun = {
      ...failedRun,
      runId: "run-older-failure",
      inputMessageId: "message-older",
      startedAt: failedRun.startedAt,
      partialOutput: "旧运行留下的部分输出",
      errorReason: "旧运行错误详情",
    };
    const latestRun = {
      ...failedRun,
      runId: "run-latest-failure",
      inputMessageId: "message-latest",
      startedAt: failedRun.startedAt + 10_000,
      partialOutput: "最新运行留下的部分输出",
      errorReason: "新的模型服务错误",
    };
    const messages = [
      { ...existingMessage, messageId: "message-older", content: "旧问题", runId: olderRun.runId, createdAt: 1 },
      { ...existingMessage, messageId: "message-latest", content: "继续", runId: latestRun.runId, createdAt: 2 },
    ];
    await renderWorkbench(conversationContext(), { messages, runs: [olderRun, latestRun] });

    expect(screen.getAllByLabelText("未完成的 AI 消息")).toHaveLength(2);
    expect(screen.getByText("旧运行留下的部分输出")).toBeTruthy();
    expect(screen.getByText("最新运行留下的部分输出")).toBeTruthy();
    expect(screen.getAllByText("运行未能完成，已保留已生成内容")).toHaveLength(2);
    expect(screen.getByRole("region", { name: "最近一次运行异常" })).toBeTruthy();
  });

  test("图片可以随文字排队，以缩略图展示并在编辑时完整回到输入框", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-active-media",
      model: "Qwen3.8-27B-NVFP4",
      status: "running",
      pid: 123,
      startedAt: Date.now(),
      completedAt: null,
      errorReason: null,
      sessionId: "thread-active-media",
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [activeRun] });
    await screen.findByRole("button", { name: /选择模型：Qwen3\.8 27B NVFP4/ });

    const input = screen.getByLabelText("发送给 AI");
    const mediaInput = document.querySelector<HTMLInputElement>("input.ai-media-input")!;
    fireEvent.change(mediaInput, { target: { files: [new File([new Uint8Array([1, 2, 3])], "image.png", { type: "image/png" })] } });
    const composerThumbnail = await screen.findByRole("button", { name: "查看图片 image.png" });
    const attachmentStrip = screen.getByLabelText("待发送附件");
    expect(attachmentStrip.compareDocumentPosition(input) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.queryByText("image.png")).toBeNull();

    fireEvent.click(composerThumbnail);
    expect(screen.getByRole("dialog", { name: "查看图片 image.png" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "关闭媒体预览" }));

    fireEvent.change(input, { target: { value: "请分析这张图" } });
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
    const queue = screen.getByRole("region", { name: "待发送的后续指令" });
    expect(within(queue).getByText("请分析这张图")).toBeTruthy();
    expect(within(queue).getByRole("button", { name: "查看图片 image.png" })).toBeTruthy();
    expect(within(queue).queryByRole("button", { name: /补充回复/ })).toBeNull();

    openQueuedEdit(queue);
    expect((screen.getByLabelText("发送给 AI") as HTMLTextAreaElement).value).toBe("请分析这张图");
    expect(screen.getByRole("button", { name: "查看图片 image.png" })).toBeTruthy();
  });

  test("消息发出后输入区立即解除锁定，不等整份快照刷新", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    const base = invoke.getMockImplementation()!;
    const activeRun = { runId: "run-sent", conversationId: "conv-1", agentId: "agent-1", inputMessageId: "message-sent", status: "running", startedAt: 3, events: [], usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 } };
    let snapshotRequests = 0;
    invoke.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "ai_message_submit") return { status: "recorded", run: activeRun };
      // 发送后再有整份快照请求就永远挂起：输入区若还在等它，就会一直处于锁定
      if (command === "ai_runtime_snapshot") {
        snapshotRequests += 1;
        return new Promise(() => { /* 永不返回 */ });
      }
      return base(command, args as never);
    });

    const input = screen.getByLabelText("发送给 AI") as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: "继续分析" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    await waitFor(() => expect(input.disabled).toBe(false));
    expect(snapshotRequests).toBe(0);
    expect(screen.getByRole("button", { name: "停止回复" })).toBeTruthy();
  });

  test("已发送图片与用户消息右对齐，只显示缩略图并可放大", async () => {
    const attachment = {
      attachmentId: "attachment-sent",
      fileName: "screen.png",
      mimeType: "image/png",
      mediaType: "image" as const,
      sizeBytes: 2048,
      localPath: "",
    };
    const userMessage = { ...existingMessage, content: "这是什么内容", attachments: [attachment] };
    await renderWorkbench(conversationContext(), { messages: [userMessage] });

    const userArticle = screen.getByLabelText("用户消息");
    const thumbnail = await within(userArticle).findByRole("button", { name: "查看图片 screen.png" });
    const attachments = within(userArticle).getByLabelText("消息附件");
    expect(attachments.classList.contains("ai-message-attachments")).toBe(true);
    expect(within(userArticle).queryByText("screen.png")).toBeNull();
    expect(attachments.compareDocumentPosition(within(userArticle).getByText("这是什么内容")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    fireEvent.click(thumbnail);
    expect(screen.getByRole("dialog", { name: "查看图片 screen.png" })).toBeTruthy();
  });

  test("补充提交后立即显示，当前回复位于补充下方且接收后只保留一条", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-active",
      model: "Qwen3.8-27B-NVFP4",
      status: "running",
      pid: 123,
      startedAt: Date.now(),
      completedAt: null,
      errorReason: null,
      partialOutput: "当前仍在生成的回复",
      sessionId: "thread-active",
      events: [{ sequence: 1, eventType: "turn.started", phase: "reply", summary: "模型开始处理本轮请求", createdAt: 1 }],
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [activeRun] });
    let recordedMessages = [existingMessage];
    const originalInvoke = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages: recordedMessages, runs: [activeRun] };
      if (command === "ai_runtime_activity_snapshot") return { stateRevision: 0, conversations: snapshot(conversationContext()).conversations, messages: recordedMessages, runs: [activeRun], permissionRequests: [], missingRunIds: [] };
      return originalInvoke(command, args);
    });

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "只列当前层目录" } });
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
    const queue = screen.getByRole("region", { name: "待发送的后续指令" });
    fireEvent.click(within(queue).getByRole("button", { name: "补充回复：排队 1" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_steer")).toBe(true));
    const steerCall = invoke.mock.calls.find(([command]) => command === "ai_message_steer");
    expect(steerCall?.[1]?.request).toMatchObject({ runId: "run-active", content: "只列当前层目录", clientMessageId: expect.stringMatching(/^queued-follow-up-/) });
    const submittedId = steerCall![1].request.clientMessageId;
    await waitFor(() => expect(document.querySelector(`[data-message-id="${submittedId}"]`)).toBeTruthy());
    expect(screen.queryByRole("region", { name: "待发送的后续指令" })).toBeNull();
    const submitted = document.querySelector(`[data-message-id="${submittedId}"]`)!;
    expect(submitted.textContent).not.toMatch(/正在确认补充|等待发送|正在发送/);
    expect(submitted.compareDocumentPosition(screen.getByLabelText("正在输出的回复")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    recordedMessages = [...recordedMessages, { ...existingMessage, messageId: steerCall![1].request.clientMessageId, content: "只列当前层目录", runId: "run-active", createdAt: Date.now() }];
    act(() => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
    await waitFor(() => expect(document.querySelectorAll(`[data-message-id="${submittedId}"]`).length).toBe(1));
    await waitFor(() => expect(document.querySelector(`[data-message-id="${submittedId}"]`)?.textContent).not.toContain("正在确认补充"));
    expect(document.querySelector(`[data-message-id="${submittedId}"]`)!.compareDocumentPosition(screen.getByLabelText("正在输出的回复")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  test.each(["macos", "windows"])("%s 运行中从输入框排队的纯文字消息显示补充回复", async (hostPlatform) => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun, runId: "run-active", model: "Qwen3.8-27B-NVFP4", status: "running", pid: 123,
      startedAt: Date.now(), completedAt: null, errorReason: null, partialOutput: "生成中", sessionId: "thread-active",
      events: [{ sequence: 1, eventType: "turn.started", phase: "reply", summary: "模型开始处理本轮请求", createdAt: 1 }],
    };
    await renderWorkbench(conversationContext(), { hostPlatform, messages: [existingMessage], runs: [activeRun] });
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "只列当前层目录" } });
    fireEvent.click(await screen.findByRole("button", { name: "排队发送" }));
    const queue = await screen.findByRole("region", { name: "待发送的后续指令" });
    const steer = within(queue).getByRole("button", { name: "补充回复：排队 1" }) as HTMLButtonElement;
    expect(steer.disabled).toBe(false);
  });

  test("补充前的历史回复保留原位，继续生成的回复位于补充消息下方", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-segmented",
      model: "Qwen3.8-27B-NVFP4",
      status: "running",
      startedAt: 1,
      completedAt: null,
      errorReason: null,
      partialOutput: "继续生成的内容",
      events: [{ sequence: 1, eventType: "turn.steered", phase: "reply", summary: "已将补充指令加入当前回复", createdAt: 2 }],
    };
    await renderWorkbench(conversationContext(), {
      messages: [
        existingMessage,
        { ...existingMessage, messageId: "reply-before-steer", role: "assistant", runId: activeRun.runId, content: "已经生成的内容", createdAt: 2 },
        { ...existingMessage, messageId: "steer-message", runId: activeRun.runId, content: "还没好吗", createdAt: 2 },
      ],
      runs: [activeRun],
    });

    const priorReply = screen.getByText("已经生成的内容");
    const supplementalMessage = document.querySelector('[data-message-id="steer-message"]')!;
    const continuedReply = screen.getByLabelText("正在输出的回复");
    expect(supplementalMessage.textContent).toContain("还没好吗");
    expect(priorReply.compareDocumentPosition(supplementalMessage) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(supplementalMessage.compareDocumentPosition(continuedReply) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(continuedReply).getByText("继续生成的内容")).toBeTruthy();
  });

  test.each([3, 4])("连续补充后完成和重新打开对话，回复位于补充下方（回复时间 %s）", async (replyAt) => {
    const run = { ...failedRun, runId: "anchored-run", inputMessageId: existingMessage.messageId,
      status: "completed" as const, output: "按两条补充继续生成的完整回复", partialOutput: null, errorReason: null };
    const messages = [existingMessage,
      { ...existingMessage, messageId: "supplement-1", runId: run.runId, content: "第一条补充", createdAt: 2 },
      { ...existingMessage, messageId: "supplement-2", runId: run.runId, content: "第二条补充", createdAt: 3 },
      { ...existingMessage, messageId: "complete-reply", runId: run.runId, role: "assistant" as const, content: run.output, createdAt: replyAt }];
    await renderWorkbench(conversationContext(), { messages, runs: [run] });
    const assertOrder = () => {
      const reply = within(screen.getByLabelText("AI 消息")).getByText(run.output);
      for (const content of ["第一条补充", "第二条补充"]) {
        expect(screen.getByText(content).compareDocumentPosition(reply) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      }
      expect(within(screen.getByLabelText("AI 消息")).getAllByText(run.output)).toHaveLength(1);
      expect(screen.queryByText("等待发送")).toBeNull();
    };
    assertOrder();
    cleanup();
    await renderWorkbench(conversationContext(), { messages, runs: [run] });
    assertOrder();
  });

  test("测试输入的最终回复排在测试输入之后，后续新一轮保持独立顺序", async () => {
    const run = { ...failedRun, runId: "test-input-run", inputMessageId: existingMessage.messageId,
      status: "completed" as const, output: "收到，测试正常。", partialOutput: null, errorReason: null };
    const nextRun = { ...run, runId: "next-run", inputMessageId: "next-input", output: "下一轮回复", startedAt: 5 };
    await renderWorkbench(conversationContext(), { runs: [run, nextRun], messages: [
      { ...existingMessage, content: "11212", runId: run.runId },
      { ...existingMessage, messageId: "test-input", content: "测试输入", runId: run.runId, createdAt: 2 },
      { ...existingMessage, messageId: "test-reply", role: "assistant", content: run.output, runId: run.runId, createdAt: 3 },
      { ...existingMessage, messageId: "next-input", content: "继续下一轮", runId: nextRun.runId, createdAt: 5 },
      { ...existingMessage, messageId: "next-reply", role: "assistant", content: nextRun.output, runId: nextRun.runId, createdAt: 6 },
    ] });
    expect(Array.from(document.querySelectorAll("[data-message-id]")).map((node) => node.getAttribute("data-message-id")))
      .toEqual([existingMessage.messageId, "test-input", "test-reply", "next-input", "next-reply"]);
  });

  test.each(["failed", "interrupted", "terminated"] as const)("补充后运行 %s，保留的正文位于最后一条补充下方", async (status) => {
    const run = { ...failedRun, runId: "supplement-incomplete", status, partialOutput: "补充后已生成的正文" };
    const nextRun = { ...failedRun, runId: "next-run", inputMessageId: "next-input", status: "running" as const,
      partialOutput: "下一轮正在生成", completedAt: null, errorReason: null, startedAt: 5 };
    await renderWorkbench(conversationContext(), { runs: [run, nextRun], messages: [
      { ...existingMessage, runId: run.runId },
      { ...existingMessage, messageId: "supplement-1", runId: run.runId, content: "第一条补充", createdAt: 2 },
      { ...existingMessage, messageId: "supplement-2", runId: run.runId, content: "第二条补充", createdAt: 3 },
      { ...existingMessage, messageId: "next-input", runId: nextRun.runId, content: "继续下一轮", createdAt: 5 },
    ] });
    const partial = screen.getByLabelText("未完成的 AI 消息");
    const nextInput = document.querySelector('[data-message-id="next-input"]')!;
    expect(within(partial).getByText(run.partialOutput)).toBeTruthy();
    expect(screen.getByText("第二条补充").compareDocumentPosition(partial) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(partial.compareDocumentPosition(nextInput) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(nextInput.compareDocumentPosition(screen.getByLabelText("正在输出的回复")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  test.each(["macos", "windows"].flatMap(hostPlatform => (["completed", "failed"] as const).map(status => ({ hostPlatform, status }))))("$hostPlatform 对话 $status 后点击补充会重试未保存队列并仅提交一次", async ({ hostPlatform, status }) => {
    const run = { ...failedRun, runId: "finished-run", status, partialOutput: null, errorReason: null };
    sessionApi.snapshot.values.queuedFollowUps = [{ queueId: "queued-unsaved", queuedAt: 1, conversationId: "conv-1", content: "补充说明", attachments: [], applicationContext: null, modelOverride: "Qwen3.8-27B-NVFP4", reasoningEffort: null, executionTarget: "model", permissionMode: "auto-review", status: "queued" }];
    await renderWorkbench(conversationContext(), { hostPlatform, messages: [existingMessage], runs: [run] });
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "下一条草稿" } });
    sessionApi.failNextPatch = new Error("本地保存暂时失败");
    await act(async () => { await expect(flushConversationSessions()).rejects.toThrow("本地保存暂时失败"); });
    expect(screen.getByRole("alert").textContent).toContain("草稿未保存");
    fireEvent.click(screen.getByRole("button", { name: "补充回复：排队 1" }));
    await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === "ai_message_steer")).toHaveLength(1));
    expect(invoke.mock.calls.find(([command]) => command === "ai_message_steer")?.[1].request).toMatchObject({ runId: run.runId, clientMessageId: "queued-unsaved", content: "补充说明" });
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(sessionApi.snapshot.values.drafts).toMatchObject({ "conversation:conv-1": "下一条草稿" });
  });

  test("补充回复失败时保留排队消息及其修改删除操作", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-active",
      model: "Qwen3.8-27B-NVFP4",
      status: "running",
      pid: 123,
      startedAt: Date.now(),
      completedAt: null,
      errorReason: null,
      sessionId: "thread-active",
      events: [{ sequence: 1, eventType: "turn.started", phase: "reply", summary: "模型开始处理本轮请求", createdAt: 1 }],
    };
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [activeRun], onMessage });
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "保留这条排队消息" } });
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages: [existingMessage], runs: [activeRun] };
      if (command === "ai_message_steer") throw new Error("steer unavailable");
      throw new Error(`unexpected command ${command}`);
    });

    const queue = screen.getByRole("region", { name: "待发送的后续指令" });
    fireEvent.click(within(queue).getByRole("button", { name: "补充回复：排队 1" }));

    await waitFor(() => expect(onMessage).toHaveBeenCalledWith(expect.stringContaining("补充回复失败"), "error"));
    expect(within(queue).getByText("保留这条排队消息")).toBeTruthy();
    expect(within(queue).getByRole("button", { name: "排队消息 1 的更多操作" })).toBeTruthy();
    expect(within(queue).getByRole("button", { name: "删除排队消息 1" })).toBeTruthy();
  });

  test("补充确认超时后仍保留正文消息，回合结束也不擅自重复发送", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-timeout",
      status: "running",
      completedAt: null,
      errorReason: null,
      events: [{ sequence: 1, eventType: "turn.started", phase: "reply", summary: "模型开始处理本轮请求", createdAt: 1 }],
    };
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [activeRun], onMessage });
    let runs = [activeRun];
    let rejectSteer: (reason?: unknown) => void = () => undefined;
    const pendingSteer = new Promise<never>((_, reject) => { rejectSteer = reject; });
    const originalInvoke = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages: [existingMessage], runs };
      if (command === "ai_runtime_activity_snapshot") return { stateRevision: 0, conversations: snapshot(conversationContext()).conversations, messages: [existingMessage], runs, permissionRequests: [], missingRunIds: [] };
      if (command === "ai_message_steer") return pendingSteer;
      if (command === "ai_message_submit") return { status: "recorded", run: { ...activeRun, runId: "run-after-timeout", inputMessageId: "message-after-timeout" } };
      return originalInvoke(command, args);
    });

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "超时后继续" } });
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
    const queue = screen.getByRole("region", { name: "待发送的后续指令" });
    fireEvent.click(within(queue).getByRole("button", { name: "补充回复：排队 1" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_steer")).toBe(true));
    expect(screen.getByText("超时后继续").closest('[aria-label="用户消息"]')).toBeTruthy();
    expect(screen.queryByText("正在确认补充")).toBeNull();
    expect(screen.queryByRole("region", { name: "待发送的后续指令" })).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);

    runs = [{ ...activeRun, status: "completed", completedAt: Date.now() }];
    act(() => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
    await waitFor(() => expect(screen.queryByRole("button", { name: "停止回复" })).toBeNull());
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
    rejectSteer(new Error("codex_harness_steer_timed_out"));
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith(expect.stringContaining("接收状态尚未确认"), "info"));
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
    expect(screen.getByText("超时后继续").closest('[aria-label="用户消息"]')).toBeTruthy();
    expect(screen.queryByText("等待发送")).toBeNull();
  });

  test.each(["正文结束", "归类收尾", "记忆收尾"])("%s时补充入口保留，点击后立即提交并只显示一次", async (phase) => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-classifying",
      status: "running",
      threadWriterActive: true,
      completedAt: null,
      errorReason: null,
      events: [{ sequence: 1, eventType: "turn.started", phase: "reply", summary: "模型开始处理本轮请求", createdAt: 1 }],
    };
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [activeRun], onMessage });
    let runs = [activeRun];
    let messages = [existingMessage];
    const base = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages, runs };
      if (command === "ai_runtime_activity_snapshot") return { stateRevision: 0, conversations: snapshot(conversationContext()).conversations, messages, runs, permissionRequests: [], missingRunIds: [] };
      if (command === "ai_message_steer") {
        const messageId = String(args!.request!.clientMessageId);
        const nextRun = { ...activeRun, runId: "run-next", status: "queued" as const, threadWriterActive: false, inputMessageId: messageId };
        runs = [...runs, nextRun];
        messages = [...messages, { ...existingMessage, messageId, content: String(args!.request!.content), runId: nextRun.runId, createdAt: Date.now() }];
        return nextRun;
      }
      return base(command, args);
    });
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "下一条问题" } });
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
    const queue = screen.getByRole("region", { name: "待发送的后续指令" });
    const supplement = within(queue).getByRole("button", { name: "补充回复：排队 1" }) as HTMLButtonElement;
    expect(supplement.disabled).toBe(false);

    runs = [{ ...activeRun,
      status: phase === "记忆收尾" ? "completed" : "running",
      events: [...activeRun.events,
        { sequence: 2, eventType: "turn.completed", phase: "reply", summary: "正文已结束", createdAt: 2 },
        ...(phase === "归类收尾" ? [{ sequence: 3, eventType: "harness.classification_started", phase: "classification" as const, summary: "开始生成工作归类", createdAt: 3 }] : []),
      ],
    }];
    act(() => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
    await waitFor(() => expect(supplement.title).toBe("立即发送补充"));
    expect(within(queue).getByRole("button", { name: "补充回复：排队 1" })).toBe(supplement);
    expect(supplement.disabled).toBe(false);
    fireEvent.click(supplement);

    expect(screen.queryByRole("region", { name: "待发送的后续指令" })).toBeNull();
    expect(screen.getByText("下一条问题").closest('[aria-label="用户消息"]')).toBeTruthy();
    expect(screen.queryByText("等待发送")).toBeNull();
    await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === "ai_message_steer")).toHaveLength(1));
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);

    runs = [{ ...runs[0], status: "completed", threadWriterActive: false }, { ...runs[1], status: "running", threadWriterActive: true }];
    act(() => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
    await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === "ai_message_steer")).toHaveLength(1));
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
    await waitFor(() => expect(screen.queryByText("等待发送")).toBeNull());
    expect(screen.getAllByLabelText("用户消息").filter((message) => within(message).queryByText("下一条问题"))).toHaveLength(1);
    expect(invoke.mock.calls.find(([command]) => command === "ai_message_steer")?.[1]?.request).toMatchObject({ runId: "run-classifying", content: "下一条问题" });
  });

  test("正文尚未开始时补充入口保留并说明原因，开始后原位恢复可用", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun, runId: "run-preparing", status: "running", completedAt: null, errorReason: null, events: [],
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [activeRun] });
    let runs = [activeRun];
    const base = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages: [existingMessage], runs };
      if (command === "ai_runtime_activity_snapshot") return { stateRevision: 0, conversations: snapshot(conversationContext()).conversations, messages: [existingMessage], runs, permissionRequests: [], missingRunIds: [] };
      return base(command, args);
    });
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "补充约束" } });
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
    const supplement = screen.getByRole("button", { name: "补充回复：排队 1" }) as HTMLButtonElement;
    expect(supplement.disabled).toBe(true);
    expect(supplement.title).toBe("等待当前回复开始");
    fireEvent.click(supplement);
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_steer")).toBe(false);
    runs = [{ ...activeRun, events: [{ sequence: 1, eventType: "turn.started", phase: "reply", summary: "开始回复", createdAt: 1 }] }];
    act(() => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
    await waitFor(() => expect(supplement.disabled).toBe(false));
    expect(screen.getByRole("button", { name: "补充回复：排队 1" })).toBe(supplement);
  });

  test("点击与回复结束撞在一起时，由服务端接收，前端不再次发送", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-active",
      status: "running",
      completedAt: null,
      errorReason: null,
      events: [{ sequence: 1, eventType: "turn.started", phase: "reply", summary: "模型开始处理本轮请求", createdAt: 1 }],
    };
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [activeRun], onMessage });
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "下一条问题" } });
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages: [existingMessage], runs: [activeRun] };
      if (command === "ai_runtime_activity_snapshot") return { stateRevision: 0, conversations: snapshot(conversationContext()).conversations, messages: [existingMessage], runs: [activeRun], permissionRequests: [], missingRunIds: [] };
      if (command === "ai_message_steer") return { ...activeRun, runId: "accepted-next-run", status: "queued", threadWriterActive: false };
      throw new Error(`unexpected command ${command}`);
    });

    const queue = screen.getByRole("region", { name: "待发送的后续指令" });
    fireEvent.click(within(queue).getByRole("button", { name: "补充回复：排队 1" }));
    await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === "ai_message_steer")).toHaveLength(1));
    expect(screen.getByText("下一条问题")).toBeTruthy();
    expect(screen.queryByText("等待发送")).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
    expect(screen.queryByRole("region", { name: "待发送的后续指令" })).toBeNull();
    expect(onMessage).not.toHaveBeenCalledWith(expect.stringContaining("网关"), "error");
  });

  test("后续指令及附件交由宿主持久化调度，界面不另行投递", async () => {
    const activeRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-active",
      model: "Qwen3.8-27B-NVFP4",
      status: "running",
      pid: 123,
      startedAt: Date.now(),
      completedAt: null,
      errorReason: null,
      sessionId: "thread-active",
    };
    let runs = [activeRun];
    let refreshTick: (() => void) | null = null;
    const interval = vi.spyOn(window, "setInterval").mockImplementation((handler) => {
      refreshTick = handler as () => void;
      return 1 as unknown as ReturnType<typeof window.setInterval>;
    });
    invoke.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages: [existingMessage], runs };
      if (command === "ai_message_submit") return { status: "recorded", run: { ...activeRun, runId: "run-follow-up", inputMessageId: "message-follow-up" } };
      if (command === "ai_attachment_create") return { attachmentId: "attachment-queued", fileName: args?.request?.fileName, mimeType: args?.request?.mimeType, mediaType: "image", sizeBytes: args?.request?.sizeBytes, localPath: "" };
      if (command === "ai_attachment_append") return {};
      throw new Error(`unexpected command ${command}`);
    });
    try {
      window.localStorage.setItem("cilai.workbench.ai-last-viewed-conversation", "conv-1");
      render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[selectedFile]} selectedFile={null} launchRequest={null} preferredConversationId={null} onMessage={() => { /* 消息条不参与本次断言 */ }} onOpenAgents={() => { /* 智能体页跳转不参与本次断言 */ }} canCreateProject onCreateProject={() => { /* 不检查新建 */ }} onSelectProject={() => { /* 不检查切换 */ }} projectIndex={new Map()}/>);
      await waitFor(() => expect(refreshTick).not.toBeNull());
      await screen.findByRole("button", { name: /选择模型：Qwen3\.8 27B NVFP4/ });
      const mediaInput = document.querySelector<HTMLInputElement>("input.ai-media-input")!;
      fireEvent.change(mediaInput, { target: { files: [new File([new Uint8Array([1, 2, 3])], "queue.png", { type: "image/png" })] } });
      await screen.findByRole("button", { name: "查看图片 queue.png" });
      fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "下一步检查接口" } });
      fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
      const queue = screen.getByRole("region", { name: "待发送的后续指令" });
      openQueuedEdit(queue);
      const input = screen.getByRole("textbox", { name: "发送给 AI" });
      expect(screen.queryByRole("region", { name: "待发送的后续指令" })).toBeNull();
      expect((input as HTMLTextAreaElement).value).toBe("下一步检查接口");
      fireEvent.change(input, { target: { value: "修改后的下一步检查接口" } });
      fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
      expect(within(screen.getByRole("region", { name: "待发送的后续指令" })).getByText("修改后的下一步检查接口")).toBeTruthy();
      expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);

      runs = [{ ...activeRun, status: "completed", threadWriterActive: true, pid: null, completedAt: Date.now() }];
      await act(async () => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
      expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
      runs = [{ ...runs[0], threadWriterActive: false }];
      act(() => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));

      await act(() => flushConversationSessions());
      expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
      expect(sessionApi.snapshot.values.queuedFollowUps).toEqual([expect.objectContaining({
        conversationId: "conv-1", content: "修改后的下一步检查接口", executionTarget: "model",
        attachments: [expect.objectContaining({ attachment: expect.objectContaining({ attachmentId: "attachment-queued" }), previewUrl: "" })],
      })]);
    } finally {
      interval.mockRestore();
    }
  });

  test("只展示最新异常并通过弹窗查看技术详情，关闭后恢复焦点", async () => {
    const scrollIntoView = vi.spyOn(Element.prototype, "scrollIntoView");
    const newerFailedRun = { ...failedRun, runId: "run-failed-newer", startedAt: failedRun.startedAt + 10_000, completedAt: failedRun.completedAt! + 10_000 };
    newerFailedRun.errorReason = "AI 回复未通过结构化协议校验：ai_response_envelope_invalid_json:expected value at line 1 column 1";
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [failedRun, newerFailedRun] });

    const section = screen.getByRole("region", { name: "最近一次运行异常" });
    expect(within(section).getByText("回复格式校验失败")).toBeTruthy();
    expect(screen.queryByText("模型凭据不可用")).toBeNull();
    expect(screen.queryByRole("heading", { name: "运行记录" })).toBeNull();
    const trigger = within(section).getByRole("button", { name: /查看运行详情/ });
    expect(trigger.getAttribute("aria-haspopup")).toBe("dialog");
    expect(section.querySelector("details")).toBeNull();
    expect(screen.queryByRole("dialog", { name: "运行详情" })).toBeNull();
    trigger.focus();
    fireEvent.click(trigger);
    const dialog = screen.getByRole("dialog", { name: "运行详情" });
    expect(dialog.parentElement).toBe(document.body);
    expect(section.contains(dialog)).toBe(false);
    expect(scrollIntoView).not.toHaveBeenCalled();
    expect(within(dialog).getByText("技术详情")).toBeTruthy();
    expect(dialog.querySelector("pre")?.textContent).toContain("ai_response_envelope_invalid_json");
    expect(document.activeElement).toBe(within(dialog).getByRole("button", { name: "关闭" }));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "运行详情" })).toBeNull();
    expect(document.activeElement).toBe(trigger);
    fireEvent.click(trigger);
    fireEvent.mouseDown(screen.getByRole("dialog", { name: "运行详情" }).querySelector("h3")!);
    expect(screen.getByRole("dialog", { name: "运行详情" })).toBeTruthy();
    expect(fireEvent.mouseDown(screen.getByRole("dialog", { name: "运行详情" }))).toBe(false);
    expect(screen.queryByRole("dialog", { name: "运行详情" })).toBeNull();
    expect(document.activeElement).toBe(trigger);
    fireEvent.click(trigger);
    fireEvent.click(within(screen.getByRole("dialog", { name: "运行详情" })).getByRole("button", { name: "关闭" }));
    expect(screen.queryByRole("dialog", { name: "运行详情" })).toBeNull();
    scrollIntoView.mockRestore();
  });

  test("历史运行的模型已下架时明确提示且不自动切换", async () => {
    const unavailableModel = "removed-model-id";
    await renderWorkbench(conversationContext(), {
      messages: [existingMessage],
      runs: [{ ...failedRun, model: unavailableModel }],
    });

    const picker = await screen.findByRole("button", { name: "选择模型：原模型已不可用" });
    expect(picker.querySelector("span")?.title).toBe(unavailableModel);
    expect(screen.queryByRole("button", { name: "选择模型：选择模型" })).toBeNull();
  });

  test("模型额度不足时直接显示明确原因", async () => {
    const reason = "模型额度不足：本次需预占 100，当前账号可用 80，缺少 20；企业额度池需先分配到员工账号后才能使用";
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [{ ...failedRun, errorReason: reason }] });

    expect(
      screen.getByText(reason, { selector: ".ai-run-record-summary" }),
    ).toBeTruthy();
    expect(screen.queryByText("运行未能完成")).toBeNull();
  });

  test("完成后的执行过程放在对应回复前并在消息头固定展示最终用时", async () => {
    const completedRun = { ...failedRun, runId: "run-completed", status: "completed" as const, startedAt: failedRun.startedAt + 10_000, completedAt: failedRun.completedAt! + 10_000, errorReason: null };
    const reply = { ...existingMessage, messageId: "reply-completed", role: "assistant" as const, content: "这是最终回复", runId: completedRun.runId, createdAt: completedRun.completedAt! };
    await renderWorkbench(conversationContext(), { messages: [existingMessage, reply], runs: [failedRun, completedRun] });

    const message = screen.getByLabelText("AI 消息");
    const processToggle = within(message).getByRole("button", { name: "展开执行过程：执行过程" });
    expect(processToggle.textContent).toBe("执行过程");
    expect([...processToggle.children].map((node) => node.tagName)).toEqual(["STRONG", "svg"]);
    expect(processToggle.querySelector("path")?.getAttribute("d")).toBe("m10 7 5 5-5 5");
    fireEvent.click(processToggle);
    expect(processToggle.getAttribute("aria-expanded")).toBe("true");
    expect(processToggle.querySelector("path")?.getAttribute("d")).toBe("m7 10 5 5 5-5");
    expect(message.textContent).not.toContain("输出 103 token");
    expect(within(message).getByLabelText("用时 00:05").textContent).toBe("用时 00:05");
    expect(processToggle.compareDocumentPosition(within(message).getByText("这是最终回复")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.queryByRole("region", { name: "最近一次运行" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "运行记录" })).toBeNull();
  });

  test("当前回复产生的文件作为独立结果显示在正文底部", async () => {
    const completedRun = {
      ...failedRun,
      runId: "run-inline-artifact",
      status: "completed" as const,
      errorReason: null,
      artifacts: [{ artifactId: "artifact-inline", artifactType: "document", label: "企业AI落地秘诀.md", path: "企业AI落地秘诀.md" }],
    };
    const reply = { ...existingMessage, messageId: "reply-inline-artifact", role: "assistant" as const, content: "文章已经写好了。", runId: completedRun.runId, createdAt: 2 };
    const generatedFile = { ...selectedFile, localFileId: "generated-md", relativePath: "企业AI落地秘诀.md" };
    const onOpenFile = vi.fn();
    await renderWorkbench(conversationContext(), { messages: [existingMessage, reply], runs: [completedRun], files: [generatedFile], onOpenFile });

    const message = screen.getByLabelText("AI 消息");
    const content = within(message).getByText("文章已经写好了。");
    const artifacts = within(message).getByRole("region", { name: "本次运行文件" });
    expect(content.compareDocumentPosition(artifacts) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(artifacts).getByText("已生成 1 个文件")).toBeTruthy();
    fireEvent.click(within(artifacts).getByRole("button", { name: "打开文件 企业AI落地秘诀.md" }));
    expect(onOpenFile).toHaveBeenCalledWith("generated-md", workspace.workspaceId);
  });

  test("生成图片使用独立画廊并可切换、全屏查看", async () => {
    const completedRun = {
      ...failedRun,
      runId: "run-inline-image",
      status: "completed" as const,
      errorReason: null,
      artifacts: [
        { artifactId: "artifact-image", artifactType: "image", label: "生成图片.png", path: "生成图片/生成图片.png" },
        { artifactId: "artifact-image-2", artifactType: "image", label: "生成图片2.png", path: "生成图片/生成图片2.png" },
      ],
    };
    const reply = { ...existingMessage, messageId: "reply-inline-image", role: "assistant" as const, content: "图片已生成。", runId: completedRun.runId, createdAt: 2 };
    await renderWorkbench(conversationContext(), { messages: [existingMessage, reply], runs: [completedRun] });

    const artifacts = within(screen.getByLabelText("AI 消息")).getByRole("region", { name: "本次运行图片" });
    expect(within(artifacts).getByText("已生成 2 张图片")).toBeTruthy();
    const button = within(artifacts).getByRole("button", { name: /^查看图片/ });
    await waitFor(() => expect(button.querySelector("img")?.getAttribute("src")).toBe("blob:attachment-preview"));
    expect(invoke).toHaveBeenCalledWith("ai_artifact_read_image", {
      runId: completedRun.runId,
      artifactId: "artifact-image",
    });
    fireEvent.click(within(artifacts).getByRole("listitem", { name: "选择图片 2：生成图片2.png" }));
    const second = within(artifacts).getByRole("button", { name: "查看图片 生成图片2.png" });
    fireEvent.click(second);
    expect(screen.getByRole("dialog", { name: "查看图片 生成图片2.png" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "上一张" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "下一张" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "关闭图片预览" }));
    expect(screen.queryByRole("dialog", { name: "查看图片 生成图片2.png" })).toBeNull();
  });

  test("生图工具执行期间显示图片生成和保存状态", async () => {
    const generatingRun: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun,
      runId: "run-image-generating",
      status: "running",
      completedAt: null,
      errorReason: null,
      events: [{ sequence: 1, eventType: "item.started", itemId: "image-1", phase: "reply", kind: "mcp", status: "started", summary: "正在使用 generate_image", createdAt: 1 }],
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [generatingRun] });

    const pending = screen.getByRole("status", { name: "图片生成中" });
    expect(within(pending).getAllByText("正在生成图片")).toHaveLength(2);
    expect(imageGenerationProgress({ ...generatingRun, events: [{ ...generatingRun.events[0], eventType: "item.completed", status: "completed", summary: "已使用 generate_image" }] })).toBe("saving");
    expect(imageGenerationProgress({ ...generatingRun, events: [{ ...generatingRun.events[0], eventType: "item.completed", status: "failed", summary: "generate_image 调用失败" }] })).toBeNull();
  });

  test("Windows 绝对路径、文件 URL 和行号引用可以定位同一工作区文件", () => {
    const file = { ...selectedFile, relativePath: "产物/报告.md" };
    for (const path of ["D:/工作区/产物/报告.md", "D:\\工作区\\产物\\报告.md", "/D:/工作区/产物/报告.md", "file:///D:/工作区/产物/报告.md", "d:/工作区/产物/报告.md:12:3", "file:///d:/工作区/产物/报告.md#L12"]) {
      expect(markdownWorkspaceFile(path, "D:\\工作区", [file]), path).toBe(file);
    }
    for (const path of ["D:/工作区外/产物/报告.md", "D:/工作区/../产物/报告.md", "file:///D:/工作区/%2e%2e/产物/报告.md", "file://other/工作区/产物/报告.md", "D:产物/报告.md"]) {
      expect(markdownWorkspaceFile(path, "D:\\工作区", [file]), path).toBeNull();
    }
  });

  test.each(["windows", "macos"])("%s 对话内绝对路径和未索引文件可打开，产物卡片可右键", async (hostPlatform) => {
    const root = hostPlatform === "windows" ? "D:\\工作区" : "/Users/me/工作区";
    const path = `${root.replaceAll("\\", "/")}/产物/报告.md`;
    const completedRun = { ...failedRun, runId: "run-file-actions", status: "completed" as const, errorReason: null,
      artifacts: [{ artifactId: "artifact-file-actions", artifactType: "document", label: "报告.md", path }],
    };
    const reply = { ...existingMessage, messageId: "file-actions", role: "assistant" as const, content: `[报告](${path}) [未索引文件](产物/新文件.md)`, runId: completedRun.runId, createdAt: 2 };
    const onOpenWorkspacePath = vi.fn().mockResolvedValue(true);
    await renderWorkbench(conversationContext({ workspaceRoot: root }), { messages: [reply], runs: [completedRun], files: [], hostPlatform, onOpenWorkspacePath });
    fireEvent.click(screen.getByRole("link", { name: "报告" }));
    await waitFor(() => expect(onOpenWorkspacePath).toHaveBeenCalledWith("产物/报告.md", workspace.workspaceId));
    fireEvent.click(screen.getByRole("link", { name: "未索引文件" }));
    await waitFor(() => expect(onOpenWorkspacePath).toHaveBeenCalledWith("产物/新文件.md", workspace.workspaceId));
    const card = screen.getByRole("button", { name: "打开文件 报告.md" });
    fireEvent.contextMenu(card);
    expect(screen.getByRole("menuitem", { name: "另存为…" })).toBeTruthy();
    fireEvent.click(screen.getByRole("menuitem", { name: hostPlatform === "windows" ? "在文件管理器中显示" : "在访达中显示" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("workspace_open_item", { localRoot: root, relativePath: "产物/报告.md", openMode: "finder" }));
  });

  test("回复中的工作区文件链接按预览能力打开文件页或系统文件管理器", async () => {
    const previewable = { ...selectedFile, localFileId: "file-html", relativePath: "页面/方案.html" };
    const external = { ...selectedFile, localFileId: "file-zip", relativePath: "附件/材料.zip" };
    const reply = { ...existingMessage, messageId: "reply-links", role: "assistant" as const, content: "[查看方案](页面/方案.html) [查看附件](附件/材料.zip) [访问官网](https://example.com)", runId: null, createdAt: 2 };
    const onOpenFile = vi.fn();
    expect(markdownWorkspaceFile("页面/%E6%96%B9%E6%A1%88.html", workspace.localRoot, [previewable])).toBe(previewable);
    expect(markdownWorkspaceFile("https://example.com/file.html", workspace.localRoot, [previewable])).toBeNull();
    expect(markdownWorkspaceFile("../方案.html", workspace.localRoot, [previewable])).toBeNull();
    await renderWorkbench(conversationContext(), { messages: [existingMessage, reply], files: [previewable, external], onOpenFile });

    fireEvent.click(screen.getByRole("link", { name: "查看方案" }));
    expect(onOpenFile).toHaveBeenCalledWith("file-html", workspace.workspaceId);

    fireEvent.click(screen.getByRole("link", { name: "查看附件" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("workspace_open_item", {
      localRoot: workspace.localRoot,
      relativePath: "附件/材料.zip",
      openMode: "finder",
    }));
    expect(screen.queryByRole("link", { name: "访问官网" })).toBeNull();
    expect(screen.getByText("访问官网").tagName).toBe("SPAN");
  });

  test("未索引文件打开失败显示原因，越界文件不提供操作", async () => {
    const reply = { ...existingMessage, messageId: "unindexed-file", role: "assistant" as const, content: "[新报告](产物/报告.md) [越界文件](../secret.md)", runId: null, createdAt: 2 };
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { messages: [reply], files: [], onMessage, onOpenWorkspacePath: vi.fn().mockRejectedValue(new Error("file_missing")) });
    fireEvent.click(screen.getByRole("link", { name: "新报告" }));
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith("无法打开文件：Error: file_missing", "error"));
    expect(screen.queryByRole("link", { name: "越界文件" })).toBeNull();
  });

  test("有版本快照的产物右键通过产物编号打开，不误用当前工作区文件", async () => {
    const artifact = { artifactId: "version-menu", artifactType: "document", label: "报告.md", path: "产物/报告.md", version: { digest: "digest-123", sizeBytes: 100, createdAt: 1 } };
    const completedRun = { ...failedRun, runId: "run-version-menu", status: "completed" as const, errorReason: null, artifacts: [artifact] };
    const reply = { ...existingMessage, messageId: "version-menu", role: "assistant" as const, content: "报告已生成。", runId: completedRun.runId, createdAt: 2 };
    await renderWorkbench(conversationContext(), { messages: [reply], runs: [completedRun] });
    fireEvent.contextMenu(screen.getByRole("button", { name: "打开文件 报告.md" }));
    expect(screen.queryByRole("menuitem", { name: "查看版本与反馈" })).toBeNull();
    fireEvent.click(screen.getByRole("menuitem", { name: "打开文件" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_artifact_open", { runId: completedRun.runId, artifactId: artifact.artifactId, openMode: "default" }));
    expect(invoke.mock.calls.some(([command]) => command === "workspace_open_item")).toBe(false);
  });

  test.each(["windows", "macos"])("%s 正文和侧栏的版本产物直接打开登记文件，不弹出反馈或候选提交", async (hostPlatform) => {
    const artifacts = [
      { artifactId: "direct-document", artifactType: "document", label: "报告.md", path: "产物/报告.md", version: { digest: "document-v1", sizeBytes: 100, createdAt: 1 } },
      { artifactId: "direct-image", artifactType: "image", label: "图片.png", path: "产物/图片.png", version: { digest: "image-v1", sizeBytes: 200, createdAt: 1 } },
    ];
    const completedRun = { ...failedRun, runId: "run-direct-artifact", status: "completed" as const, errorReason: null, artifacts };
    const reply = { ...existingMessage, messageId: "direct-artifact", role: "assistant" as const, content: "文件已生成。", runId: completedRun.runId, createdAt: 2 };
    const onOpenFile = vi.fn();
    const onOpenWorkspacePath = vi.fn().mockResolvedValue(true);
    await renderWorkbench(conversationContext({ projectId: "proj-1" }), {
      messages: [reply], runs: [completedRun], hostPlatform, onOpenFile, onOpenWorkspacePath,
      files: artifacts.map((artifact) => ({ ...selectedFile, localFileId: artifact.artifactId, relativePath: artifact.path })),
    });

    fireEvent.click(screen.getByRole("button", { name: "打开文件 报告.md" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_artifact_open", { runId: completedRun.runId, artifactId: "direct-document", openMode: "default" }));
    fireEvent.click(screen.getByRole("button", { name: "显示产物与来源" }));
    const panel = screen.getByRole("complementary", { name: "对话产物与上下文" });
    fireEvent.click(within(panel).getByRole("button", { name: "打开产物 图片.png" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_artifact_open", { runId: completedRun.runId, artifactId: "direct-image", openMode: "default" }));

    expect(screen.queryByRole("dialog", { name: "产物版本" })).toBeNull();
    expect(screen.queryByLabelText("版本反馈")).toBeNull();
    expect(screen.queryByText("提交候选版本")).toBeNull();
    expect(onOpenFile).not.toHaveBeenCalled();
    expect(onOpenWorkspacePath).not.toHaveBeenCalled();
    expect(invoke.mock.calls.some(([command, args]) => command === "app_server_request" && args?.method === "deliverable/list")).toBe(false);
    expect(invoke.mock.calls.some(([command]) => command === "ai_artifact_feedback" || command === "ai_artifact_submit_candidate")).toBe(false);
  });

  test("版本产物直接打开失败时提示错误，不改为打开当前工作区文件", async () => {
    const artifact = { artifactId: "missing-snapshot", artifactType: "document", label: "报告.md", path: "产物/报告.md", version: { digest: "missing-v1", sizeBytes: 100, createdAt: 1 } };
    const completedRun = { ...failedRun, runId: "run-missing-artifact", status: "completed" as const, errorReason: null, artifacts: [artifact] };
    const onMessage = vi.fn();
    const onOpenWorkspacePath = vi.fn().mockResolvedValue(true);
    await renderWorkbench(conversationContext(), { runs: [completedRun], onMessage, onOpenWorkspacePath });
    const originalImplementation = invoke.getMockImplementation()!;
    invoke.mockImplementation((command, args) => command === "ai_artifact_open" ? Promise.reject(new Error("ai_artifact_unavailable")) : originalImplementation(command, args));
    fireEvent.click(screen.getByRole("button", { name: "显示产物与来源" }));
    fireEvent.click(screen.getByRole("button", { name: "打开产物 报告.md" }));
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith("无法打开产物：Error: ai_artifact_unavailable", "error"));
    expect(onOpenWorkspacePath).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog", { name: "产物版本" })).toBeNull();
  });

  test("云端文件的右键打开不会调用本地文件程序", async () => {
    const reply = { ...existingMessage, messageId: "cloud-file-menu", role: "assistant" as const, content: "[压缩包](资料.zip)", runId: null, createdAt: 2 };
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext({ teamSpaceId: "cloud-team" }), { messages: [reply], onMessage });
    fireEvent.contextMenu(screen.getByRole("link", { name: "压缩包" }));
    expect(screen.queryByRole("menuitem", { name: "默认程序打开" })).toBeNull();
    fireEvent.click(screen.getByRole("menuitem", { name: "打开文件" }));
    expect(onMessage).toHaveBeenCalledWith("云端文件请在文件页预览。", "error");
    expect(invoke.mock.calls.some(([command]) => command === "workspace_open_item")).toBe(false);
  });

  test("旧应用下载地址不开放，本地文件链接仍可打开且不查询企业来源", async () => {
    const reply = { ...existingMessage, messageId: "local-after-retirement", role: "assistant" as const,
      content: `[本地文件](${selectedFile.relativePath}) [旧应用下载](https://app.example.com/exports/file) [其他网站](https://other.example.com/file.gif)`, runId: null, createdAt: 2 };
    const onOpenFile = vi.fn();
    await renderWorkbench(conversationContext(), { messages: [reply], onOpenFile });
    expect(screen.queryByRole("link", { name: "旧应用下载" })).toBeNull();
    expect(screen.queryByRole("link", { name: "其他网站" })).toBeNull();
    fireEvent.click(screen.getByRole("link", { name: "本地文件" }));
    expect(onOpenFile).toHaveBeenCalledWith(selectedFile.localFileId, workspace.workspaceId);
    expect(invoke.mock.calls.some(([command]) => command.startsWith("application_container_"))).toBe(false);
  });

  test("回复中的本地文件链接右键提供打开、另存、复制和定位", async () => {
    const generatedImage = { ...selectedFile, localFileId: "generated-image", relativePath: "生成图片/人像.png" };
    const completedRun = { ...failedRun, runId: "run-image-link", status: "completed" as const, completedAt: 2, errorReason: null,
      artifacts: [{ artifactId: "artifact-image-link", artifactType: "image", label: "人像.png", path: generatedImage.relativePath }],
    };
    const reply = { ...existingMessage, messageId: "reply-image-link", role: "assistant" as const, content: "[人像图](生成图片/人像.png)", runId: completedRun.runId, createdAt: 2 };
    await renderWorkbench(conversationContext(), { messages: [existingMessage, reply], runs: [completedRun], files: [] });

    const link = screen.getByRole("link", { name: "人像图" });
    fireEvent.click(link);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("workspace_open_item", {
      localRoot: workspace.localRoot,
      relativePath: "生成图片/人像.png",
      openMode: "default",
    }));
    fireEvent.contextMenu(link, { clientX: 120, clientY: 140 });
    expect(screen.getByRole("menuitem", { name: "打开文件" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "默认程序打开" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "另存为…" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "复制路径" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "复制文件内容" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "在访达中显示" })).toBeTruthy();

    fireEvent.click(screen.getByRole("menuitem", { name: "默认程序打开" }));
    expect(invoke).toHaveBeenCalledWith("workspace_open_item", {
      localRoot: workspace.localRoot,
      relativePath: "生成图片/人像.png",
      openMode: "default",
    });

    fireEvent.contextMenu(link, { clientX: 120, clientY: 140 });
    fireEvent.click(screen.getByRole("menuitem", { name: "另存为…" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("workspace_save_copy", {
      localRoot: workspace.localRoot,
      relativePath: "生成图片/人像.png",
    }));

    fireEvent.contextMenu(link, { clientX: 120, clientY: 140 });
    fireEvent.click(screen.getByRole("menuitem", { name: "复制文件内容" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("workspace_copy_item_content", {
      localRoot: workspace.localRoot,
      relativePath: "生成图片/人像.png",
    }));

    fireEvent.contextMenu(link, { clientX: 120, clientY: 140 });
    fireEvent.click(screen.getByRole("menuitem", { name: "复制路径" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("/tmp/ws-1/生成图片/人像.png"));
  });

  test("Windows 文件定位失败使用文件管理器文案，未知平台不发起定位", async () => {
    const external = { ...selectedFile, localFileId: "file-zip", relativePath: "附件/材料.zip" };
    const reply = { ...existingMessage, messageId: "reply-file-manager", role: "assistant" as const, content: "[查看附件](附件/材料.zip)", runId: null, createdAt: 2 };
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { files: [external], hostPlatform: "windows", messages: [existingMessage, reply], onMessage, workspaceOpenError: new Error("open failed") });
    fireEvent.click(screen.getByRole("link", { name: "查看附件" }));
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith("无法在文件管理器中显示：Error: open failed", "error"));

    cleanup();
    invoke.mockClear();
    onMessage.mockClear();
    await renderWorkbench(conversationContext(), { files: [external], hostPlatform: "other", messages: [existingMessage, reply], onMessage });
    fireEvent.click(screen.getByRole("link", { name: "查看附件" }));
    expect(onMessage).toHaveBeenCalledWith("当前平台不支持文件定位。", "error");
    expect(invoke.mock.calls.some(([command]) => command === "workspace_open_item")).toBe(false);
  });

  test("尚未进入文件列表的可预览产物会刷新索引后打开文件页", async () => {
    const completedRun = {
      ...failedRun,
      runId: "run-new-artifact",
      status: "completed" as const,
      errorReason: null,
      artifacts: [{ artifactId: "artifact-new", artifactType: "document", label: "新报告.md", path: "产物/新报告.md" }],
    };
    const reply = { ...existingMessage, messageId: "reply-new-artifact", role: "assistant" as const, content: "报告已生成。", runId: completedRun.runId, createdAt: 2 };
    const onOpenWorkspacePath = vi.fn().mockResolvedValue(true);
    await renderWorkbench(conversationContext(), { messages: [existingMessage, reply], runs: [completedRun], files: [], onOpenWorkspacePath });

    fireEvent.click(screen.getByRole("button", { name: "打开文件 新报告.md" }));
    await waitFor(() => expect(onOpenWorkspacePath).toHaveBeenCalledWith("产物/新报告.md", workspace.workspaceId));
    expect(invoke.mock.calls.some(([command]) => command === "ai_artifact_open")).toBe(false);
  });

  test("工作归类失败时保留回复并显示非阻断提示", async () => {
    const completedRun = {
      ...failedRun,
      runId: "run-classification-failed",
      status: "completed" as const,
      output: "已经生成的正文",
      errorReason: null,
      classificationStatus: "failed" as const,
      classificationError: "AI 工作归类执行失败: codex_harness_turn_failed:unexpected status 502 Bad Gateway: 模型回复中断, url: http://127.0.0.1:60582/v1/responses, request id: c7859279-6aa1-4a0e-8d85-4e6e9b797146",
    };
    const reply = {
      ...existingMessage,
      messageId: "reply-classification-failed",
      role: "assistant" as const,
      content: "已经生成的正文",
      runId: completedRun.runId,
      createdAt: 2,
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage, reply], runs: [completedRun] });

    expect(within(screen.getByLabelText("AI 消息")).getByText("已经生成的正文")).toBeTruthy();
    const section = screen.getByRole("region", { name: "最近一次运行提示" });
    expect(within(section).getByText("工作归类失败，任务状态未更新")).toBeTruthy();
    expect(within(section).getByText("回复已生成")).toBeTruthy();
    expect(within(section).queryByText("已完成")).toBeNull();
    expect(screen.queryByRole("region", { name: "最近一次运行异常" })).toBeNull();
    fireEvent.click(within(section).getByRole("button", { name: /查看运行详情/ }));
    const dialog = screen.getByRole("dialog", { name: "运行详情" });
    expect(within(dialog).getByText("归类详情")).toBeTruthy();
    expect(within(dialog).getByText("Qwen3.6 35B A3B NVFP4")).toBeTruthy();
    expect(within(dialog).queryByText("Qwen3.6-35B-A3B-NVFP4")).toBeNull();
    expect(within(dialog).getByText("AI-CLASSIFY-001")).toBeTruthy();
    expect(within(dialog).getByText("c7859279-6aa1-4a0e-8d85-4e6e9b797146")).toBeTruthy();
    expect(dialog.textContent).not.toContain("127.0.0.1");
    expect(dialog.textContent).not.toContain("codex_harness_turn_failed");
    writeText.mockRejectedValueOnce(new Error("clipboard unavailable"));
    fireEvent.click(within(dialog).getByRole("button", { name: "复制详情" }));
    await within(dialog).findByText("复制失败，请重试");
    fireEvent.click(within(dialog).getByRole("button", { name: "复制详情" }));
    await within(dialog).findByText("已复制");
    expect(writeText).toHaveBeenLastCalledWith("工作归类暂时失败，本次回复已保留，任务状态未更新。\n错误代码：AI-CLASSIFY-001\n请求编号：c7859279-6aa1-4a0e-8d85-4e6e9b797146");
  });

  test("已被替换的运行不再作为当前会话错误展示", async () => {
    await renderWorkbench(conversationContext(), {
      messages: [existingMessage],
      runs: [{ ...failedRun, superseded: true }],
    });

    expect(screen.queryByRole("region", { name: "最近一次运行异常" })).toBeNull();
  });

  test("上下文详情展示来源类别且整个占用模块均可展开", async () => {
    const run = { ...failedRun, status: "completed" as const, errorReason: null,
      skillIds: ["skill-summary"], toolPermissions: ["read-files", "search-workspace"],
      events: [{ sequence: 1, eventType: "item.started", summary: "开始调用已授权工具", createdAt: 1 }], usage: {
      inputTokens: 20335, cachedInputTokens: 100, outputTokens: 172,
      reply: { inputTokens: 10665, cachedInputTokens: 100, outputTokens: 9 },
      classification: { inputTokens: 9670, cachedInputTokens: 0, outputTokens: 163 },
      context: { tokens: 10674, windowTokens: 249036, composition: {
        conversationTokens: 4500, systemPromptTokens: 3000, toolTokens: 1200,
        skillTokens: 600, mcpTokens: 374, internalTokens: 1000,
      } },
    } };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [run] });
    expect(screen.queryByRole("region", { name: "最近一次运行" })).toBeNull();
    expect(screen.queryByRole("button", { name: "查看本次运行用量明细" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /查看当前对话上下文占用/ }));
    const detail = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(detail).getByText("10,674 / 243K · 4.3%")).toBeTruthy();
    expect(within(detail).queryByText("Qwen3.6 35B A3B NVFP4")).toBeNull();
    const toggle = within(detail).getByRole("button", { name: "展开上下文详细统计" });
    expect(toggle.querySelector("svg")).toBeTruthy();
    fireEvent.click(within(toggle).getByText("已占用"));
    expect(within(detail).getByText("对话内容")).toBeTruthy();
    expect(within(detail).getByText("系统提示词")).toBeTruthy();
    expect(within(detail).getByText("工具上下文")).toBeTruthy();
    expect(within(detail).getByText("Skill")).toBeTruthy();
    expect(within(detail).getByText("MCP")).toBeTruthy();
    expect(within(detail).getByText("Codex 内部")).toBeTruthy();
    expect(within(detail).getByText("分类估算")).toBeTruthy();
    expect(within(detail).getByText("4,500 · 1.8%")).toBeTruthy();
    expect(within(detail).getByText("3,000 · 1.2%")).toBeTruthy();
    expect(within(detail).getByText("1,200 · 0.5%")).toBeTruthy();
    expect(within(detail).getByText("600 · 0.2%")).toBeTruthy();
    expect(within(detail).getByText("374 · 0.2%")).toBeTruthy();
    expect(within(detail).getByText("1,000 · 0.4%")).toBeTruthy();
    expect(detail.querySelectorAll(".ai-model-usage-bar-used > i")).toHaveLength(6);
    expect(within(detail).queryByText("剩余空间")).toBeNull();
    expect(within(detail).queryByText("正文回复")).toBeNull();
    expect(within(detail).queryByText("后台归类")).toBeNull();
    fireEvent.click(detail.querySelector(".ai-model-usage-bar")!);
    expect(within(detail).queryByText("系统提示词")).toBeNull();
  });

  test.each([null, 0, 374])("MCP 用量为 %s 时区分缺失统计和真实零值", async (mcpTokens) => {
    const run = { ...failedRun, status: "completed" as const, errorReason: null,
      usage: { ...failedRun.usage, context: { tokens: 10_674, windowTokens: 249_036, composition: {
        conversationTokens: 4500, systemPromptTokens: 3000, toolTokens: 1200,
        skillTokens: 600, mcpTokens, internalTokens: 1374 - (mcpTokens ?? 0),
      } } } };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [run] });
    fireEvent.click(screen.getByRole("button", { name: "查看当前对话上下文占用：4.3%" }));
    const detail = screen.getByRole("region", { name: "当前对话上下文占用" });
    fireEvent.click(within(detail).getByRole("button", { name: "展开上下文详细统计" }));
    const mcpRow = within(detail).getByText("MCP").closest("div")!;
    expect(within(mcpRow).getByText(mcpTokens === null ? "未提供 · 未知" : mcpTokens === 0 ? "0 · 0.0%" : "374 · 0.2%")).toBeTruthy();
    expect(within(detail).getByText("10,674 / 243K · 4.3%")).toBeTruthy();
    const counts = [...detail.querySelectorAll("dd")];
    expect(counts).toHaveLength(6);
    if (mcpTokens !== null) {
      for (const count of counts) expect(count.textContent).toMatch(/^[\d,]+ · \d+\.\d%$/);
      expect(counts.reduce((total, count) => total + Number(count.textContent!.split(" · ")[0].replaceAll(",", "")), 0)).toBe(10_674);
    }
  });

  test("历史合计不可冒充上下文占用，缺少快照时保留未知", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [{ ...failedRun, status: "completed", errorReason: null }] });
    const trigger = screen.getByRole("button", { name: "查看当前对话上下文占用：未知" }) as HTMLButtonElement;
    expect(trigger.disabled).toBe(false);
    expect(trigger.querySelector(".ai-context-usage-value")?.getAttribute("stroke-dasharray")?.startsWith("0 ")).toBe(true);
    fireEvent.click(trigger);
    const detail = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(detail).getByText("未提供 / 未知 · 未知")).toBeTruthy();
    fireEvent.click(within(detail).getByRole("button", { name: "展开上下文详细统计" }));
    expect(within(detail).getAllByText("未提供 · 未知")).toHaveLength(6);
    expect(within(detail).queryByText("未提供分类")).toBeTruthy();
  });

  test.each([null, 0])("请求窗口容量为 %s 时保留已有数值，容量和占比显示未知", async (windowTokens) => {
    const run = { ...failedRun, status: "completed" as const, errorReason: null,
      usage: { ...failedRun.usage, context: { tokens: 10_674, windowTokens, composition: {
        conversationTokens: 4500, systemPromptTokens: 3000, toolTokens: 1200,
        skillTokens: 600, mcpTokens: null, internalTokens: 1374,
      } } } };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [run] });
    fireEvent.click(screen.getByRole("button", { name: "查看当前对话上下文占用：未知" }));
    const detail = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(detail).getByText("10,674 / 未知 · 未知")).toBeTruthy();
    fireEvent.click(within(detail).getByRole("button", { name: "展开上下文详细统计" }));
    expect(within(detail).getByText("4,500 · 未知")).toBeTruthy();
    expect(within(detail).getByText("未提供 · 未知")).toBeTruthy();
    expect(detail.innerHTML).not.toMatch(/NaN|Infinity/);
  });

  test("尚无运行记录时六类统计均显示未提供", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [] });
    fireEvent.click(screen.getByRole("button", { name: "查看当前对话上下文占用：未知" }));
    const detail = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(detail).getByText("未提供 / 未知 · 未知")).toBeTruthy();
    fireEvent.click(within(detail).getByRole("button", { name: "展开上下文详细统计" }));
    expect([...detail.querySelectorAll("dt")].map((label) => label.textContent)).toEqual(["对话内容", "系统提示词", "工具上下文", "Skill", "MCP", "Codex 内部"]);
    expect(within(detail).getAllByText("未提供 · 未知")).toHaveLength(6);
    expect(detail.innerHTML).not.toMatch(/NaN|Infinity/);
  });

  test("首轮回复期间保留未知，回复结束后更新实际数值并保留展开状态", async () => {
    const running = { ...failedRun, status: "running" as const, completedAt: null, errorReason: null,
      usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 } };
    const overrides: { messages: AiRuntimeSnapshot["messages"]; runs: AiRuntimeSnapshot["runs"] } = { messages: [existingMessage], runs: [running] };
    await renderWorkbench(conversationContext(), overrides);
    const trigger = screen.getByRole("button", { name: "查看当前对话上下文占用：未知" });
    expect((trigger as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(trigger);
    const detail = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(detail).getByText("最近请求上下文")).toBeTruthy();
    expect(within(detail).getByText("未提供 / 未知 · 未知")).toBeTruthy();
    expect(within(detail).queryByText("等待本轮统计")).toBeNull();
    fireEvent.click(within(detail).getByRole("button", { name: "展开上下文详细统计" }));
    expect(within(detail).getAllByText("未提供 · 未知")).toHaveLength(6);

    overrides.runs = [{ ...running, partialOutput: "首轮仍在输出正文",
      usage: { ...running.usage, context: { tokens: 10674, windowTokens: 249036 } },
    }];
    await screen.findByText("首轮仍在输出正文");
    await waitFor(() => expect(within(detail).getByText("未提供 / 243K · 未知")).toBeTruthy());
    expect(within(detail).queryByText(/10,674/)).toBeNull();
    expect(within(detail).getByRole("button", { name: "收起上下文详细统计" }).getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByRole("button", { name: "查看当前对话上下文占用：未知" })).toBeTruthy();
    overrides.runs = [{ ...overrides.runs[0], status: "completed", completedAt: running.startedAt + 10_000, partialOutput: null }];
    await waitFor(() => expect(within(detail).getByText("10,674 / 243K · 4.3%")).toBeTruthy());
    expect(screen.getByRole("button", { name: "查看当前对话上下文占用：4.3%" })).toBeTruthy();
    expect(within(detail).getAllByText("未提供 · 未知")).toHaveLength(6);
    expect(within(detail).queryByText("历史记录未保存分类统计")).toBeNull();
    expect(within(detail).getByText("未提供分类")).toBeTruthy();
    expect(detail.querySelector<HTMLElement>(".ai-model-usage-bar-used")!.style.width).toBe(`${10_674 / 249_036 * 100}%`);
    expect(detail.querySelectorAll(".ai-model-usage-bar-used > i")).toHaveLength(0);
  });

  test("整轮回复保留上次统计和分类，回复结束后已展开面板整体切换", async () => {
    const previous: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun, runId: "run-previous", status: "completed", errorReason: null,
      model: "Qwen3.8-27B-NVFP4", usage: { ...failedRun.usage, context: { tokens: 12_000, windowTokens: 524_288,
        composition: { conversationTokens: 4500, systemPromptTokens: 3000, toolTokens: 1200, skillTokens: 600, mcpTokens: 700, internalTokens: 2000 },
      } },
    };
    const running: AiRuntimeSnapshot["runs"][number] = {
      ...failedRun, runId: "run-current", startedAt: previous.startedAt + 10_000,
      status: "running", completedAt: null, errorReason: null,
      usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 },
    };
    const replaced = { ...previous, runId: "run-replaced", startedAt: previous.startedAt + 5_000,
      superseded: true, usage: { ...previous.usage, context: { tokens: 80_000, windowTokens: 524_288 } } };
    const otherConversation = { ...previous, runId: "run-other", conversationId: "conv-2", startedAt: running.startedAt + 10_000,
      usage: { ...previous.usage, context: { tokens: 90_000, windowTokens: 524_288 } } };
    const overrides: { messages: AiRuntimeSnapshot["messages"]; runs: AiRuntimeSnapshot["runs"] } = {
      messages: [existingMessage], runs: [previous, replaced, running, otherConversation],
    };
    await renderWorkbench(conversationContext(), overrides);
    const trigger = screen.getByRole("button", { name: "查看上次请求上下文占用：2.3%" });
    fireEvent.click(trigger);
    const detail = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(detail).getByText("上次请求上下文")).toBeTruthy();
    expect(within(detail).getByText("12,000 / 512K · 2.3%")).toBeTruthy();
    expect(within(detail).queryByText(/90,000/)).toBeNull();
    fireEvent.click(within(detail).getByRole("button", { name: "展开上下文详细统计" }));
    expect(within(detail).getByText("4,500 · 0.9%")).toBeTruthy();

    const live = { ...running, partialOutput: "新一轮仍在输出正文", usage: { ...running.usage, context: { tokens: 10_674, windowTokens: 262_144 } } };
    overrides.runs = [previous, replaced, live, otherConversation];
    await screen.findByText("新一轮仍在输出正文");
    expect(within(detail).getByText("12,000 / 512K · 2.3%")).toBeTruthy();
    expect(within(detail).getByText("4,500 · 0.9%")).toBeTruthy();
    expect(within(detail).queryByText("分类统计将在本轮结束后显示")).toBeNull();
    expect(screen.getByRole("button", { name: "查看上次请求上下文占用：2.3%" })).toBeTruthy();

    overrides.runs = [previous, replaced, { ...live, status: "completed", completedAt: running.startedAt + 10_000,
      partialOutput: null, classificationStatus: "pending", usage: { ...live.usage, context: { ...live.usage.context,
        composition: { conversationTokens: 5000, systemPromptTokens: 3000, toolTokens: 1000, skillTokens: 600, mcpTokens: 74, internalTokens: 1000 },
      } },
    }, otherConversation];
    await waitFor(() => expect(within(detail).getByText("10,674 / 256K · 4.1%")).toBeTruthy());
    expect(within(detail).getByText("最近请求上下文")).toBeTruthy();
    expect(within(detail).queryByText("上次请求上下文")).toBeNull();
    expect(within(detail).getByText("5,000 · 1.9%")).toBeTruthy();
    expect(within(detail).queryByText("4,500 · 0.9%")).toBeNull();
    expect(within(detail).getByRole("button", { name: "收起上下文详细统计" }).getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByRole("button", { name: "查看当前对话上下文占用：4.1%" })).toBeTruthy();
  });

  test.each(["queued", "running", "paused", "terminating", "waiting_permission"] as const)("%s 状态重新打开页面仍显示上次已结束统计", async (status) => {
    const previous = { ...failedRun, runId: "previous", status: "completed" as const, errorReason: null,
      usage: { ...failedRun.usage, context: { tokens: 12_000, windowTokens: 524_288 } } };
    const current = { ...previous, runId: "current", startedAt: previous.startedAt + 10_000, status, completedAt: null,
      usage: { ...previous.usage, context: { tokens: 20_000, windowTokens: 262_144 } } };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [previous, current] });
    fireEvent.click(screen.getByRole("button", { name: "查看上次请求上下文占用：2.3%" }));
    const detail = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(detail).getByText("12,000 / 512K · 2.3%")).toBeTruthy();
    expect(within(detail).queryByText(/20,000/)).toBeNull();
  });

  test.each(["completed", "failed", "terminated", "interrupted"] as const)("本轮 %s 后展示本轮实际记录的统计", async (status) => {
    const previous = { ...failedRun, runId: "previous", status: "completed" as const, errorReason: null,
      usage: { ...failedRun.usage, context: { tokens: 12_000, windowTokens: 524_288 } } };
    const current = { ...previous, runId: "current", startedAt: previous.startedAt + 10_000, completedAt: previous.startedAt + 20_000, status,
      usage: { ...previous.usage, context: { tokens: 20_000, windowTokens: 262_144 } } };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [previous, current] });
    fireEvent.click(screen.getByRole("button", { name: "查看当前对话上下文占用：7.6%" }));
    const detail = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(detail).getByText("20,000 / 256K · 7.6%")).toBeTruthy();
    expect(within(detail).getByText("最近请求上下文")).toBeTruthy();
  });

  test("实际返回零占用时保留零，不把已知数值标为未知", async () => {
    const run = { ...failedRun, status: "completed" as const, errorReason: null,
      usage: { ...failedRun.usage, context: { tokens: 0, windowTokens: 262_144,
        composition: { conversationTokens: 0, systemPromptTokens: 0, toolTokens: 0, skillTokens: 0, mcpTokens: 0, internalTokens: 0 } } } };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [run] });
    fireEvent.click(screen.getByRole("button", { name: "查看当前对话上下文占用：0.0%" }));
    const detail = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(detail).getByText("0 / 256K · 0.0%")).toBeTruthy();
    fireEvent.click(within(detail).getByRole("button", { name: "展开上下文详细统计" }));
    expect(within(detail).getAllByText("0 · 0.0%")).toHaveLength(6);
    expect(detail.textContent).not.toContain("未提供");
  });

  test("最新回复结束但没有记录统计时不拿更早数据替代", async () => {
    const previous = { ...failedRun, runId: "previous", status: "completed" as const, errorReason: null,
      usage: { ...failedRun.usage, context: { tokens: 12_000, windowTokens: 524_288 } } };
    const current = { ...failedRun, runId: "current", startedAt: previous.startedAt + 10_000, completedAt: previous.startedAt + 20_000 };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [previous, current] });
    fireEvent.click(screen.getByRole("button", { name: "查看当前对话上下文占用：未知" }));
    const detail = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(detail).getByText("未提供 / 未知 · 未知")).toBeTruthy();
    expect(within(detail).queryByText(/12,000/)).toBeNull();
    fireEvent.click(within(detail).getByRole("button", { name: "展开上下文详细统计" }));
    expect(within(detail).getAllByText("未提供 · 未知")).toHaveLength(6);
  });

  test("重新运行开始后隐藏上一条异常", async () => {
    const retryingRun = { ...failedRun, runId: "run-retrying", retryOfRunId: failedRun.runId, status: "running" as const, startedAt: failedRun.startedAt + 10_000, completedAt: null, errorReason: null };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [failedRun, retryingRun] });

    expect(screen.getByRole("button", { name: "展开执行过程：正在处理…" })).toBeTruthy();
    expect(screen.getAllByText("正在处理…")).toHaveLength(1);
    expect(screen.queryByRole("region", { name: "最近一次运行异常" })).toBeNull();
    expect(screen.queryByText("模型凭据不可用")).toBeNull();
  });

  test("会话开始后不在输入框重复显示项目，临时上下文仍可移除", async () => {
    // 这一组能力此前挂在对话区顶上那条横带上，横带删掉时被一并带走了，
    // 而当时没有任何测试盯着它。
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "示例项目", phaseId: "phase-1", phaseName: "需求阶段" }), { messages: [existingMessage] });
    const row = document.querySelector(".ai-composer footer") as HTMLElement;
    expect(row.textContent).not.toContain(projects[0].displayName);
    expect(row.textContent).toContain("需求阶段");
    expect(within(row).queryByLabelText("移除项目上下文")).toBeNull();
    expect(within(row).getByLabelText("移除阶段上下文")).toBeTruthy();
    expect(within(row).queryByLabelText("移除文件上下文")).toBeNull();
  });

  test("已发送文件在消息中显示名称与完整路径，不再回到输入框", async () => {
    const referencedMaterials = [{ localFileId: selectedFile.localFileId, relativePath, fileRevision: 3, workspaceId: workspace.workspaceId }];
    await renderWorkbench(conversationContext(), { messages: [{ ...existingMessage, referencedMaterials }] });
    const material = screen.getByLabelText("消息文件");
    expect(material.querySelector("[title]")?.getAttribute("title")).toBe(relativePath);
    expect(material.textContent).toBe(fileNameFromPath(relativePath));
    expect(screen.queryByLabelText("待发送文件")).toBeNull();
    expect(document.querySelector(".ai-composer")?.textContent).not.toContain(fileNameFromPath(relativePath));
  });

  test("对话右上角的上下文只展示引用材料", async () => {
    const completedRun: AiRuntimeSnapshot["runs"][number] = {
      runId: "run-artifact", conversationId: "conv-1", agentId: "agent-1", inputMessageId: "message-existing",
      retryOfRunId: null, provider: "codex", model: "gpt-test", executionTarget: "agent",
      status: "completed", pid: null, startedAt: 2, completedAt: 3, output: "已完成", errorReason: null,
      sessionId: null, skillIds: [], toolPermissions: [], permissionRequestId: null, automationRuleId: null,
      automationRuleVersion: null, sandboxMode: "read-only", events: [
        { sequence: 1, eventType: "turn.plan.updated", summary: "Todo 已完成 2/2", createdAt: 2 },
      ], usage: { inputTokens: 10, cachedInputTokens: 0, outputTokens: 5 }, logTail: [],
      artifacts: [
        { artifactId: "artifact-1", artifactType: "assistant-response", label: "AI 候选结果", path: "/tmp/runs/run-artifact/output.md" },
        { artifactId: "artifact-2", artifactType: "document", label: "生成报告", path: "产物/报告.md" },
      ],
    };
    const reportFile = { ...selectedFile, localFileId: "report-md", relativePath: "产物/报告.md" };
    const onOpenFile = vi.fn();
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "项目上下文" }), { messages: [existingMessage], runs: [completedRun], files: [reportFile], onOpenFile });

    fireEvent.click(screen.getByRole("button", { name: "显示产物与来源" }));
    const panel = screen.getByRole("complementary", { name: "对话进度、产物与上下文" });
    expect(within(panel).getByRole("heading", { name: "进度" })).toBeTruthy();
    expect(within(panel).getByRole("img", { name: "当前运行进度：已完成" })).toBeTruthy();
    expect(within(panel).getByRole("heading", { name: "产物" })).toBeTruthy();
    const artifactButton = within(panel).getByRole("button", { name: "打开产物 报告.md" });
    expect(within(artifactButton).getByText("报告.md")).toBeTruthy();
    expect(within(artifactButton).getByText("产物/报告.md")).toBeTruthy();
    expect(panel.textContent).not.toContain("AI 候选结果");
    expect(panel.textContent).not.toContain("对话输出");
    expect(panel.textContent).not.toContain("run-artifact");
    expect(within(panel).getByRole("heading", { name: "上下文" })).toBeTruthy();
    expect(panel.textContent).not.toContain(projects[0].displayName);
    expect(within(panel).getByText(fileNameFromPath(relativePath))).toBeTruthy();
    expect(within(panel).getByText(relativePath)).toBeTruthy();
    expect(within(panel).getByTitle(relativePath)).toBeTruthy();
    expect(panel.textContent).not.toContain("已添加");
    expect(panel.textContent).not.toContain("已读取");
    fireEvent.click(artifactButton);
    expect(onOpenFile).toHaveBeenCalledWith("report-md", workspace.workspaceId);
  });

  test("产物只展示有文件路径的非回复型结果并按路径去重", () => {
    const runs = [{
      runId: "run-artifacts",
      artifacts: [
        { artifactId: "response", artifactType: "assistant-response", label: "模型候选结果", path: "/tmp/runs/response.md" },
        { artifactId: "document-1", artifactType: "document", label: "报告", path: "产物/报告.md" },
        { artifactId: "document-2", artifactType: "document", label: "报告副本记录", path: "产物/报告.md" },
        { artifactId: "missing-path", artifactType: "document", label: "无路径", path: null },
      ],
    }];

    expect(conversationFileArtifacts(runs).map(({ runId, artifact }) => [runId, artifact.artifactId])).toEqual([["run-artifacts", "document-2"]]);
  });

  test("只有项目归属时上下文显示引用材料空状态", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "项目不是材料", localFileId: null, relativePath: null, fileRevision: null }), { messages: [existingMessage] });

    fireEvent.click(screen.getByRole("button", { name: "显示产物与来源" }));
    const panel = screen.getByRole("complementary", { name: "对话产物与上下文" });
    expect(within(panel).queryByRole("heading", { name: "进度" })).toBeNull();
    expect(within(panel).getByText("本对话尚未引用文件或图片")).toBeTruthy();
    expect(panel.textContent).not.toContain("项目不是材料");
  });

  test("上下文保留当前对话引用过的多个材料", async () => {
    await renderWorkbench(conversationContext({
      localFileId: null,
      relativePath: null,
      fileRevision: null,
      referencedMaterials: [
        { localFileId: "file-1", relativePath, fileRevision: 3 },
        { localFileId: "file-2", relativePath: "assets/产品原型.png", fileRevision: 1 },
      ],
    }), { messages: [existingMessage] });

    fireEvent.click(screen.getByRole("button", { name: "显示产物与来源" }));
    const panel = screen.getByRole("complementary", { name: "对话产物与上下文" });
    expect(within(panel).getByText(fileNameFromPath(relativePath))).toBeTruthy();
    expect(within(panel).getByText("产品原型.png")).toBeTruthy();
    expect(within(panel).getByText("assets/产品原型.png")).toBeTruthy();
    expect(panel.querySelectorAll(".ai-conversation-material")).toHaveLength(2);
  });

  test("同一个右上角按钮在面板外切换产物与来源的显隐", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });

    expect(screen.queryByRole("complementary", { name: "对话产物与上下文" })).toBeNull();
    const show = screen.getByRole("button", { name: "显示产物与来源" });
    expect(show.getAttribute("aria-expanded")).toBe("false");
    expect(show.querySelector("svg")?.getAttribute("data-open")).toBe("false");

    fireEvent.click(show);
    const reopenedPanel = screen.getByRole("complementary", { name: "对话产物与上下文" });
    const reopenedToggle = screen.getByRole("button", { name: "隐藏产物与来源" });
    expect(reopenedToggle).toBe(show);
    expect(reopenedPanel.contains(reopenedToggle)).toBe(false);
    expect(reopenedToggle.getAttribute("aria-expanded")).toBe("true");
    expect(reopenedToggle.querySelector("svg")?.getAttribute("data-open")).toBe("true");

    fireEvent.click(reopenedToggle);
    expect(screen.queryByRole("complementary", { name: "对话产物与上下文" })).toBeNull();
  });

  test("宽内容区自动停靠，缩回普通宽度时先隐藏面板", async () => {
    let resize: ResizeObserverCallback | null = null;
    class ResizeObserverMock {
      constructor(callback: ResizeObserverCallback) { resize = callback; }
      observe(target: Element) {
        resize?.([{ target, contentRect: { width: 1200 } } as ResizeObserverEntry], this as unknown as ResizeObserver);
      }
      disconnect() { /* 测试无需释放浏览器资源 */ }
      unobserve() { /* 测试无需单独取消元素 */ }
    }
    vi.stubGlobal("ResizeObserver", ResizeObserverMock);
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });

    await waitFor(() => expect(document.querySelector(".ai-conversation-shell.info-docked")).toBeTruthy());
    expect(screen.getByRole("complementary", { name: "对话产物与上下文" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "隐藏产物与来源" }));
    expect(document.querySelector(".ai-conversation-shell.info-docked")).toBeNull();
    expect(document.querySelector(".ai-conversation-shell.info-popover .ai-conversation-tools")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "显示产物与来源" }));
    expect(document.querySelector(".ai-conversation-shell.info-open.info-docked .ai-conversation-tools")).toBeTruthy();

    const stage = document.querySelector(".ai-chat-stage")!;
    act(() => resize?.([{ target: stage, contentRect: { width: 900 } } as ResizeObserverEntry], {} as ResizeObserver));
    expect(document.querySelector(".ai-conversation-shell.info-popover")).toBeTruthy();
    expect(screen.queryByRole("complementary", { name: "对话产物与上下文" })).toBeNull();
  });

  test("右侧面板分区可独立折叠且上下文添加入口打开文件选择", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [{
      ...failedRun,
      events: [...failedRun.events, { sequence: 3, eventType: "turn.plan.updated", summary: "Todo 已完成 1/2", createdAt: 3 }],
    }] });
    fireEvent.click(screen.getByRole("button", { name: "显示产物与来源" }));
    const panel = screen.getByRole("complementary", { name: "对话进度、产物与上下文" });
    const progressToggle = within(panel).getByRole("button", { name: "进度" });

    expect(progressToggle.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(progressToggle);
    expect(progressToggle.getAttribute("aria-expanded")).toBe("false");
    expect(within(panel).queryByRole("img", { name: /当前运行进度/ })).toBeNull();

    fireEvent.click(within(panel).getByRole("button", { name: "添加引用材料" }));
    expect(screen.getByRole("dialog", { name: "选择工作区文件" })).toBeTruthy();
  });

  test("中间省略保住结尾，同目录下的同前缀文件不会显示成同一个", () => {
    expect(elideMiddle("短名.md")).toBe("短名.md");
    expect(elideMiddle("项目目录结构与产品分类.md")).toBe("项目目录结…类.md");
    // 掐尾的话这两个都会变成「接口设计说明-v…」，中间省略才保住区分度。
    expect(elideMiddle("接口设计说明-v1.md")).not.toBe(elideMiddle("接口设计说明-v2.md"));
  });

  test("运行时就绪且已输入内容时发送按钮可用", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    const send = screen.getByRole("button", { name: "发送" }) as HTMLButtonElement;
    expect(send.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "帮我看看这份文档" } });
    expect(send.disabled).toBe(false);
    expect(send.title).toBe("发送");
    expect(send.textContent).toBe("");
    expect(send.querySelector(".ai-enter-icon")).toBeTruthy();
    expect(send.previousElementSibling?.classList.contains("ai-starter-permission-picker")).toBe(true);
  });

  test("会话按项目成组，项目行只写项目名", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "示例项目" }));
    const toggle = document.querySelector(".ai-conversation-group-toggle") as HTMLElement;
    expect(toggle.textContent).toContain(projects[0].displayName);
  });

  test("项目改名后分组使用最新名称，输入框不重复项目归属", async () => {
    const renamedProjects = projects.map((project) => project.projectId === "proj-1" ? { ...project, displayName: "最新项目名" } : project);
    await renderWorkbench(
      conversationContext({ projectId: "proj-1", projectName: "创建会话时的旧名称" }),
      { messages: [existingMessage], projects: renamedProjects },
    );

    expect(document.querySelector(".ai-conversation-group-toggle")?.textContent).toContain("最新项目名");
    expect([...document.querySelectorAll(".ai-context-chip")].some((node) => node.getAttribute("title")?.startsWith("项目："))).toBe(false);
    expect(document.body.textContent).not.toContain("创建会话时的旧名称");
  });

  test("输入框不展示关联任务，但保留会话中的任务归属", async () => {
    await renderWorkbench(conversationContext({ taskId: "task-1", taskName: "优化项目文章内容" }), { messages: [existingMessage] });

    expect([...document.querySelectorAll(".ai-context-chip")].some((node) => node.getAttribute("title")?.startsWith("任务："))).toBe(false);
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "继续优化文章" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_context_update")).toBeUndefined();
  });

  test("会话记录以主题、消息摘要和更新时间组成，不重复项目与模式", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "示例项目" }));
    const item = document.querySelector(".ai-conversation-item") as HTMLElement;
    expect(item.textContent).not.toContain("示例项目");
    expect(item.querySelector("strong")?.textContent).toBe("文件讨论");
    expect(item.querySelector("small")?.textContent).toContain("尚未发送消息");
    expect(item.querySelector("time")).not.toBeNull();
    expect(screen.getByRole("button", { name: "归档会话：文件讨论" })).toBeTruthy();
  });

  test("当前会话行暴露持续选中语义，悬停不改变选中身份", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "示例项目" }));
    const item = document.querySelector(".ai-conversation-item") as HTMLElement;
    const open = item.querySelector(".ai-conversation-open") as HTMLButtonElement;

    expect(item.classList.contains("selected")).toBe(true);
    expect(open.getAttribute("aria-current")).toBe("true");
    fireEvent.mouseEnter(open);
    expect(item.classList.contains("selected")).toBe(true);
    expect(open.getAttribute("aria-current")).toBe("true");
  });

  test("当前会话所属项目只保留当前语义，不持续显示选中背景", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "示例项目" }));
    const header = document.querySelector(".ai-conversation-group-header") as HTMLElement;
    const toggle = header.querySelector(".ai-conversation-group-toggle") as HTMLButtonElement;
    const more = screen.getByLabelText("主项目 的更多操作") as HTMLButtonElement;

    expect(header.classList.contains("selected")).toBe(false);
    expect(toggle.getAttribute("aria-current")).toBe("true");
    fireEvent.mouseEnter(more);
    expect(header.classList.contains("selected")).toBe(false);
    expect(toggle.getAttribute("aria-current")).toBe("true");
    fireEvent.click(more);
    expect(header.classList.contains("menu-open")).toBe(true);
    expect(header.classList.contains("selected")).toBe(false);
    expect(more.getAttribute("aria-expanded")).toBe("true");
  });

  test("会话行的归档按钮直接执行归档", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "示例项目" }));
    fireEvent.click(screen.getByRole("button", { name: "归档会话：文件讨论" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_conversation_set_archived", { request: { archived: true, conversationIds: ["conv-1"] } }));
  });

  test("归档等待期间可以切换对话，回执不打断新的选择", async () => {
    const context = conversationContext();
    const conversations = ["conv-1", "conv-2"].map((conversationId) => ({ ...snapshot(context).conversations[0], conversationId, title: conversationId }));
    await renderWorkbench(context, { conversations });
    let resolve!: (value: unknown) => void;
    const original = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, args?: unknown) => command === "ai_conversation_set_archived"
      ? new Promise((done) => { resolve = done; }) : original(command, args));
    fireEvent.click(screen.getByRole("button", { name: "归档会话：conv-1" }));
    expect(screen.getByRole("button", { name: "新建 AI 对话" }).hasAttribute("disabled")).toBe(false);
    fireEvent.click(document.querySelectorAll<HTMLButtonElement>(".ai-conversation-open")[1]);
    expect(document.querySelectorAll(".ai-conversation-open")[1].getAttribute("aria-current")).toBe("true");
    await act(async () => resolve({ stateRevision: 1, conversations: [{ ...conversations[0], archived: true }], changedConversationIds: ["conv-1"], failures: [] }));
    expect(screen.queryByRole("button", { name: "归档会话：conv-1" })).toBeNull();
    expect(document.querySelector(".ai-conversation-open")?.getAttribute("aria-current")).toBe("true");
    expect(screen.getByRole("button", { name: "归档会话：conv-2" }).hasAttribute("disabled")).toBe(false);
  });

  test("归档写入失败保留会话和当前选择并恢复操作", async () => {
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { onMessage });
    const original = invoke.getMockImplementation()!;
    invoke.mockImplementation((command: string, args?: unknown) => command === "ai_conversation_set_archived" ? Promise.reject(new Error("ai_state_write_failed")) : original(command, args));
    fireEvent.click(screen.getByRole("button", { name: "归档会话：文件讨论" }));
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith(expect.stringContaining("归档失败"), "error"));
    expect(screen.getByRole("button", { name: "归档会话：文件讨论" }).hasAttribute("disabled")).toBe(false);
    expect(document.querySelector(".ai-conversation-open")?.getAttribute("aria-current")).toBe("true");
  });

  test("批量归档部分失败时停止，报告实际数量并保留未归档会话", async () => {
    const context = conversationContext({ projectId: "proj-1", projectName: "示例项目" });
    const state = snapshot(context);
    state.conversations = ["conv-1", "conv-2", "conv-3"].map((conversationId) => ({ ...state.conversations[0], conversationId, title: conversationId }));
    const onMessage = vi.fn();
    invoke.mockImplementation(async (command: string, args?: { request: { conversationId: string } }) => {
      if (command === "ai_runtime_snapshot") return state;
      if (command === "ai_conversation_set_archived") return {
        stateRevision: 1, conversations: [{ ...state.conversations[0], archived: true }], changedConversationIds: ["conv-1"],
        failures: [{ conversationId: "conv-2", reason: "conversation_run_already_active" }, { conversationId: "conv-3", reason: "conversation_archive_not_attempted" }],
      };
      throw new Error(command);
    });
    render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[]} selectedFile={null} launchRequest={null} preferredConversationId={null} onMessage={onMessage} onOpenAgents={vi.fn()} canCreateProject onCreateProject={vi.fn()} onSelectProject={vi.fn()} projectIndex={new Map()}/>);
    await screen.findByRole("button", { name: "归档会话：conv-1" });
    fireEvent.click(screen.getByLabelText(`${projects[0].displayName} 的更多操作`));
    fireEvent.click(screen.getByRole("menuitem", { name: "归档这里的 3 条会话" }));
    fireEvent.click(screen.getByRole("button", { name: "归档" }));
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith(expect.stringContaining("已归档 1 条，2 条未完成"), "error"));
    expect(invoke.mock.calls.filter(([command]) => command === "ai_conversation_set_archived")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "归档会话：conv-1" })).toBeNull();
    expect(screen.getByRole("button", { name: "归档会话：conv-2" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "归档会话：conv-3" })).toBeTruthy();
    expect(onMessage.mock.calls.some(([message]) => message.includes("已归档 3 条"))).toBe(false);
  });

  test("点击项目行折叠该项目的会话", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "示例项目" }));
    expect(document.querySelectorAll(".ai-conversation-item")).toHaveLength(1);

    const toggle = document.querySelector(".ai-conversation-group-toggle") as HTMLElement;
    expect(toggle.getAttribute("aria-label")).toBe("收起项目分类：主项目");
    const expandedIcon = toggle.querySelector('svg[data-state="expanded"]') as SVGElement;
    expect(expandedIcon.querySelectorAll("path")).toHaveLength(2);
    const expandedPath = expandedIcon.querySelector("path")?.getAttribute("d");
    fireEvent.click(toggle);

    expect(document.querySelectorAll(".ai-conversation-item")).toHaveLength(0);
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(toggle.getAttribute("aria-label")).toBe("展开项目分类：主项目");
    const collapsedIcon = toggle.querySelector('svg[data-state="collapsed"]') as SVGElement;
    expect(collapsedIcon.querySelectorAll("path")).toHaveLength(1);
    expect(collapsedIcon.querySelector("path")?.getAttribute("d")).not.toBe(expandedPath);
  });

  test("没有项目归属的会话仍然成组，不会从列表里消失", async () => {
    await renderWorkbench();
    const names = [...document.querySelectorAll(".ai-conversation-group-toggle > span")].map((node) => node.textContent);
    expect(names).toEqual(["主项目", "第二项目", "未分类"]);
  });

  test("项目行只保留会话组织操作，不混入项目编辑入口", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "示例项目" }));
    expect(screen.getByLabelText(`${projects[0].displayName} 的更多操作`)).toBeTruthy();
    expect(screen.queryByLabelText(`编辑项目 ${projects[0].displayName}`)).toBeNull();
  });

  test("项目菜单只给出置顶和批量归档", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "示例项目" }));
    fireEvent.click(screen.getByLabelText(`${projects[0].displayName} 的更多操作`));
    const labels = [...document.querySelectorAll('[role="menuitem"]')].map((node) => node.textContent);
    expect(labels).toContain("置顶");
    expect(labels).not.toContain("编辑");
    expect(labels).not.toContain("在访达中显示");
    expect(labels).not.toContain("移除项目");
    expect(labels.some((label) => label?.includes("归档"))).toBe(true);
  });

  test("旧对话只有工作区时不得将映射项目显示为已绑定项目", async () => {
    const projectIndex = new Map([[workspace.workspaceId, { projectId: "proj-9", displayName: "映射出来的项目" }]]);
    await renderWorkbench(conversationContext(), { projectIndex });

    const toggle = [...document.querySelectorAll(".ai-conversation-group-toggle")].find((node) => node.textContent?.includes("未分类")) as HTMLElement;
    expect(toggle.textContent).toContain("未分类");
    expect(screen.queryByLabelText("编辑项目 映射出来的项目")).toBeNull();
  });

  test("未关联项目时仍只提供会话组织操作", async () => {
    await renderWorkbench();
    expect(screen.queryByLabelText(`编辑项目 ${workspace.displayName}`)).toBeNull();
    fireEvent.click(screen.getByLabelText("未分类 的更多操作"));
    const labels = [...document.querySelectorAll('[role="menuitem"]')].map((node) => node.textContent);
    expect(labels).not.toContain("移除项目");
    expect(labels).not.toContain("在访达中显示");
  });

  test("置顶写入本地存储，重开应用仍然置顶", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "示例项目" }));
    fireEvent.click(screen.getByLabelText(`${projects[0].displayName} 的更多操作`));
    fireEvent.click(screen.getByRole("menuitem", { name: "置顶" }));
    expect(JSON.parse(window.localStorage.getItem("cilai.workbench.ai-pinned-projects") ?? "[]")).toContain(projects[0].projectId);
  });

  test("对话区不再重复显示会话标题栏", async () => {
    await renderWorkbench();
    expect(document.querySelector(".ai-chat-header")).toBeNull();
    expect(screen.queryByRole("heading", { level: 2, name: "文件讨论" })).toBeNull();
  });

  test("已有空会话显示任务启动页，不再落成空白消息区", async () => {
    await renderWorkbench();
    expect(screen.queryByText("围绕当前上下文开始讨论")).toBeNull();
    expect(screen.queryByText(/对话结论不会自动改变项目状态/)).toBeNull();
    expect(screen.getByRole("heading", { name: "Hi，今天有什么安排？" })).toBeTruthy();
    expect((screen.getByLabelText("描述要处理的任务") as HTMLTextAreaElement).rows).toBe(1);
    expect(screen.getByLabelText("描述要处理的任务").closest(".ai-composer")).toBeTruthy();
    expect(screen.queryByLabelText("发送给 AI")).toBeNull();
  });

  test("已有空会话提交首条任务时直接使用当前会话", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "主项目" }), { projectIndex: defaultProjectIndex });
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "梳理今天的项目安排" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create")).toBe(false);
    const sendCall = invoke.mock.calls.find(([command]) => command === "ai_message_submit");
    expect(sendCall?.[1]?.request).toMatchObject({ conversationId: "conv-1", content: "梳理今天的项目安排" });
  });

  test("没有会话时只在输入面板内显示智能体与模型选择和业务入口", async () => {
    await renderEmptyWorkbench();

    expect(screen.getByRole("heading", { name: "Hi，今天有什么安排？" })).toBeTruthy();
    expect(screen.getByLabelText("描述要处理的任务")).toBeTruthy();
    expect(screen.queryByLabelText("智能体快捷选择")).toBeNull();
    expect(screen.queryByRole("combobox", { name: "选择执行智能体" })).toBeNull();
    expect(screen.getByRole("button", { name: "选择模型：选择模型" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "选择一项业务开始" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "开始查资料业务" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "开始写材料业务" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "开始填报信息业务" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "开始推进事项业务" })).toBeTruthy();
    expect(document.querySelector(".ai-starter-project-picker")?.textContent).toBe(projects[0].displayName);
    expect(screen.queryByText("选择或创建一个 AI 会话")).toBeNull();
  });

  test("选择业务会把可编辑的业务任务模板带入输入框", async () => {
    await renderEmptyWorkbench();
    const input = screen.getByLabelText("描述要处理的任务") as HTMLTextAreaElement;

    fireEvent.click(screen.getByRole("button", { name: "开始查资料业务" }));
    fireEvent.change(input, { target: { value: `${input.value}\n查询客户项目状态` } });
    fireEvent.click(screen.getByRole("button", { name: "开始写材料业务" }));

    expect(input.value).not.toContain("查询并整理以下信息");
    expect(input.value).toContain("撰写以下材料");
    expect(input.value).toContain("查询客户项目状态");
    expect(screen.getByRole("button", { name: "开始写材料业务" }).getAttribute("aria-pressed")).toBe("true");
  });

  test("本次运行模型会随发送请求进入后端", async () => {
    await renderEmptyWorkbench();
    selectDefaultStarterAgent();
    fireEvent.click(screen.getByRole("button", { name: /^选择模型：/ }));
    expect(screen.queryByRole("group", { name: "智能体" })).toBeNull();
    expect(screen.getByRole("group", { name: "模型" })).toBeTruthy();
    const modelOption = screen.getByRole("menuitemradio", { name: /Qwen3.6 35B A3B NVFP4.*上下文 256K/ });
    expect(modelOption.textContent).toBe("Qwen3.6 35B A3B NVFP4上下文 256K");
    expect(modelOption.textContent).not.toContain("模型 Harness");
    fireEvent.click(modelOption);
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "检查本次模型" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    const sendCall = invoke.mock.calls.find(([command]) => command === "ai_conversation_submit");
    expect(sendCall?.[1]?.request.modelOverride).toBe("Qwen3.6-35B-A3B-NVFP4");
    expect(sendCall?.[1]?.request.executionTarget).toBe("model");
  });

  test("落地页发送后立即进入对话视图，不被自动新建的草稿挡回落地页", async () => {
    await renderEmptyWorkbench();
    selectDefaultStarterAgent();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "跳转回归" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    // 锁定「发送成功后落地页必须让位给对话视图」这一行为。注意它只覆盖选中态是否最终写入，
    // 不覆盖真实环境里那个时序窗口：jsdom 下刷新是瞬时的，自动新建草稿的 effect 没有机会
    // 挤进选中态写入之前，因此该窗口本身无法在此环境复现。
    await waitFor(() => expect(screen.queryByRole("heading", { name: "Hi，今天有什么安排？" })).toBeNull());
  });

  test("发出第一句话时输入框、消息区与工具栏沿用同一份，不整块换成另一套页面", async () => {
    await renderEmptyWorkbench();
    selectDefaultStarterAgent();
    const form = document.querySelector("form.ai-composer");
    const messageList = document.querySelector(".ai-message-list");
    const tools = document.querySelector(".ai-conversation-tools");
    const input = screen.getByLabelText("描述要处理的任务");
    expect(form).not.toBeNull();
    fireEvent.change(input, { target: { value: "同一套结构" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));

    await waitFor(() => expect(screen.getByLabelText("发送给 AI")).toBeTruthy());
    expect(document.querySelectorAll("form.ai-composer")).toHaveLength(1);
    expect(document.querySelector("form.ai-composer")).toBe(form);
    expect(document.querySelector(".ai-message-list")).toBe(messageList);
    expect(document.querySelector(".ai-conversation-tools")).toBe(tools);
    expect(screen.getByLabelText("发送给 AI")).toBe(input);
  });

  test("重新进入时上次的对话还没取回，不先闪出起始引导", async () => {
    window.localStorage.setItem("cilai.workbench.ai-last-viewed-conversation", "conv-1");
    let deliverSnapshot: (value: AiRuntimeSnapshot) => void = () => { throw new Error("快照请求尚未发出"); };
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return new Promise<AiRuntimeSnapshot>((resolve) => { deliverSnapshot = resolve; });
      throw new Error(`unexpected command ${command}`);
    });
    render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[selectedFile]} selectedFile={null} launchRequest={null} preferredConversationId={null} onMessage={() => { /* 消息条不参与本次断言 */ }} onOpenAgents={() => { /* 不检查跳转 */ }} canCreateProject onCreateProject={() => { /* 不检查新建 */ }} onSelectProject={() => { /* 不检查切换 */ }} projectIndex={new Map()}/>);
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_runtime_snapshot")).toBe(true));

    expect(screen.queryByRole("heading", { name: "Hi，今天有什么安排？" })).toBeNull();
    const form = document.querySelector("form.ai-composer");
    expect(form).not.toBeNull();

    await act(async () => { deliverSnapshot({ ...snapshot(conversationContext()), messages: [existingMessage], runs: [] } as unknown as AiRuntimeSnapshot); });
    await waitFor(() => expect(document.querySelector(`.ai-message-list [data-message-id="${existingMessage.messageId}"]`)).not.toBeNull());
    expect(screen.queryByRole("heading", { name: "Hi，今天有什么安排？" })).toBeNull();
    expect(document.querySelector("form.ai-composer")).toBe(form);
  });

  test("模型选择宽度随当前名称变化且不允许自由输入", async () => {
    await renderEmptyWorkbench();
    expect(screen.queryByRole("textbox", { name: "选择或输入模型" })).toBeNull();
    const trigger = screen.getByRole("button", { name: "选择模型：选择模型" });
    expect(trigger.querySelector("span")?.style.width).toBe("9ch");
    expect(trigger.querySelector("span")?.title).toBe("选择模型");

    selectDefaultStarterAgent();
    fireEvent.click(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4/ }));

    expect(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" }).querySelector("span")?.style.width).toBe("18ch");
  });

  test("对话右上角沿用请求窗口计算上下文占比，切换输入模型不改写统计", async () => {
    const now = Date.now();
    const run = (runId: string, conversationId: string, startedAt: number, inputTokens: number, outputTokens: number, model = "Qwen3.6-35B-A3B-NVFP4"): AiRuntimeSnapshot["runs"][number] => ({
      runId, conversationId, agentId: "agent-1", inputMessageId: `message-${runId}`, retryOfRunId: null,
      provider: "codex", model, status: "completed", pid: null,
      startedAt, completedAt: startedAt + 1000, output: "完成", errorReason: null, sessionId: null,
      skillIds: [], toolPermissions: [], permissionRequestId: null, automationRuleId: null,
      automationRuleVersion: null, sandboxMode: "read-only", events: [], artifacts: [],
      usage: { inputTokens: inputTokens + 9000, cachedInputTokens: 0, outputTokens, context: { tokens: inputTokens + outputTokens, windowTokens: 262144 } }, logTail: [],
    });
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [
      run("run-1", "conv-1", now - 60_000, 10_000, 2_000),
      run("run-2", "conv-2", now - 2 * 24 * 60 * 60 * 1000, 4_000, 1_000, "Qwen3.8-27B-NVFP4"),
    ] });
    expect(screen.queryByRole("region", { name: "当前对话上下文占用" })).toBeNull();
    const trigger = screen.getByRole("button", { name: /查看当前对话上下文占用/ });
    expect(trigger.closest(".ai-conversation-tools")).toBeTruthy();
    expect(trigger.closest(".ai-composer-actions")).toBeNull();
    fireEvent.click(trigger);

    const usage = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(usage).getByText("12,000 / 256K · 4.6%")).toBeTruthy();
    expect(within(usage).queryByText("17,000")).toBeNull();
    expect(within(usage).queryByText("输入 14,000 · 输出 3,000")).toBeNull();
    const usageBar = within(usage).getByRole("button", { name: "展开上下文详细统计" });
    expect(usageBar.getAttribute("aria-expanded")).toBe("false");
    expect(parseFloat(usageBar.querySelector<HTMLElement>(".ai-model-usage-bar-used")!.style.width)).toBeCloseTo(4.6, 1);
    fireEvent.keyDown(document, { key: "Escape" });

    fireEvent.click(screen.getByRole("button", { name: /^选择模型：/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4.*上下文 512K/ }));
    const updatedTrigger = screen.getByRole("button", { name: "查看当前对话上下文占用：4.6%" });
    fireEvent.click(updatedTrigger);
    const updatedUsage = screen.getByRole("region", { name: "当前对话上下文占用" });
    expect(within(updatedUsage).getByText("12,000 / 256K · 4.6%")).toBeTruthy();
    expect(within(updatedUsage).queryByText("Qwen3.8 27B NVFP4")).toBeNull();
  });

  test("对话不提供计划模式，新消息使用普通运行并保持审批选择", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    fireEvent.click(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4.*上下文 512K/ }));
    expect(screen.queryByRole("button", { name: "计划模式" })).toBeNull();
    expect(screen.queryByRole("button", { name: "按计划执行" })).toBeNull();
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "只分析并给出计划" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_message_submit")?.[1]?.request).toMatchObject({ planning: false, executionTarget: "model", permissionMode: "request" });
  });

  test("已有消息的对话可以切换 Qwen 模型并用于后续发送", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    fireEvent.click(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4.*上下文 512K/ }));
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "继续分析" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    const sendCall = invoke.mock.calls.find(([command]) => command === "ai_message_submit");
    expect(sendCall?.[1]?.request.modelOverride).toBe("Qwen3.8-27B-NVFP4");
    expect(sendCall?.[1]?.request.executionTarget).toBe("model");
    expect(sendCall?.[1]?.request.permissionMode).toBe("request");
  });

  test("新资料模型任务默认请求人工批准", async () => {
    await renderEmptyWorkbench();
    selectDefaultStarterAgent();
    fireEvent.click(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4.*上下文 512K/ }));

    fireEvent.click(screen.getByRole("button", { name: "选择审批模式：请求批准" }));
    const menu = screen.getByRole("menu", { name: "审批模式" });
    expect(within(menu).getByRole("menuitemradio", { name: /^请求批准/ })).toBeTruthy();
    expect(within(menu).getByRole("menuitemradio", { name: /^帮我批准/ })).toBeTruthy();
    expect(within(menu).getByRole("menuitemradio", { name: /^完全访问权限/ })).toBeTruthy();
    fireEvent.keyDown(document, { key: "Escape" });

    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "检查并按需修改文件" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    expect(screen.queryByRole("heading", { name: "允许修改工作区文件？" })).toBeNull();
    const sendCall = invoke.mock.calls.find(([command]) => command === "ai_conversation_submit");
    expect(sendCall?.[1]?.request.permissionMode).toBe("request");
  });

  test("每个对话独立恢复并保存审批模式", async () => {
    const baseConversation = snapshot(conversationContext()).conversations[0] as AiConversation;
    const conversations: AiRuntimeSnapshot["conversations"] = [
      { ...baseConversation, conversationId: "conv-1", title: "自动审批对话", approvalMode: "auto-review", updatedAt: 2 },
      { ...baseConversation, conversationId: "conv-2", title: "请求审批对话", approvalMode: "request", updatedAt: 1 },
    ];
    const runs: AiRuntimeSnapshot["runs"] = [
      { ...failedRun, runId: "run-auto", conversationId: "conv-1", approvalMode: "auto-review" },
      { ...failedRun, runId: "run-request", conversationId: "conv-2", approvalMode: "request" },
    ];
    await renderWorkbench(conversationContext(), { conversations, runs });

    expect(await screen.findByRole("button", { name: "选择审批模式：帮我批准" })).toBeTruthy();
    fireEvent.click(document.querySelector(".ai-conversation-open[title='请求审批对话']") as HTMLButtonElement);
    await waitFor(() => expect(screen.getByRole("button", { name: "选择审批模式：请求批准" })).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: "选择审批模式：请求批准" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /^完全访问权限/ }));
    await waitFor(() => expect(invoke.mock.calls.some(([command, args]) => command === "ai_conversation_update"
      && args.request.conversationId === "conv-2"
      && args.request.approvalMode === "full-access")).toBe(true));

    fireEvent.click(document.querySelector(".ai-conversation-open[title='自动审批对话']") as HTMLButtonElement);
    await waitFor(() => expect(screen.getByRole("button", { name: "选择审批模式：帮我批准" })).toBeTruthy());
    fireEvent.click(document.querySelector(".ai-conversation-open[title='请求审批对话']") as HTMLButtonElement);
    await waitFor(() => expect(screen.getByRole("button", { name: "选择审批模式：完全访问权限" })).toBeTruthy());
  });

  test("Windows 沙箱未就绪时阻止直接模型发送并可打开设置", async () => {
    const onOpenAgents = vi.fn();
    await renderEmptyWorkbench(snapshot(conversationContext()).agents, onOpenAgents, [], "windows", "notConfigured");
    fireEvent.click(screen.getByRole("button", { name: "选择模型：选择模型" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4/ }));

    const sandboxGate = await screen.findByRole("button", { name: "Windows 安全沙箱未初始化，打开设置" });
    const sendButton = screen.getByRole("button", { name: "开始对话" });
    expect(sendButton).toHaveProperty("disabled", true);
    const input = screen.getByLabelText("描述要处理的任务");
    fireEvent.change(input, { target: { value: "检查 Windows 工作区" } });
    fireEvent.keyDown(input, { key: "Enter", shiftKey: false, keyCode: 13 });
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(false);

    fireEvent.click(sandboxGate);
    expect(onOpenAgents).toHaveBeenCalledTimes(1);
  });

  test("新建对话后保留用户选择的请求审批模式", async () => {
    await renderEmptyWorkbench();
    selectDefaultStarterAgent();
    fireEvent.click(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4.*上下文 512K/ }));
    fireEvent.click(screen.getByRole("button", { name: "选择审批模式：请求批准" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /^请求批准/ }));

    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "检查系统运行状态" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    const startCall = invoke.mock.calls.find(([command]) => command === "ai_conversation_submit");
    expect(startCall?.[1]?.request.permissionMode).toBe("request");
    await waitFor(() => expect(screen.getByRole("button", { name: "选择审批模式：请求批准" })).toBeTruthy());
  });

  test("Codex 原生审批请求显示在对话中并回复原运行", async () => {
    const run: AiRuntimeSnapshot["runs"][number] = {
      runId: "run-approval", conversationId: "conv-1", agentId: "agent-1", inputMessageId: "message-existing",
      retryOfRunId: null, provider: "codex", model: "Qwen3.8-27B-NVFP4", executionTarget: "model",
      status: "waiting_permission", pid: 42, startedAt: Date.now(), completedAt: null, output: null, errorReason: null,
      sessionId: "thread-1", approvalMode: "request", skillIds: [], toolPermissions: ["read-files", "write-files"],
      permissionRequestId: "permission-1", automationRuleId: null, automationRuleVersion: null, sandboxMode: "workspace-write",
      events: [], artifacts: [], usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 }, logTail: [],
    };
    const permission: AiRuntimeSnapshot["permissionRequests"][number] = {
      permissionRequestId: "permission-1", runId: run.runId, agentId: "agent-1", workspaceRoot: workspace.localRoot,
      directoryScope: "runtime-request", tools: ["execute-command"], networkAccess: false, credentialLabels: [],
      reason: "需要运行测试验证修改", riskLevel: "runtime", status: "pending", requestedAt: Date.now(),
      validUntil: Date.now() + 60_000, decidedAt: null, decidedBy: null, protocolRequestId: 41,
      protocolMethod: "item/commandExecution/requestApproval", protocolItemId: "command-1", command: "npm test",
      resource: workspace.localRoot, requestedPermissions: null, availableDecisions: ["accept", "acceptForSession", "decline"],
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [run], permissionRequests: [permission] });

    expect(screen.getByText("允许执行这条命令？")).toBeTruthy();
    expect(screen.getByText("npm test")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "允许一次" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_permission_request_resolve", { request: {
      permissionRequestId: "permission-1", decision: "approved", decidedBy: "当前用户",
    } }));
  });

  test("应用命令写入展示精确参数且不提供会话级授权", async () => {
    const run: AiRuntimeSnapshot["runs"][number] = {
      runId: "run-application-write", conversationId: "conv-1", agentId: "agent-1", inputMessageId: "message-existing",
      retryOfRunId: null, provider: "codex", model: "Qwen3.8-27B-NVFP4", executionTarget: "model",
      status: "waiting_permission", pid: 42, startedAt: Date.now(), completedAt: null, output: null, errorReason: null,
      sessionId: "thread-1", approvalMode: "request", skillIds: [], toolPermissions: ["read-files"],
      permissionRequestId: "permission-application-write", automationRuleId: null, automationRuleVersion: null, sandboxMode: "read-only",
      events: [], artifacts: [], usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 }, logTail: [],
    };
    const permission: AiRuntimeSnapshot["permissionRequests"][number] = {
      permissionRequestId: "permission-application-write", runId: run.runId, agentId: "agent-1", workspaceRoot: workspace.localRoot,
      directoryScope: "application-write", tools: ["submit_review"], networkAccess: true, credentialLabels: [],
      reason: "确认通过环保档案执行“提交审核”", riskLevel: "write", status: "pending", requestedAt: Date.now(),
      validUntil: Date.now() + 60_000, decidedAt: null, decidedBy: null, protocolRequestId: null,
      protocolMethod: "application/mcp/write", protocolItemId: "cilai_write_submit_review_123", command: null,
      resource: "环保档案", requestedPermissions: null, availableDecisions: ["accept", "decline"],
      applicationWrite: {
        confirmationId: "33333333-3333-4333-8333-333333333333",
        applicationId: "com.cilai.environmental.archive",
        applicationName: "环保档案",
        toolName: "submit_review",
        toolTitle: "提交审核",
        description: "提交指定档案版本进入审核。",
        riskLevel: "write",
        arguments: { versionId: "version-1", expectedDocumentRevision: 3 },
        executionStatus: "awaiting_confirmation",
        confirmationView: {
          reviewMode: "actionConfirmation",
          title: "提交档案审核",
          submitLabel: "提交审核",
          impact: "当前精确档案版本将进入人工审核流程。",
          fields: [
            { name: "versionId", label: "档案版本", visibility: "primary" },
            { name: "expectedDocumentRevision", label: "数据修订", visibility: "technical" },
          ],
        },
      },
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [run], permissionRequests: [permission] });

    expect(screen.getByText("提交档案审核")).toBeTruthy();
    expect(screen.getByText("档案版本")).toBeTruthy();
    expect(screen.getByText("version-1")).toBeTruthy();
    fireEvent.click(screen.getByText("技术信息"));
    expect(screen.getByText("数据修订")).toBeTruthy();
    expect(screen.getByText("3")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "本次会话允许" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "提交审核" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_permission_request_resolve", { request: {
      permissionRequestId: "permission-application-write", decision: "approved", decidedBy: "当前用户",
    } }));
  });

  test("应用业务提案允许修改已声明字段并一次提交最终参数", async () => {
    const run: AiRuntimeSnapshot["runs"][number] = {
      runId: "run-editable-proposal", conversationId: "conv-1", agentId: "agent-1", inputMessageId: "message-existing",
      retryOfRunId: null, provider: "codex", model: "Qwen3.8-27B-NVFP4", executionTarget: "model",
      status: "waiting_permission", pid: 42, startedAt: Date.now(), completedAt: null, output: null, errorReason: null,
      sessionId: "thread-1", approvalMode: "request", skillIds: [], toolPermissions: ["read-files"],
      permissionRequestId: "permission-editable-proposal", automationRuleId: null, automationRuleVersion: null, sandboxMode: "read-only",
      events: [], artifacts: [], usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 }, logTail: [],
    };
    const permission: AiRuntimeSnapshot["permissionRequests"][number] = {
      permissionRequestId: "permission-editable-proposal", runId: run.runId, agentId: "agent-1", workspaceRoot: workspace.localRoot,
      directoryScope: "application-write", tools: ["create_task"], networkAccess: true, credentialLabels: [],
      reason: "审核环保档案整改任务提案", riskLevel: "write", status: "pending", requestedAt: Date.now(),
      validUntil: Date.now() + 60_000, decidedAt: null, decidedBy: null, protocolRequestId: null,
      protocolMethod: "application/mcp/write", protocolItemId: "cilai_write_create_task_123", command: null,
      resource: "环保档案", requestedPermissions: null, availableDecisions: ["accept", "decline"],
      applicationWrite: {
        confirmationId: "44444444-4444-4444-8444-444444444444",
        applicationId: "com.cilai.environmental.archive", applicationName: "环保档案",
        toolName: "create_task", toolTitle: "创建整改任务", description: "创建整改任务。", riskLevel: "write",
        arguments: { title: "补齐排污许可证", findingId: "finding-1", operationId: "generated" },
        executionStatus: "awaiting_confirmation",
        confirmationView: {
          reviewMode: "editableProposal", title: "创建整改任务", submitLabel: "创建任务", impact: "将创建一项整改任务。",
          fields: [
            { name: "title", label: "任务名称", visibility: "primary", editable: true },
            { name: "findingId", label: "异常项", visibility: "technical" },
          ],
        },
      },
    };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [run], permissionRequests: [permission] });

    fireEvent.change(screen.getByRole("textbox", { name: "任务名称" }), { target: { value: "补齐排污许可证原件" } });
    fireEvent.click(screen.getByRole("button", { name: "创建任务" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_permission_request_resolve", { request: {
      permissionRequestId: "permission-editable-proposal", decision: "approved", decidedBy: "当前用户",
      applicationWriteArguments: { title: "补齐排污许可证原件" },
    } }));
  });

  test("应用操作完成后在对话历史中保留状态和正式回执", async () => {
    const inputMessage = { ...existingMessage, runId: "run-application-finished" };
    const run: AiRuntimeSnapshot["runs"][number] = {
      runId: "run-application-finished", conversationId: "conv-1", agentId: "agent-1", inputMessageId: inputMessage.messageId,
      retryOfRunId: null, provider: "codex", model: "Qwen3.8-27B-NVFP4", executionTarget: "model",
      status: "completed", pid: null, startedAt: 2, completedAt: 3, output: "已提交", errorReason: null,
      sessionId: "thread-1", approvalMode: "request", skillIds: [], toolPermissions: [], permissionRequestId: null,
      automationRuleId: null, automationRuleVersion: null, sandboxMode: "read-only", events: [], artifacts: [],
      usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 }, logTail: [],
    };
    const permission: AiRuntimeSnapshot["permissionRequests"][number] = {
      permissionRequestId: "permission-finished", runId: run.runId, agentId: "agent-1", workspaceRoot: workspace.localRoot,
      directoryScope: "application-write", tools: ["submit_review"], networkAccess: true, credentialLabels: [],
      reason: "确认提交", riskLevel: "write", status: "consumed", requestedAt: 2, validUntil: 10_000,
      decidedAt: 3, decidedBy: "当前用户", protocolMethod: "application/mcp/write", availableDecisions: [],
      applicationWrite: {
        confirmationId: "33333333-3333-4333-8333-333333333333", applicationId: "com.cilai.environmental.archive",
        applicationName: "环保档案", toolName: "submit_review", toolTitle: "提交审核",
        description: "提交指定档案版本进入审核。", riskLevel: "write", arguments: { versionId: "version-1" },
        executionStatus: "succeeded", receipt: { invocationId: "invocation-1", status: "succeeded", result: { reviewRequestId: "review-1" } },
        confirmationView: { title: "提交档案审核", submitLabel: "提交审核", impact: "当前精确档案版本将进入人工审核流程。", fields: [{ name: "versionId", label: "档案版本", visibility: "primary" }] },
      },
    };

    await renderWorkbench(conversationContext(), { messages: [inputMessage], runs: [run], permissionRequests: [permission] });

    expect(screen.getByLabelText("提交档案审核操作卡")).toBeTruthy();
    expect(screen.getByText("已提交审核")).toBeTruthy();
    fireEvent.click(screen.getByText("技术信息"));
    expect(screen.getByText("invocation-1")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "提交审核" })).toBeNull();
  });

  test("直接模型回复显示真实模型名且添加菜单不暴露 Agent Skills", async () => {
    const modelRun: AiRuntimeSnapshot["runs"][number] = {
      runId: "run-model", conversationId: "conv-1", agentId: "agent-1", inputMessageId: "message-existing",
      retryOfRunId: null, provider: "codex", model: "Qwen3.8-27B-NVFP4", executionTarget: "model",
      status: "completed", pid: null, startedAt: 2, completedAt: 3, output: "模型回复", errorReason: null,
      sessionId: "thread-model", skillIds: [], toolPermissions: ["read-files", "search-workspace"],
      permissionRequestId: null, automationRuleId: null, automationRuleVersion: null, sandboxMode: "read-only",
      events: [], artifacts: [], usage: { inputTokens: 100, cachedInputTokens: 20, outputTokens: 30 }, logTail: [],
    };
    const modelMessage: AiRuntimeSnapshot["messages"][number] = {
      messageId: "message-model", conversationId: "conv-1", role: "assistant", content: "模型回复", attachments: [], runId: "run-model", createdAt: 3,
    };

    await renderWorkbench(conversationContext(), { messages: [existingMessage, modelMessage], runs: [modelRun] });

    expect(document.querySelector(".ai-message.assistant header span")?.textContent).toBe("Qwen3.8 27B NVFP4");
    fireEvent.click(screen.getByRole("button", { name: "添加上下文" }));
    const addMenu = screen.getByRole("menu", { name: "添加上下文" });
    expect(within(addMenu).queryByText("文档摘要")).toBeNull();
    expect(within(addMenu).queryByText(/技能/)).toBeNull();
  });

  test("项目内已创建对话不显示项目选择控件", async () => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "主项目" }), { projectIndex: defaultProjectIndex });
    expect(screen.queryByRole("button", { name: /选择对话项目/ })).toBeNull();
    expect(document.querySelector(".ai-starter-project-picker")).toBeNull();
  });

  test("当前工作区没有会话时不回选其他工作区的会话上下文", async () => {
    await renderWorkbench(conversationContext({
      workspaceId: secondWorkspace.workspaceId,
      workspaceName: secondWorkspace.displayName,
      relativePath: "其他工作区/机密.md",
    }), { rememberConversation: false });

    expect(screen.getByRole("heading", { name: "Hi，今天有什么安排？" })).toBeTruthy();
    expect(screen.queryByText("机密.md")).toBeNull();
  });

  test("启动页只有显式点击添加后才带入当前文件", async () => {
    await renderEmptyWorkbench();
    selectDefaultStarterAgent();
    fireEvent.click(screen.getByRole("button", { name: "添加上下文" }));
    expect(screen.getByRole("menu", { name: "添加上下文" })).toBeTruthy();
    fireEvent.click(screen.getByRole("menuitem", { name: /工作区文件/ }));
    const picker = screen.getByRole("dialog", { name: "选择工作区文件" });
    expect(screen.getByLabelText("搜索工作区文件").parentElement?.getAttribute("data-focus-frame")).toBe("wrapper");
    fireEvent.click(within(picker).getByRole("button", { name: /项目目录结构与产品分类/ }));
    fireEvent.click(within(picker).getByRole("button", { name: "添加" }));
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "检查这份文件" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    const startCall = invoke.mock.calls.find(([command]) => command === "ai_conversation_submit");
    expect(startCall?.[1]?.request).toMatchObject({
      conversationType: "direct",
      context: { localFileId: null, relativePath: null, referencedMaterials: [] },
      referencedMaterials: [{ localFileId: selectedFile.localFileId, relativePath, workspaceId: workspace.workspaceId }],
    });
  });

  test("已有会话选择文件立即进入草稿，发送前不更新会话或刷新快照", async () => {
    await renderWorkbench(conversationContext({ localFileId: null, relativePath: null, fileRevision: null }), { messages: [existingMessage] });

    fireEvent.click(screen.getByRole("button", { name: "添加上下文" }));
    expect(screen.getByRole("menu", { name: "添加上下文" })).toBeTruthy();
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_update")).toBe(false);

    fireEvent.click(screen.getByRole("menuitem", { name: /工作区文件/ }));
    const picker = screen.getByRole("dialog", { name: "选择工作区文件" });
    fireEvent.click(within(picker).getByRole("button", { name: /项目目录结构与产品分类/ }));
    fireEvent.click(within(picker).getByRole("button", { name: "添加" }));

    expect(screen.getByLabelText("待发送文件").textContent).toContain(fileNameFromPath(relativePath));
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_update")).toBe(false);
    const snapshots = invoke.mock.calls.filter(([command]) => command === "ai_runtime_snapshot").length;
    await act(() => flushConversationSessions());
    expect(invoke.mock.calls.filter(([command]) => command === "ai_runtime_snapshot")).toHaveLength(snapshots);
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    expect(screen.queryByLabelText("待发送文件")).toBeNull();
    expect(screen.getByLabelText("消息文件").textContent).toContain(fileNameFromPath(relativePath));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_message_submit")?.[1].request).toMatchObject({
      content: "", referencedMaterials: [{ localFileId: selectedFile.localFileId, relativePath, fileRevision: 3, workspaceId: "ws-1" }],
    });
  });

  test("普通自然语言意图不会启用企业应用工具", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages: [existingMessage] };
      if (command === "ai_message_submit") return {};
      throw new Error(`unexpected command ${command}`);
    });

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "请用客户管理读取当前客户" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    const request = invoke.mock.calls.find(([command]) => command === "ai_message_submit")?.[1].request;
    expect(request.applicationContext).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "application_catalog_list")).toBe(false);
  });

  const conversationSkillCatalog: SkillConversationCatalogItem[] = [
      { skillId: "system.skill-creator", name: "技能创建器", description: "创建新技能", source: "system", automatic: true, stateRevision: 2 },
      { skillId: "system.capability-creator", name: "能力创建器", description: "创建个人能力", source: "system", automatic: true, stateRevision: 1 },
      { skillId: "skill-weekly", name: "周报整理", description: "按统一结构整理项目周报", source: "user", automatic: true, stateRevision: 3 },
  ];
test("从技能页面发起新对话时保留技能与输入，不自动选首个连接模型", async () => {
  await renderWorkbench(conversationContext(), { rememberModel: false, conversationSkills: conversationSkillCatalog,
    launchRequest: { token: 10001, title: "使用周报技能", conversationType: "direct", mode: "discuss", context: conversationContext(),
      selectedSkillId: "skill-weekly", prefill: "保留技能页面带来的任务输入" } });
  expect(screen.getByRole("button", { name: "选择模型：选择模型" })).toBeTruthy();
  expect(screen.getByLabelText("描述要处理的任务").textContent).toBe("保留技能页面带来的任务输入");
  expect(screen.getByText("周报整理")).toBeTruthy();
  expect(invoke.mock.calls.some(([command]) => ["ai_conversation_submit", "ai_message_submit", "ai_conversation_start"].includes(command))).toBe(false);
});



  test("对话展示服务端的技能结果并隐藏完整交付数据", async () => {
    const run = { runId: "run-created-skill", conversationId: "conv-1", status: "completed", startedAt: 2, completedAt: 3,
      artifacts: [], events: [], skillIds: ["system.skill-creator"], executionTarget: "model" } as unknown as AiRuntimeSnapshot["runs"][number];
    const assistant = { ...existingMessage, messageId: "message-skill", role: "assistant" as const, runId: run.runId, createdAt: 3,
      content: '交付完成。\n\n```cilai-skill\n{"name":"复杂技能","description":"说明","instructions":"nested ```bash","files":[]}\n```' };
    await renderWorkbench(conversationContext(), {
      messages: [existingMessage, assistant], runs: [run],
      conversationSkills: [{ skillId: "skill-created", name: "复杂技能", description: "说明", source: "user", automatic: false, stateRevision: 1 }],
      skillInstallations: [{ runId: run.runId, messageId: assistant.messageId, status: "installed", skillId: "skill-created", name: "复杂技能",
        errorReason: null, errorDetail: null, retryable: false, displayContent: "交付完成。" }],
    });
    await waitFor(() => expect(screen.getByText("已添加 · 复杂技能")).toBeTruthy());
    expect(screen.queryByText(/nested/)).toBeNull();
    expect(invoke.mock.calls.filter(([command, args]) => command === "app_server_request" && ["skill/save", "skill/importDirectory", "skill/runRetry"].includes(args?.method))).toHaveLength(0);
    fireEvent.click(screen.getByRole("button", { name: "试用" }));
    expect(screen.getByLabelText("发送给 AI").textContent).toBe("使用复杂技能，");
  });

  test("普通请求不由前端关键词规则指定技能", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage], conversationSkills: conversationSkillCatalog });

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "请按统一结构整理项目周报" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    const request = invoke.mock.calls.find(([command]) => command === "ai_message_submit")?.[1].request;
    expect(request.selectedSkillId).toBeNull();
    expect(request.content).toBe("请按统一结构整理项目周报");
  });

  test("手动指定技能通过斜杠完成", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage], conversationSkills: conversationSkillCatalog });

    expect(screen.queryByRole("button", { name: "选择技能" })).toBeNull();
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "/" } });
    const palette = screen.getByLabelText("选择技能");
    expect(within(palette).getAllByLabelText("系统技能").every((item) => item.textContent === "系统")).toBe(true);
    expect(within(palette).getByLabelText("个人技能").textContent).toBe("个人");
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "/周报" } });
    const skillChoice = await screen.findByRole("button", { name: /^\/周报整理/ });
    expect(within(screen.getByLabelText("选择技能")).getByRole("region", { name: "技能" })).toBeTruthy();
    fireEvent.click(skillChoice);

    const composerInput = screen.getByLabelText("发送给 AI").closest(".ai-composer-input") as HTMLElement;
    expect(within(composerInput).getByText("周报整理")).toBeTruthy();
    expect(screen.getByLabelText("发送给 AI").textContent).toBe("");

    setComposerText("发送给 AI", "整理本周项目进展");
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    const request = invoke.mock.calls.find(([command]) => command === "ai_message_submit")?.[1].request;
    expect(request.selectedSkillId).toBe("skill-weekly");
    expect(request.content).toBe("整理本周项目进展");
    await waitFor(() => expect(screen.queryByText("周报整理")).toBeNull());
  });

  test("能力创建器在同一对话的澄清回合中保持选中", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage], conversationSkills: conversationSkillCatalog });
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "/能力" } });
    fireEvent.click(await screen.findByRole("button", { name: /^\/能力创建器/ }));

    setComposerText("发送给 AI", "做一个本地文件检查工具");
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_message_submit")?.[1].request.selectedSkillId).toBe("system.capability-creator");
    expect(screen.getByText("能力创建器")).toBeTruthy();
  });

  test("未选择模型时使用技能会保留斜杠输入，不自动选第一条连接", async () => {
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { rememberModel: false, messages: [existingMessage], conversationSkills: conversationSkillCatalog, onMessage });
    setComposerText("发送给 AI", "/周报");
    fireEvent.click(await screen.findByRole("button", { name: /^\/周报整理/ }));
    expect(onMessage).toHaveBeenCalledWith("请先选择模型，再使用技能。", "error");
    expect(screen.getByLabelText("发送给 AI")).toHaveProperty("value", "/周报");
    expect(window.localStorage.getItem("cilai.workbench.ai-last-execution-selection")).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
  });

  test("空正文中的技能标签可通过退格键移除", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage], conversationSkills: conversationSkillCatalog });
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "/周报" } });
    fireEvent.click(await screen.findByRole("button", { name: /^\/周报整理/ }));

    const editor = screen.getByLabelText("发送给 AI");
    setComposerText("发送给 AI", "a");
    fireEvent.keyDown(editor, { key: "Backspace" });
    expect(screen.getByText("周报整理")).toBeTruthy();

    setComposerText("发送给 AI", "");
    expect(editor.textContent).toBe("");
    fireEvent.keyDown(editor, { key: "Backspace" });

    await waitFor(() => expect(screen.queryByText("周报整理")).toBeNull());
    expect(screen.getByLabelText("发送给 AI")).toBeInstanceOf(HTMLTextAreaElement);
  });

  test("技能标签融入正文首行且换行不保留独立左列", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage], conversationSkills: conversationSkillCatalog });
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "/周报" } });
    fireEvent.click(await screen.findByRole("button", { name: /^\/周报整理/ }));

    const editor = screen.getByLabelText("发送给 AI");
    const composerInput = editor.closest(".ai-composer-input") as HTMLElement;
    const prefix = composerInput.querySelector(".ai-invocation-input-prefix") as HTMLElement;
    expect(prefix).toBeTruthy();
    expect(prefix.contains(within(composerInput).getByText("周报整理"))).toBe(true);
    expect(composerInput.firstElementChild).toBe(prefix);
    expect(prefix.nextElementSibling).toBe(editor);
    expect(editor).not.toBeInstanceOf(HTMLTextAreaElement);
    expect(editor.getAttribute("contenteditable")).toBe("true");
    setComposerText("发送给 AI", "这是一段会在输入框中自动换行的较长正文内容");
    expect(editor.textContent).toBe("这是一段会在输入框中自动换行的较长正文内容");
  });

  test("斜杠只读取本地技能，不请求企业应用目录", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage], conversationSkills: conversationSkillCatalog });
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "/" } });
    expect(await screen.findByRole("button", { name: /^\/周报整理/ })).toBeTruthy();
    expect(screen.queryByRole("region", { name: "应用" })).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "application_catalog_list")).toBe(false);
  });

  test("应用后继续输入工具路径时按普通文本发送", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages: [existingMessage] };
      if (command === "ai_message_submit") return {};
      throw new Error(`unexpected command ${command}`);
    });

    const content = "/客户管理/查询客户 读取当前客户";
    setComposerText("发送给 AI", content);
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    const request = invoke.mock.calls.find(([command]) => command === "ai_message_submit")?.[1].request;
    expect(request.content).toBe(content);
    expect(request.applicationContext).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "application_catalog_list")).toBe(false);
  });

  test("旧应用草稿明确拒绝并保留正文，移除引用后可发送", async () => {
    sessionApi.snapshot.values.applicationContextSelections = {
      "conversation:conv-1": { applicationId: "old-enterprise-app", contextIds: [], toolNames: [], invocationSource: "slash" },
    };
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { messages: [existingMessage], onMessage });
    await screen.findByRole("button", { name: "移除旧应用引用" });
    setComposerText("发送给 AI", "保留这份本地草稿");
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith("旧应用引用已停用，请移除后发送。", "error"));
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit" || command === "application_catalog_list")).toBe(false);
    expect(screen.getByLabelText("发送给 AI").textContent).toBe("保留这份本地草稿");
    fireEvent.click(screen.getByRole("button", { name: "移除旧应用引用" }));
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_message_submit")?.[1].request).toMatchObject({ content: "保留这份本地草稿", applicationContext: null });
  });

  test("旧应用排队消息重试仍保留失败记录，不恢复企业调用", async () => {
    const queued = {
      queueId: "retired-queued", queuedAt: 1, conversationId: "conv-1", content: "保留原排队正文", attachments: [],
      applicationContext: { applicationId: "old-enterprise-app", contextIds: [], toolNames: [], invocationSource: "slash" },
      requiresApplicationResolution: false, modelOverride: "Qwen3.8-27B-NVFP4", reasoningEffort: null,
      executionTarget: "model", permissionMode: "request", status: "failed",
    };
    sessionApi.snapshot.values.queuedFollowUps = [queued];
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { messages: [existingMessage], onMessage });
    const queue = await screen.findByRole("region", { name: "待发送的后续指令" });
    fireEvent.click(within(queue).getByRole("button", { name: "重试排队消息 1" }));
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith("旧应用引用已停用，请移除后发送。", "error"));
    await act(() => flushConversationSessions());
    expect(sessionApi.snapshot.values.queuedFollowUps).toEqual([queued]);
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit" || command === "application_catalog_list")).toBe(false);
  });

  test("分隔线暴露当前宽度和允许范围", async () => {
    await renderWorkbench();
    const separator = screen.getByRole("separator", { name: "调整对话记录宽度" });
    const workbench = document.querySelector(".ai-workbench") as HTMLElement;

    expect(separator.getAttribute("aria-valuemin")).toBe("280");
    expect(separator.getAttribute("aria-valuemax")).toBe("520");
    expect(separator.getAttribute("aria-valuenow")).toBe("304");
    expect(workbench.style.getPropertyValue("--ai-conversation-rail-width")).toBe("304px");
  });

  test("拖动分隔线连续调整宽度并在结束时保存", async () => {
    await renderWorkbench();
    const separator = screen.getByRole("separator", { name: "调整对话记录宽度" });

    fireEvent(separator, new MouseEvent("pointerdown", { bubbles: true, button: 0, clientX: 304 }));
    fireEvent(window, new MouseEvent("pointermove", { bubbles: true, clientX: 384 }));
    expect(separator.getAttribute("aria-valuenow")).toBe("384");
    fireEvent(window, new MouseEvent("pointerup", { bubbles: true }));

    expect(window.localStorage.getItem("cilai.workbench.ai-conversation-rail-width")).toBe("384");
  });

  test("保存的宽度会恢复，键盘调整仍受上下边界限制", async () => {
    window.localStorage.setItem("cilai.workbench.ai-conversation-rail-width", "400");
    await renderWorkbench();
    const separator = screen.getByRole("separator", { name: "调整对话记录宽度" });

    expect(separator.getAttribute("aria-valuenow")).toBe("400");
    fireEvent.keyDown(separator, { key: "ArrowRight" });
    expect(separator.getAttribute("aria-valuenow")).toBe("416");
    fireEvent.keyDown(separator, { key: "Home" });
    expect(separator.getAttribute("aria-valuenow")).toBe("280");
    fireEvent.keyDown(separator, { key: "ArrowLeft" });
    expect(separator.getAttribute("aria-valuenow")).toBe("280");
    fireEvent.keyDown(separator, { key: "End" });
    expect(separator.getAttribute("aria-valuenow")).toBe("520");
    fireEvent.keyDown(separator, { key: "ArrowRight" });
    expect(separator.getAttribute("aria-valuenow")).toBe("520");
    expect(window.localStorage.getItem("cilai.workbench.ai-conversation-rail-width")).toBe("520");
  });

  test("双击分隔线恢复默认宽度", async () => {
    window.localStorage.setItem("cilai.workbench.ai-conversation-rail-width", "460");
    await renderWorkbench();
    const separator = screen.getByRole("separator", { name: "调整对话记录宽度" });

    fireEvent.doubleClick(separator);

    expect(separator.getAttribute("aria-valuenow")).toBe("304");
    expect(window.localStorage.getItem("cilai.workbench.ai-conversation-rail-width")).toBe("304");
  });

  test("从对话工具列收起并重新展开对话记录", async () => {
    await renderWorkbench();

    const collapse = screen.getByRole("button", { name: "收起对话记录" });
    expect(collapse.querySelector("rect")?.getAttribute("width")).toBe("16");
    expect(collapse.querySelector("rect")?.getAttribute("height")).toBe("12");
    expect(collapse.querySelector("path")?.getAttribute("d")).toBe("M9 8.5v7");
    fireEvent.click(collapse);
    expect(document.querySelector(".ai-conversation-rail")?.hasAttribute("hidden")).toBe(true);
    expect(document.querySelector(".ai-workbench")?.classList.contains("conversation-rail-collapsed")).toBe(true);

    const expand = screen.getByRole("button", { name: "展开对话记录" });
    expect(expand.querySelector("rect")?.getAttribute("width")).toBe("16");
    expect(expand.querySelector("rect")?.getAttribute("height")).toBe("12");
    expect(expand.querySelector("path")?.getAttribute("d")).toBe("M7 8.5v7");
    fireEvent.click(expand);
    expect(document.querySelector(".ai-conversation-rail")?.hasAttribute("hidden")).toBe(false);
    expect(screen.getByRole("button", { name: "收起对话记录" }).querySelector("path")?.getAttribute("d")).toBe("M9 8.5v7");
    expect(screen.getByRole("separator", { name: "调整对话记录宽度" }).getAttribute("aria-valuenow")).toBe("304");
  });

});

describe("从文件页发起对话", () => {
  test("只有一个关联项目时直接进入文件草稿，首次发送才创建文件对话", async () => {
    let state = { ...snapshot(conversationContext()), conversations: [], messages: [] } as unknown as AiRuntimeSnapshot;
    const onLaunchConsumed = vi.fn();
    const onSelectProject = vi.fn();
    invoke.mockImplementation(async (command: string, args: { request: Record<string, unknown> }) => {
      if (command === "ai_runtime_snapshot") return state;
      if (command === "ai_conversation_submit") {
        const started = startResult(args.request);
        state = { ...state, conversations: [started.conversation], messages: [started.message], runs: [started.run] };
        return { status: "recorded", conversation: started.conversation, run: started.run };
      }
      throw new Error(command);
    });

    render(<AiWorkbenchPage
      workspace={workspace}
      projects={projects}
      files={[selectedFile]}
      selectedFile={selectedFile}
      launchRequest={{ token: 101, title: `讨论 ${relativePath}`, conversationType: "file", mode: "discuss", context: workspaceContext(workspace, selectedFile, projects[0]), projectIds: ["proj-1"] }}
      preferredConversationId={null}
      onMessage={() => {}}
      onOpenAgents={() => {}}
      canCreateProject
      onCreateProject={() => {}}
      onLaunchConsumed={onLaunchConsumed}
      onSelectProject={onSelectProject}
      projectIndex={defaultProjectIndex}/>);

    await waitFor(() => expect(onSelectProject).toHaveBeenCalledWith("proj-1"));
    expect(screen.queryByRole("dialog", { name: "选择新对话的项目" })).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create" || command === "ai_conversation_submit")).toBe(false);
    expect(screen.getByLabelText("待发送文件").textContent).toContain("项目目录结构");
    expect(document.querySelector(".ai-starter-project-row")).toBeNull();
    selectDefaultStarterAgent();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "检查当前文件" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1].request).toMatchObject({
      title: "检查当前文件",
      conversationType: "file",
      context: { projectId: "proj-1", localFileId: null },
      referencedMaterials: [{ localFileId: "file-1", relativePath, workspaceId: "ws-1" }],
    });
    expect(onLaunchConsumed).toHaveBeenCalledWith(101);
    expect(onSelectProject).toHaveBeenCalledWith("proj-1");
  });

  test("多个关联项目时只展示候选项目，选择后仍保留文件引用", async () => {
    const sharedProject = { ...projects[1], workspaceIds: [workspace.workspaceId, secondWorkspace.workspaceId] };
    const unrelatedProject = { ...projects[1], projectId: "proj-3", displayName: "无关项目" };
    let state = { ...snapshot(conversationContext()), conversations: [], messages: [] } as unknown as AiRuntimeSnapshot;
    invoke.mockImplementation(async (command: string, args: { request: Record<string, unknown> }) => {
      if (command === "ai_runtime_snapshot") return state;
      if (command === "ai_conversation_submit") {
        const started = startResult(args.request);
        state = { ...state, conversations: [started.conversation], messages: [started.message], runs: [started.run] };
        return { status: "recorded", conversation: started.conversation, run: started.run };
      }
      throw new Error(command);
    });

    render(<AiWorkbenchPage
      workspace={workspace}
      projects={[projects[0], sharedProject, unrelatedProject]}
      files={[selectedFile]}
      selectedFile={selectedFile}
      launchRequest={{ token: 102, title: `讨论 ${relativePath}`, conversationType: "file", mode: "discuss", context: workspaceContext(workspace, selectedFile, null), projectIds: ["proj-1", "proj-2"] }}
      preferredConversationId={null}
      onMessage={() => {}}
      onOpenAgents={() => {}}
      canCreateProject
      onCreateProject={() => {}}
      onSelectProject={() => {}}
      projectIndex={defaultProjectIndex}/>);

    const dialog = await screen.findByRole("dialog", { name: "选择新对话的项目" });
    expect(within(dialog).getByText("主项目")).toBeTruthy();
    expect(within(dialog).getByText("第二项目")).toBeTruthy();
    expect(within(dialog).queryByText("无关项目")).toBeNull();
    expect((within(dialog).getByRole("button", { name: "新建项目" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(within(dialog).getByText("第二项目"));

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "选择新对话的项目" })).toBeNull());
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(false);
    selectDefaultStarterAgent();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "分析文件" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1].request).toMatchObject({
      conversationType: "file",
      context: { workspaceId: "ws-1", projectId: "proj-2", projectName: "第二项目", localFileId: null },
      referencedMaterials: [{ localFileId: "file-1", relativePath, workspaceId: "ws-1" }],
    });
  });
});

describe("新建对话项目归属", () => {
  test("项目行直接进入该项目草稿，切换后仍保留未发送内容", async () => {
    const state = { ...snapshot(conversationContext()), conversations: [], messages: [], runs: [] } as unknown as AiRuntimeSnapshot;
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return state;
      throw new Error(`unexpected command ${command}`);
    });
    render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[]} selectedFile={null} launchRequest={null} preferredConversationId={null}
      onMessage={() => {}} onOpenAgents={() => {}} canCreateProject onCreateProject={() => {}} onSelectProject={() => {}} projectIndex={defaultProjectIndex}/>);

    fireEvent.click(await screen.findByLabelText("在 主项目 中新建对话"));
    expect(screen.queryByRole("dialog", { name: "选择新对话的项目" })).toBeNull();
    await waitFor(() => expect(screen.queryByRole("button", { name: /选择对话项目/ })).toBeNull());
    const projectInput = screen.getByLabelText("描述要处理的任务") as HTMLTextAreaElement;
    expect(projectInput.placeholder).toBe("输入问题、补充约束或下一步要求…");
    expect(projectInput.closest("form")?.className).toBe("ai-composer");
    expect(projectInput.closest("form")?.querySelectorAll("footer")).toHaveLength(1);
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "主项目未发送草稿" } });
    fireEvent.click(screen.getByLabelText("在 第二项目 中新建对话"));
    expect((screen.getByLabelText("描述要处理的任务") as HTMLTextAreaElement).value).toBe("");
    fireEvent.click(screen.getByLabelText("在 主项目 中新建对话"));
    expect((screen.getByLabelText("描述要处理的任务") as HTMLTextAreaElement).value).toBe("主项目未发送草稿");
    fireEvent.click(screen.getByLabelText("新建 AI 对话"));
    const globalInput = screen.getByLabelText("描述要处理的任务") as HTMLTextAreaElement;
    // 起始状态不再另有一个输入框：带项目行的新对话与进行中的对话是同一个表单，只多一行项目选择
    expect(globalInput.closest("form")?.className).toBe("ai-composer");
    expect(globalInput.closest("form")?.querySelectorAll("footer")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "选择对话项目：未分类" }).closest(".ai-starter-project-row")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "选择对话项目：未分类" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /第二项目/ }));
    expect(screen.getByRole("button", { name: "选择对话项目：第二项目" }).closest(".ai-starter-project-row")).toBeTruthy();
    fireEvent.click(screen.getByLabelText("在 主项目 中新建对话"));
    expect((screen.getByLabelText("描述要处理的任务") as HTMLTextAreaElement).value).toBe("主项目未发送草稿");
    expect(screen.queryByRole("button", { name: /选择对话项目/ })).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create" || command === "ai_conversation_submit")).toBe(false);
  });

  test("记住模型选择但不继承更高权限，恢复历史对话也不改写默认选择", async () => {
    await renderEmptyWorkbench();
    fireEvent.click(screen.getByRole("button", { name: "选择模型：选择模型" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4/ }));
    fireEvent.click(screen.getByRole("button", { name: "选择审批模式：请求批准" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /^完全访问权限/ }));
    await act(async () => cleanup());
    sessionApi.snapshot = { sessionId: "test-session", revision: 0, values: {} };
    await renderWorkbench();
    expect(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" })).toBeTruthy();
    await act(async () => cleanup());
    sessionApi.snapshot = { sessionId: "test-session", revision: 0, values: {} };
    await renderEmptyWorkbench();
    expect(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" })).toBeTruthy();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "整理任务" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1].request).toMatchObject({ executionTarget: "model", modelOverride: "Qwen3.8-27B-NVFP4", permissionMode: "request" });
  });

  test("思考开关按供应商布尔映射显示开启关闭，并保留发送参数", async () => {
    const models = snapshot(conversationContext()).models.map(model => model.modelId === "Qwen3.8-27B-NVFP4" ? {
      ...model, supportedReasoningEfforts: ["none", "medium"] as AiReasoningEffort[],
      defaultReasoningEffort: "medium" as const, reasoningControl: "toggle" as const,
      reasoningEnabledEfforts: ["medium"] as AiReasoningEffort[],
    } : model);
    await renderEmptyWorkbench(snapshot(conversationContext()).agents, undefined, [], "macos", "ready", models, "tenant-a");
    fireEvent.click(await screen.findByRole("button", { name: "选择模型：选择模型" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4/ }));
    expect(screen.getByRole("group", { name: "选择思考模式" })).toBeTruthy();
    const slider = screen.getByRole("slider", { name: "思考模式" });
    expect(slider.getAttribute("aria-valuetext")).toBe("开启");
    fireEvent.change(slider, { target: { value: "0" } });
    expect(slider.getAttribute("aria-valuetext")).toBe("关闭");
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "分析方案" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1].request).toMatchObject({ modelOverride: "Qwen3.8-27B-NVFP4", reasoningEffort: "none" });
  });

  test("模型与思考深度在同一弹窗完成选择并按租户记住完整组合", async () => {
    const models = snapshot(conversationContext()).models.map((model) =>
      model.modelId === "Qwen3.8-27B-NVFP4"
        ? {
            ...model,
            supportedReasoningEfforts: ["low", "medium", "high", "max"] as AiReasoningEffort[],
            defaultReasoningEffort: "medium" as const,
          }
        : {
            ...model,
            supportedReasoningEfforts: ["low", "medium", "xhigh"] as AiReasoningEffort[],
            defaultReasoningEffort: "xhigh" as const,
          },
    ) as AiRuntimeSnapshot["models"];
    await renderEmptyWorkbench(snapshot(conversationContext()).agents, undefined, [], "macos", "ready", models, "tenant-a");
    fireEvent.click(screen.getByRole("button", { name: "选择模型：选择模型" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4/ }));
    expect(screen.getByRole("group", { name: "选择思考深度" })).toBeTruthy();
    const firstSlider = screen.getByRole("slider", { name: "思考深度" });
    expect(firstSlider.getAttribute("aria-valuetext")).toBe("标准");
    fireEvent.pointerDown(firstSlider);
    fireEvent.change(firstSlider, { target: { value: "2" } });
    fireEvent.pointerUp(firstSlider);
    expect(firstSlider.getAttribute("aria-valuetext")).toBe("较高");
    expect(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4 较高" })).toBeTruthy();
    expect(screen.getByRole("group", { name: "选择思考深度" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "切换模型：Qwen3.8 27B NVFP4" }));
    expect(screen.getByRole("group", { name: "模型" })).toBeTruthy();
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.6 35B A3B NVFP4/ }));
    expect(screen.getByRole("group", { name: "选择思考深度" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "切换模型：Qwen3.6 35B A3B NVFP4" })).toBeTruthy();
    fireEvent.change(screen.getByRole("slider", { name: "思考深度" }), { target: { value: "0" } });
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "分析方案" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1].request).toMatchObject({
      modelOverride: "Qwen3.6-35B-A3B-NVFP4",
      reasoningEffort: "low",
    });
    expect(JSON.parse(window.localStorage.getItem("cilai.workbench.ai-last-execution-selection:tenant-a")!)).toEqual({
      agentId: "agent-1",
      modelId: "Qwen3.6-35B-A3B-NVFP4",
      reasoningEffort: "low",
    });

    cleanup();
    await renderEmptyWorkbench(snapshot(conversationContext()).agents, undefined, [], "macos", "ready", models, "tenant-a");
    expect(screen.getByRole("button", { name: "选择模型：Qwen3.6 35B A3B NVFP4 较低" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "选择模型：Qwen3.6 35B A3B NVFP4 较低" }));
    expect(screen.getByRole("group", { name: "选择思考深度" })).toBeTruthy();

    await act(async () => cleanup());
    sessionApi.snapshot = { sessionId: "tenant-b-session", revision: 0, values: {} };
    await renderEmptyWorkbench(snapshot(conversationContext()).agents, undefined, [], "macos", "ready", models, "tenant-b");
    expect(await screen.findByRole("button", { name: "选择模型：选择模型" })).toBeTruthy();
  });

  test("鼠标只在圆形滑块范围内触发轻微放大状态", async () => {
    const models = snapshot(conversationContext()).models.map((model) => model.modelId === "Qwen3.8-27B-NVFP4" ? {
      ...model,
      supportedReasoningEfforts: ["low", "medium", "high", "max"] as AiReasoningEffort[],
      defaultReasoningEffort: "medium" as const,
    } : model) as AiRuntimeSnapshot["models"];
    await renderEmptyWorkbench(snapshot(conversationContext()).agents, undefined, [], "macos", "ready", models);
    fireEvent.click(screen.getByRole("button", { name: "选择模型：选择模型" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4/ }));
    const slider = screen.getByRole("slider", { name: "思考深度" }) as HTMLInputElement;
    const track = slider.parentElement as HTMLElement;
    vi.spyOn(slider, "getBoundingClientRect").mockReturnValue({
      bottom: 82,
      height: 32,
      left: 100,
      right: 340,
      top: 50,
      width: 240,
      x: 100,
      y: 50,
      toJSON: () => ({}),
    });

    fireEvent.mouseMove(slider, { clientX: 185, clientY: 66 });
    expect(track.hasAttribute("data-thumb-hover")).toBe(true);

    fireEvent.mouseMove(slider, { clientX: 300, clientY: 66 });
    expect(track.hasAttribute("data-thumb-hover")).toBe(false);

    fireEvent.mouseMove(slider, { clientX: 185, clientY: 66 });
    fireEvent.mouseLeave(slider);
    expect(track.hasAttribute("data-thumb-hover")).toBe(false);
  });

  test("记住的思考深度已被模型移除时要求重新选择", async () => {
    const models = snapshot(conversationContext()).models.map((model) =>
      model.modelId === "Qwen3.8-27B-NVFP4"
        ? {
            ...model,
            supportedReasoningEfforts: ["low", "medium"] as AiReasoningEffort[],
            defaultReasoningEffort: "medium" as const,
          }
        : model,
    ) as AiRuntimeSnapshot["models"];
    window.localStorage.setItem("cilai.workbench.ai-last-execution-selection:tenant-a", JSON.stringify({
      agentId: "agent-1",
      modelId: "Qwen3.8-27B-NVFP4",
      reasoningEffort: "high",
    }));

    await renderEmptyWorkbench(snapshot(conversationContext()).agents, undefined, [], "macos", "ready", models, "tenant-a");
    expect(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4 重新选择深度" })).toBeTruthy();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "分析方案" } });
    expect((screen.getByRole("button", { name: "开始对话" }) as HTMLButtonElement).disabled).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4 重新选择深度" }));
    expect(screen.getByRole("group", { name: "选择思考深度" })).toBeTruthy();
    const slider = screen.getByRole("slider", { name: "思考深度" });
    fireEvent.pointerDown(slider);
    fireEvent.pointerUp(slider);
    expect(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4 较低" })).toBeTruthy();
    expect((screen.getByRole("button", { name: "开始对话" }) as HTMLButtonElement).disabled).toBe(false);
  });

  test("当前页面选择模型后，为另一项目新建对话也沿用最后选择", async () => {
    await openPicker();
    fireEvent.click(screen.getByRole("button", { name: /^选择模型：/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4/ }));
    fireEvent.click(screen.getByLabelText("新建 AI 对话"));
    fireEvent.click(screen.getByRole("button", { name: "选择对话项目：未分类" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /第二项目/ }));
    expect(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" })).toBeTruthy();
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create" || command === "ai_conversation_submit")).toBe(false);
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "沿用当前模型" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1].request).toMatchObject({ context: { projectId: "proj-2" }, executionTarget: "model", modelOverride: "Qwen3.8-27B-NVFP4" });
  });

  test("记住的模型被删除时明确提示，不自动切换其他执行方式", async () => {
    window.localStorage.setItem("cilai.workbench.ai-last-execution-selection", JSON.stringify({ agentId: "agent-1", modelId: "removed-model" }));
    const onMessage = vi.fn();
    const state = snapshot(conversationContext());
    invoke.mockResolvedValue({ ...state, conversations: [{ ...state.conversations[0], agentId: null }] });
    window.localStorage.setItem("cilai.workbench.ai-last-viewed-conversation", "conv-1");
    render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[]} selectedFile={null} launchRequest={null} preferredConversationId={null} onMessage={onMessage} onOpenAgents={() => {}} canCreateProject onCreateProject={() => {}} onSelectProject={() => {}} projectIndex={defaultProjectIndex}/>);
    await waitFor(() => expect(onMessage).toHaveBeenCalledWith("上次选择的模型已不可用，请重新选择。", "error"));
    expect(screen.getByRole("button", { name: "选择模型：选择模型" })).toBeTruthy();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "整理任务" } });
    expect((screen.getByRole("button", { name: "开始对话" }) as HTMLButtonElement).disabled).toBe(true);
  });

  test("创建项目成功后返回更新后的列表，再选择项目创建对话", async () => {
    const createdProject = { ...projects[1], projectId: "proj-created", displayName: "新创建项目", primaryWorkspaceId: null, workspaceIds: [] as string[] };
    const addWorkspace = vi.fn(async () => secondWorkspace);
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return snapshot(conversationContext());
      if (command === "app_server_request") return { project: createdProject };
      throw new Error(command);
    });
    function Harness() {
      const [creating, setCreating] = useState(false);
      const [options, setOptions] = useState<ComponentProps<typeof AiWorkbenchPage>["projects"]>(projects);
      return <>
        <AiWorkbenchPage workspace={workspace} projects={options} files={[]} selectedFile={null} launchRequest={null} preferredConversationId={null} onMessage={() => {}} onOpenAgents={() => {}} canCreateProject projectCreationOpen={creating} onCreateProject={() => setCreating(true)} onSelectProject={() => {}} projectIndex={defaultProjectIndex}/>
        {creating ? <ProjectCreatePanel existingProjects={projects} workspaces={[secondWorkspace]} onAddWorkspace={addWorkspace} onCancel={() => setCreating(false)} onCreated={() => { setOptions([...projects, createdProject]); setCreating(false); }} onMessage={() => {}}/> : null}
      </>;
    }
    render(<Harness/>);
    fireEvent.click(await screen.findByLabelText("新建 AI 对话"));
    fireEvent.click(screen.getByRole("button", { name: "选择对话项目：未分类" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /新增项目/ }));
    fireEvent.change(screen.getByLabelText("项目名称"), { target: { value: "新创建项目" } });
    fireEvent.change(screen.getByLabelText("项目说明"), { target: { value: "形成验收材料" } });
    expect((screen.getByRole("button", { name: "创建项目" }) as HTMLButtonElement).disabled).toBe(false);
    expect(addWorkspace).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "创建项目" }));
    const selector = await screen.findByRole("button", { name: "选择对话项目：未分类" });
    fireEvent.click(selector);
    expect(screen.getByRole("menuitemradio", { name: /新创建项目/ })).toBeTruthy();
    expect(invoke.mock.calls.find(([command, input]) => command === "app_server_request" && input.method === "project/create")?.[1]).toMatchObject({ method: "project/create", params: { displayName: "新创建项目", workspaceIds: [] } });
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create")).toBe(false);
  });

  test("选择项目时复用新建项目弹窗，取消后返回列表且不创建对话", async () => {
    invoke.mockResolvedValue(snapshot(conversationContext()));
    function Harness() {
      const [creating, setCreating] = useState(false);
      return <>
        <AiWorkbenchPage workspace={workspace} projects={projects} files={[]} selectedFile={null} launchRequest={null} preferredConversationId={null} onMessage={() => {}} onOpenAgents={() => {}} canCreateProject projectCreationOpen={creating} onCreateProject={() => setCreating(true)} onSelectProject={() => {}} projectIndex={defaultProjectIndex}/>
        {creating ? <ProjectCreatePanel existingProjects={projects} workspaces={[]} onAddWorkspace={async () => undefined} onCancel={() => setCreating(false)} onCreated={() => setCreating(false)} onMessage={() => {}}/> : null}
      </>;
    }
    render(<Harness/>);
    fireEvent.click(await screen.findByLabelText("新建 AI 对话"));
    fireEvent.click(screen.getByRole("button", { name: "选择对话项目：未分类" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /新增项目/ }));
    const createDialog = screen.getByRole("dialog", { name: "新建项目" });
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(within(createDialog).getByText("项目说明")).toBeTruthy();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: "选择对话项目：未分类" })).toBeTruthy();
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create")).toBe(false);
  });

  async function openPicker(agents = snapshot(conversationContext()).agents, failCreate = false) {
    let state = { ...snapshot(conversationContext()), agents } as unknown as AiRuntimeSnapshot;
    invoke.mockImplementation(async (command: string, args: { request: Record<string, unknown> }) => {
      if (command === "ai_runtime_snapshot") return state;
      if (command === "ai_conversation_submit") {
        if (failCreate) throw new Error("创建失败");
        const started = startResult(args.request);
        state = { ...state, conversations: [...state.conversations, started.conversation], messages: [...state.messages, started.message], runs: [...state.runs, started.run] };
        return { status: "recorded", conversation: started.conversation, run: started.run };
      }
      if (command === "ai_conversation_update") {
        const conversation = state.conversations.find((item) => item.conversationId === args.request.conversationId)!;
        conversation.agentId = args.request.agentId as string;
        return conversation;
      }
      if (command === "ai_message_submit") return {};
      throw new Error(`unexpected command ${command}`);
    });
    const view = render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[selectedFile]} selectedFile={selectedFile} launchRequest={null} preferredConversationId={null} onMessage={() => {}} onOpenAgents={() => {}} canCreateProject onCreateProject={() => {}} onSelectProject={() => {}} projectIndex={defaultProjectIndex}/>);
    // 等待初始会话和模型目录完成加载，避免迟到的初始化覆盖刚打开的草稿。
    await waitFor(() => expect(document.querySelectorAll(".ai-conversation-item")).toHaveLength(state.conversations.length));
    await waitFor(() => expect(screen.getByRole("button", { name: /^选择模型：/ }).hasAttribute("disabled")).toBe(false));
    fireEvent.click(screen.getByLabelText("新建 AI 对话"));
    await screen.findByRole("button", { name: "选择对话项目：未分类" });
    return view;
  }

  test("顶部新建直接进入未分类草稿且发送前不创建", async () => {
    await openPicker([]);
    expect(screen.queryByRole("dialog", { name: "选择新对话的项目" })).toBeNull();
    expect(screen.getByRole("button", { name: "选择对话项目：未分类" })).toBeTruthy();
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create")).toBe(false);
  });

  test("未选择具体项目时首次发送归入未分类", async () => {
    await openPicker();
    selectDefaultStarterAgent();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "未分类问题" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1].request).toMatchObject({
      content: "未分类问题",
      context: { projectId: null, projectName: null, workspaceId: null, workspaceName: null, workspaceRoot: "" },
    });
  });

  test("没有项目和工作区时仍可以开始未分类对话", async () => {
    let state = { ...snapshot(conversationContext()), conversations: [], messages: [], runs: [] } as unknown as AiRuntimeSnapshot;
    invoke.mockImplementation(async (command: string, args?: { request: Record<string, unknown> }) => {
      if (command === "ai_runtime_snapshot") return state;
      if (command === "ai_conversation_submit") {
        const started = startResult(args!.request);
        state = { ...state, conversations: [started.conversation], messages: [started.message], runs: [started.run] };
        return { status: "recorded", conversation: started.conversation, run: started.run };
      }
      if (command === "ai_message_submit") return {};
      throw new Error(`unexpected command ${command}`);
    });

    render(<AiWorkbenchPage workspace={null} projects={[]} files={[]} selectedFile={null} launchRequest={null} preferredConversationId={null}
      onMessage={() => {}} onOpenAgents={() => {}} canCreateProject onCreateProject={() => {}} onSelectProject={() => {}} projectIndex={new Map()}/>);

    await screen.findByRole("heading", { name: "Hi，今天有什么安排？" });
    expect(await screen.findByRole("button", { name: "选择对话项目：未分类" })).toBeTruthy();
    selectDefaultStarterAgent();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "直接讨论这个问题" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1].request).toMatchObject({
      content: "直接讨论这个问题",
      context: { projectId: null, projectName: null, workspaceId: null, workspaceName: null, workspaceRoot: "" },
    });
  });

  test("选择另一个项目只建立草稿，首次发送保留正确归属", async () => {
    await openPicker();
    fireEvent.click(screen.getByRole("button", { name: "选择对话项目：未分类" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /第二项目/ }));
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create" || command === "ai_conversation_submit")).toBe(false);
    selectDefaultStarterAgent();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "整理项目任务" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    expect(invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1].request).toMatchObject({ agentId: "agent-1", content: "整理项目任务", context: { projectId: "proj-2", workspaceId: "ws-2", workspaceRoot: "/tmp/ws-2", localFileId: null, relativePath: null } });
  });

  test("普通新建不继承文件浏览器当前选中项", async () => {
    await openPicker();
    fireEvent.click(screen.getByRole("button", { name: "选择对话项目：未分类" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /主项目/ }));
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create" || command === "ai_conversation_submit")).toBe(false);
    selectDefaultStarterAgent();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "普通项目对话" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    const request = invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1].request;
    expect(request).toMatchObject({ title: "普通项目对话", conversationType: "direct", context: { localFileId: null, relativePath: null, referencedMaterials: [] } });
  });

  test("不发送不创建；首次发送失败仍保留未分类草稿", async () => {
    await openPicker(undefined, true);
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create")).toBe(false);
    selectDefaultStarterAgent();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "保留这段草稿" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(screen.getByLabelText("描述要处理的任务")).toHaveProperty("value", "保留这段草稿"));
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create")).toBe(false);
  });

});



test("没有可用模型时模型下拉说明原因，不只剩一个分组标题", async () => {
  modelApi.models = [];
  const state = { ...snapshot(conversationContext()), localAgentsEnabled: false, models: [], messages: [existingMessage] } as unknown as AiRuntimeSnapshot;
  invoke.mockImplementation(async (command: string) => {
    if (command === "ai_runtime_snapshot") return state;
    if (command === "ai_runtime_activity_snapshot") return { stateRevision: 0, conversations: state.conversations, messages: state.messages, runs: [], permissionRequests: [] };
    if (command === "app_server_request") return { installations: [], skills: [] };
    return {};
  });
  render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[]} selectedFile={null} launchRequest={null} preferredConversationId={null}
    onMessage={() => {}} onOpenAgents={() => {}} canCreateProject onCreateProject={() => {}} onSelectProject={() => {}} projectIndex={defaultProjectIndex}/>);
  const picker = await screen.findByRole("button", { name: /^选择模型：/ });
  await waitFor(() => expect(picker).toHaveProperty("disabled", false));
  fireEvent.click(picker);
  const group = screen.getByRole("group", { name: "模型" });
  expect(within(group).getByText("暂无可用模型")).toBeTruthy();
  expect(within(group).queryByRole("menuitemradio")).toBeNull();
});



test.each([false, true])("空对话未选项目也能选择模型，选择结果保留至首次发送（本地智能体 %s）", async (localAgentsEnabled) => {
  let state = { ...snapshot(conversationContext()), localAgentsEnabled, runtimes: [], conversations: [] } as unknown as AiRuntimeSnapshot;
  invoke.mockImplementation(async (command: string, args?: { request: Record<string, unknown> }) => {
    if (command === "ai_runtime_snapshot") return state;
    if (command === "ai_conversation_submit") {
      const started = startResult(args!.request);
      state = { ...state, conversations: [started.conversation], messages: [started.message], runs: [started.run] };
      return { status: "recorded", conversation: started.conversation, run: started.run };
    }
    if (command === "ai_message_submit") return {};
    throw new Error(`unexpected command ${command}`);
  });
  render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[]} selectedFile={null} launchRequest={null} preferredConversationId={null}
    onMessage={() => {}} onOpenAgents={() => {}} canCreateProject onCreateProject={() => {}} onSelectProject={() => {}} projectIndex={defaultProjectIndex}/>);
  const picker = await screen.findByRole("button", { name: /^选择(?:智能体或)?模型：/ });
  await waitFor(() => expect(picker).toHaveProperty("disabled", false));
  fireEvent.click(picker);
  if (!localAgentsEnabled) expect(screen.queryByRole("group", { name: "智能体" })).toBeNull();
  const model = screen.getByRole("menuitemradio", { name: /Qwen3.8 27B/ });
  expect(model).toHaveProperty("disabled", false);
  fireEvent.click(model);
  expect(screen.getByRole("button", { name: "开始对话" })).toHaveProperty("disabled", true);
  expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_create" || command === "ai_message_submit")).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: /^选择(?:智能体或)?模型：Qwen3.8/ }));
  fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.6 35B/ }));
  fireEvent.click(screen.getByLabelText("新建 AI 对话"));
  fireEvent.click(screen.getByRole("button", { name: "选择对话项目：未分类" }));
  fireEvent.click(screen.getByRole("menuitemradio", { name: /主项目/ }));
  expect(screen.getByRole("button", { name: /^选择(?:智能体或)?模型：Qwen3.6 35B/ })).toBeTruthy();
  fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "整理项目材料" } });
  fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_conversation_submit", expect.objectContaining({ request: expect.objectContaining({
    title: "整理项目材料", executionTarget: "model", modelOverride: "Qwen3.6-35B-A3B-NVFP4",
  }) })));
});


test.each([false, true])("对话输入 Enter 发送，Shift+Enter 和输入法确认不发送（已有消息 %s）", async (hasMessages) => {
  await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "主项目" }), {
    projectIndex: defaultProjectIndex, messages: hasMessages ? [existingMessage] : [],
  });
  const input = screen.getByLabelText(hasMessages ? "发送给 AI" : "描述要处理的任务");
  fireEvent.change(input, { target: { value: "检查键盘发送" } });
  expect(fireEvent.keyDown(input, { key: "Enter", shiftKey: true })).toBe(true);
  expect(fireEvent.keyDown(input, { key: "Enter", isComposing: true })).toBe(true);
  expect(fireEvent.keyDown(input, { key: "Enter", keyCode: 229 })).toBe(true);
  expect(invoke.mock.calls.some(([command]) => ["ai_message_send", "ai_message_submit"].includes(command))).toBe(false);
  expect(fireEvent.keyDown(input, { key: "Enter" })).toBe(false);
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_message_submit", expect.objectContaining({ request: expect.objectContaining({
    conversationId: "conv-1", content: "检查键盘发送",
  }) })));
});

/*
 * 提交中的消息必须当场可见，失败也不能把它藏起来。
 *
 * 线上出过的样子：两次发送都在启动运行时失败，界面什么也没变，第三次成功之后
 * 三条一模一样的内容一起冒出来——前两条其实早写进了服务端，只是失败分支没有重取快照，
 * 而退回输入框的草稿又引着用户照原样再发一遍。
 */
describe("提交中的用户消息", () => {
  const conversationSnapshot = (messages: AiRuntimeSnapshot["messages"], runs: AiRuntimeSnapshot["runs"] = []) => ({
    ...snapshot(conversationContext()),
    messages,
    runs,
  });
  // 只数对话正文里的用户消息气泡：左侧会话列表的摘要也会带上同一段文本。
  const userMessageCount = (text: string) => [...document.querySelectorAll("article.ai-message.user")]
    .filter((node) => node.textContent?.includes(text)).length;

  test("按下发送就能看见自己发的内容，不等服务端回话", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    let releaseSend = () => { /* 由测试主体放行 */ };
    const sendGate = new Promise<void>((resolve) => { releaseSend = resolve; });
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return conversationSnapshot([existingMessage]);
      if (command === "ai_message_submit") { await sendGate; return {}; }
      throw new Error(`unexpected command ${command}`);
    });

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "刚按下发送的内容" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(userMessageCount("刚按下发送的内容")).toBe(1));
    const feedback = screen.getByLabelText("Qwen3.8 27B NVFP4 正在处理…");
    expect(within(feedback).getByRole("button", { name: "展开执行过程：正在处理…" })).toBeTruthy();
    expect(within(feedback).getByRole("status", { name: "回复准备中" }).querySelectorAll(".ai-pending-reply-dots i")).toHaveLength(3);
    expect(within(feedback).queryByText("已收到，正在准备回复")).toBeNull();
    expect(screen.queryByLabelText("AI 正在处理")).toBeNull();
    releaseSend();
  });

  test("宿主受理后输入框立即可用，运行出现前的新内容进入排队且不能补充", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    let submissions: NonNullable<AiRuntimeSnapshot["messageSubmissions"]> = [];
    invoke.mockImplementation(async (command: string, args?: { request?: { clientMessageId?: string } }) => {
      if (command === "ai_runtime_snapshot") return { ...conversationSnapshot([existingMessage]), messageSubmissions: submissions };
      if (command === "ai_message_submit") {
        submissions = [{ clientMessageId: args!.request!.clientMessageId!, conversationId: "conv-1", status: "preparing", reason: null, acceptedAt: 1 }];
        return { status: "accepted" };
      }
      throw new Error(`unexpected command ${command}`);
    });

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "第一条" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
    await waitFor(() => expect(userMessageCount("第一条")).toBe(1));

    const input = screen.getByLabelText("发送给 AI") as HTMLTextAreaElement;
    expect(input.disabled).toBe(false);
    fireEvent.change(input, { target: { value: "第二条" } });
    const queueButton = await screen.findByRole("button", { name: "排队发送" });
    expect((queueButton as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(queueButton);
    const queue = await screen.findByRole("region", { name: "待发送的后续指令" });
    expect(within(queue).getByText("第二条")).toBeTruthy();
    const steer = within(queue).getByRole("button", { name: "补充回复：排队 1" }) as HTMLButtonElement;
    expect(steer.disabled).toBe(true);
    expect(steer.title).toBe("等待当前消息开始执行");
    expect(invoke.mock.calls.filter(([command]) => command === "ai_message_submit")).toHaveLength(1);
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_steer")).toBe(false);
  });

  test("准备期轮询排队后仍只读本机状态，显式刷新保留目录核对", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    const originalInvoke = invoke.getMockImplementation()!;
    const originalInterval = window.setInterval.bind(window);
    let poll: (() => void) | undefined;
    const interval = vi.spyOn(window, "setInterval").mockImplementation((handler, delay, ...args) => {
      if (delay === 600) {
        poll = handler as () => void;
        return 1 as unknown as ReturnType<typeof window.setInterval>;
      }
      return originalInterval(handler, delay, ...args) as unknown as ReturnType<typeof window.setInterval>;
    });
    const requests: Array<{ args: unknown; resolve: (snapshot: AiRuntimeSnapshot) => void }> = [];
    let submissions: NonNullable<AiRuntimeSnapshot["messageSubmissions"]> = [];
    invoke.mockImplementation(async (command: string, args?: { request?: { clientMessageId?: string } }) => {
      if (command === "ai_runtime_snapshot") return new Promise<AiRuntimeSnapshot>((resolve) => { requests.push({ args, resolve }); });
      if (command === "ai_message_submit") {
        submissions = [{ clientMessageId: args!.request!.clientMessageId!, conversationId: "conv-1", status: "preparing", reason: null, acceptedAt: 1 }];
        return { status: "accepted" };
      }
      return originalInvoke(command, args);
    });
    const prepared = () => ({ ...conversationSnapshot([existingMessage]), messageSubmissions: submissions }) as AiRuntimeSnapshot;
    try {
      fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "验证准备期刷新" } });
      fireEvent.click(screen.getByRole("button", { name: "发送" }));
      await waitFor(() => expect(requests).toHaveLength(1));
      await waitFor(() => expect(poll).toBeDefined());
      expect(requests[0].args).toEqual({ refreshNativeCatalog: false });
      act(() => poll!());
      await act(async () => { requests[0].resolve(prepared()); });
      await waitFor(() => expect(requests).toHaveLength(2));
      expect(requests[1].args).toEqual({ refreshNativeCatalog: false });
      act(() => window.dispatchEvent(new Event("cilai-ai-runtime-changed")));
      act(() => poll!());
      await act(async () => { requests[1].resolve(prepared()); });
      await waitFor(() => expect(requests).toHaveLength(3));
      expect(requests[2].args).toBeUndefined();
      await act(async () => { requests[2].resolve(prepared()); });
    } finally {
      interval.mockRestore();
    }
  });

  test("对话空闲但队列里还有待派发内容时，新输入排在其后不抢先发送", async () => {
    sessionApi.snapshot.values.queuedFollowUps = [{ queueId: "queued-ahead", queuedAt: 1, conversationId: "conv-1", content: "先排的内容", attachments: [], applicationContext: null, modelOverride: "Qwen3.8-27B-NVFP4", reasoningEffort: null, executionTarget: "model", permissionMode: "auto-review", status: "sending" }];
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "后来的内容" } });
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
    const queue = await screen.findByRole("region", { name: "待发送的后续指令" });
    expect(within(queue).getAllByRole("article").map((item) => item.querySelector("p")?.textContent)).toEqual(["先排的内容", "后来的内容"]);
    expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
  });

  test("受理后落库前失败时给出原因，内容不留在正文且可恢复为草稿", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    let submissions: NonNullable<AiRuntimeSnapshot["messageSubmissions"]> = [];
    invoke.mockImplementation(async (command: string, args?: { request?: { clientMessageId?: string }; clientMessageId?: string }) => {
      if (command === "ai_runtime_snapshot") return { ...conversationSnapshot([existingMessage]), messageSubmissions: submissions };
      if (command === "ai_message_submit") {
        submissions = [{ clientMessageId: args!.request!.clientMessageId!, conversationId: "conv-1", status: "failed", reason: "企业登录状态已失效，请重新登录。", acceptedAt: 1 }];
        return { status: "accepted" };
      }
      if (command === "ai_message_submission_dismiss") { submissions = []; return null; }
      throw new Error(`unexpected command ${command}`);
    });

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "需要恢复的内容" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(document.querySelector('.ai-recovered-send[data-failed="true"]')?.textContent).toContain("企业登录状态已失效"));
    expect(userMessageCount("需要恢复的内容")).toBe(0);
    const strip = document.querySelector<HTMLElement>('.ai-recovered-send[data-failed="true"]')!;
    fireEvent.click(within(strip).getByRole("button", { name: "恢复草稿" }));
    await waitFor(() => expect((screen.getByLabelText("发送给 AI") as HTMLTextAreaElement).value).toBe("需要恢复的内容"));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submission_dismiss")).toBe(true));
    await waitFor(() => expect(document.querySelector('.ai-recovered-send[data-failed="true"]')).toBeNull());
  });

  test("按下发送只隐藏上一轮异常卡，已经生成的未完成回复继续保留", async () => {
    const previousRun: AiRuntimeSnapshot["runs"][number] = {
      runId: "run-previous-failed", conversationId: "conv-1", agentId: "agent-1", inputMessageId: "message-existing",
      retryOfRunId: null, provider: "codex", model: "Qwen3.6-35B-A3B-NVFP4", executionTarget: "model",
      status: "failed", pid: null, startedAt: 1_700_000_000_000, completedAt: 1_700_000_005_000, output: null,
      errorReason: "credential unavailable",
      sessionId: null, skillIds: [], toolPermissions: [], permissionRequestId: null, automationRuleId: null,
      automationRuleVersion: null, sandboxMode: "read-only", events: [], artifacts: [],
      usage: { inputTokens: 11_127, cachedInputTokens: 0, outputTokens: 103 }, logTail: [],
      partialOutput: "上一轮未完成的内容",
    };
    const previousInput = { ...existingMessage, runId: previousRun.runId };
    await renderWorkbench(conversationContext(), { messages: [previousInput], runs: [previousRun] });
    let releaseSend = () => { /* 由测试主体放行 */ };
    const sendGate = new Promise<void>((resolve) => { releaseSend = resolve; });
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return conversationSnapshot([previousInput], [previousRun]);
      if (command === "ai_message_submit") { await sendGate; return {}; }
      throw new Error(`unexpected command ${command}`);
    });

    expect(screen.getByRole("region", { name: "最近一次运行异常" })).toBeTruthy();
    expect(screen.getByLabelText("未完成的 AI 消息")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "继续处理" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(screen.queryByRole("region", { name: "最近一次运行异常" })).toBeNull());
    expect(screen.getByLabelText("未完成的 AI 消息")).toBeTruthy();
    expect(screen.getByText("上一轮未完成的内容")).toBeTruthy();
    expect(screen.getByRole("button", { name: "展开执行过程：正在处理…" })).toBeTruthy();
    releaseSend();
  });

  test("按下发送后隐藏没有回复和产物的上一轮失败", async () => {
    const previousRun: AiRuntimeSnapshot["runs"][number] = {
      runId: "run-previous-empty", conversationId: "conv-1", agentId: "agent-1", inputMessageId: "message-existing",
      retryOfRunId: null, provider: "codex", model: "Qwen3.6-35B-A3B-NVFP4", executionTarget: "model",
      status: "failed", pid: null, startedAt: 1_700_000_000_000, completedAt: 1_700_000_005_000, output: null,
      errorReason: "credential unavailable", partialOutput: null,
      sessionId: null, skillIds: [], toolPermissions: [], permissionRequestId: null, automationRuleId: null,
      automationRuleVersion: null, sandboxMode: "read-only", events: [], artifacts: [],
      usage: { inputTokens: 0, cachedInputTokens: 0, outputTokens: 0 }, logTail: [],
    };
    const previousInput = { ...existingMessage, runId: previousRun.runId };
    await renderWorkbench(conversationContext(), { messages: [previousInput], runs: [previousRun] });
    let releaseSend = () => { /* 由测试主体放行 */ };
    const sendGate = new Promise<void>((resolve) => { releaseSend = resolve; });
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return conversationSnapshot([previousInput], [previousRun]);
      if (command === "ai_message_submit") { await sendGate; return {}; }
      throw new Error(`unexpected command ${command}`);
    });

    expect(screen.getByRole("region", { name: "最近一次运行异常" })).toBeTruthy();
    expect(screen.queryByLabelText("未完成的 AI 消息")).toBeNull();
    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "重新处理" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(screen.queryByRole("region", { name: "最近一次运行异常" })).toBeNull());
    expect(screen.queryByLabelText("未完成的 AI 消息")).toBeNull();
    releaseSend();
  });

  test("新对话首条消息提交后立即进入对话反馈，不等待会话与运行建立", async () => {
    let state = { ...snapshot(conversationContext()), conversations: [], messages: [], runs: [] } as unknown as AiRuntimeSnapshot;
    let releaseStart = () => { /* 由测试主体放行 */ };
    const startGate = new Promise<void>((resolve) => { releaseStart = resolve; });
    invoke.mockImplementation(async (command: string, args?: { request?: Record<string, unknown>; method?: string }) => {
      if (command === "ai_runtime_snapshot") return state;
      if (command === "app_server_request" && args?.method === "skill/conversationCatalog") return { skills: [] };
      if (command === "ai_conversation_submit") {
        await startGate;
        const started = startResult(args!.request!);
        started.run = { ...started.run, status: "running", partialOutput: "这是真实回复" };
        state = { ...state, conversations: [started.conversation], messages: [started.message], runs: [started.run] };
        return { status: "recorded", conversation: started.conversation, run: started.run };
      }
      throw new Error(`unexpected command ${command}`);
    });
    render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[]} selectedFile={null} launchRequest={null} preferredConversationId={null}
      onMessage={() => {}} onOpenAgents={() => {}} canCreateProject onCreateProject={() => {}} onSelectProject={() => {}} projectIndex={defaultProjectIndex}/>);

    await screen.findByRole("heading", { name: "Hi，今天有什么安排？" });
    selectDefaultStarterAgent();
    fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "立即显示首条消息" } });
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));

    await waitFor(() => expect(userMessageCount("立即显示首条消息")).toBe(1));
    const feedback = screen.getByLabelText("Qwen3.8 27B NVFP4 正在处理…");
    expect(within(feedback).getByRole("button", { name: "展开执行过程：正在处理…" })).toBeTruthy();
    expect(within(feedback).getByRole("status", { name: "回复准备中" }).querySelectorAll(".ai-pending-reply-dots i")).toHaveLength(3);
    expect(within(feedback).queryByText("已收到，正在准备回复")).toBeNull();
    expect(screen.queryByLabelText("AI 正在处理")).toBeNull();
    releaseStart();
    await waitFor(() => expect(screen.getByLabelText("正在输出的回复").textContent).toContain("这是真实回复"));
    expect(screen.queryByLabelText("回复准备中")).toBeNull();
    expect(userMessageCount("立即显示首条消息")).toBe(1);
  });

  test("运行启动失败但服务端已记下时，消息照常出现且草稿不退回输入框", async () => {
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { messages: [existingMessage], onMessage });
    let recorded = false;
    let clientMessageId = "";
    const persisted = () => ({
      messageId: clientMessageId,
      conversationId: "conv-1",
      role: "user" as const,
      content: "启动失败的这条",
      attachments: [],
      runId: "run-start-failed",
      createdAt: Date.now(),
    });
    invoke.mockImplementation(async (command: string, args) => {
      if (command === "ai_runtime_snapshot") return conversationSnapshot(recorded ? [existingMessage, persisted()] : [existingMessage]);
      if (command === "ai_message_submit") {
        // 服务端先落库再启动运行，启动失败时消息留在库里，只有运行被标成失败。
        clientMessageId = args.request.clientMessageId;
        recorded = true;
        throw new Error("enterprise_tool_snapshot_rejected:application_mcp_tool_binding_missing");
      }
      throw new Error(`unexpected command ${command}`);
    });

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "启动失败的这条" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    // 同一句话只该出现一次：本地那条在快照记下它之后就撤掉，不能和服务端那条并排显示。
    await waitFor(() => expect(userMessageCount("启动失败的这条")).toBe(1));
    expect((screen.getByLabelText("发送给 AI") as HTMLTextAreaElement).value).toBe("");
    expect(onMessage).toHaveBeenCalledWith(expect.stringContaining("enterprise_tool_snapshot_rejected"), "error");
  });

  test("服务端没记下这条时，内容退回输入框且不留在对话里", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return conversationSnapshot([existingMessage]);
      if (command === "ai_message_submit") throw new Error("conversation_run_already_active");
      throw new Error(`unexpected command ${command}`);
    });

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "没能提交的这条" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect((screen.getByLabelText("发送给 AI") as HTMLTextAreaElement).value).toBe("没能提交的这条"));
    expect(userMessageCount("没能提交的这条")).toBe(0);
  });


  test.each(["等待时新输入的重要补充", " \n  "])("发送失败保留新草稿 %j，原消息只在手动重试后交给宿主", async (nextDraft) => {
    const onMessage = vi.fn();
    await renderWorkbench(conversationContext(), { messages: [existingMessage], onMessage });
    const base = invoke.getMockImplementation()!;
    let rejectSend!: (reason: Error) => void;
    const sendGate = new Promise((_, reject) => { rejectSend = reject; });

    let sends = 0;
    invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "ai_message_submit") { sends += 1; return sendGate; }
      return base(command, args);
    });

    setComposerText("发送给 AI", "第一条待发送消息");
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(sends).toBe(1));
    const input = setComposerText("发送给 AI", nextDraft) as HTMLTextAreaElement;
    expect(input.disabled).toBe(false);
    fireEvent.keyDown(input, { key: "Enter" });
    expect(sends).toBe(1);
    await act(async () => rejectSend(new Error("enterprise_session_unavailable")));

    const queue = await screen.findByRole("region", { name: "待发送的后续指令" });
    expect(within(queue).getByText("第一条待发送消息")).toBeTruthy();
    expect(within(queue).getByText("发送失败")).toBeTruthy();
    expect(input.value).toBe(nextDraft);
    expect(userMessageCount("第一条待发送消息")).toBe(0);
    expect(sends).toBe(1);
    openQueuedEdit(queue);
    expect(onMessage).toHaveBeenCalledWith("请先发送或清空当前草稿。", "error");
    expect(input.value).toBe(nextDraft);
    expect(within(queue).getByText("第一条待发送消息")).toBeTruthy();

    fireEvent.click(within(queue).getByRole("button", { name: "重试排队消息 1" }));
    await act(() => flushConversationSessions());
    expect(sessionApi.snapshot.values.queuedFollowUps).toEqual([expect.objectContaining({ conversationId: "conv-1", content: "第一条待发送消息", status: "queued" })]);
    expect(input.value).toBe(nextDraft);
    expect(sends).toBe(1);

  });


  test.each(["成功", "失败"] as const)("发送%s后保留后来粘贴的附件，失败重试沿用原附件和技能", async (outcome) => {
    await renderWorkbench(conversationContext(), {
      messages: [existingMessage],
      conversationSkills: [{ skillId: "skill-weekly", name: "周报整理", description: "整理项目周报", source: "user", automatic: true, stateRevision: 1 }],
    });
    const base = invoke.getMockImplementation()!;
    let resolveSend!: (run: unknown) => void;
    let rejectSend!: (reason: Error) => void;
    const sendGate = new Promise((resolve, reject) => { resolveSend = resolve; rejectSend = reject; });
    let sends = 0;
    invoke.mockImplementation(async (command: string, args?: { request: Record<string, unknown> }) => {
      if (command === "ai_attachment_create") return { ...await base(command, args), attachmentId: args!.request.fileName };
      if (command === "ai_message_submit") return ++sends === 1 ? sendGate
        : { ...startResult(args!.request).run, conversationId: "conv-1", status: "running" };
      return base(command, args);
    });
    setComposerText("发送给 AI", "/周报");
    fireEvent.click(await screen.findByRole("button", { name: /^\/周报整理/ }));
    setComposerText("发送给 AI", "整理第一张图片");
    const composer = screen.getByLabelText("发送给 AI").closest("form")!;
    fireEvent.change(document.querySelector<HTMLInputElement>("input.ai-media-input")!, {
      target: { files: [new File(["first"], "first.png", { type: "image/png" })] },
    });
    await within(composer).findByRole("button", { name: "查看图片 first.png" });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(sends).toBe(1));
    const firstRequest = invoke.mock.calls.find(([command]) => command === "ai_message_submit")![1].request;
    expect(firstRequest).toMatchObject({ content: "整理第一张图片", attachmentIds: ["first.png"], selectedSkillId: "skill-weekly" });

    const input = setComposerText("发送给 AI", "下一张图片另作说明");
    fireEvent.paste(input, { clipboardData: { files: [new File(["next"], "next.png", { type: "image/png" })] } });
    await within(composer).findByRole("button", { name: "查看图片 next.png" });
    await act(async () => {
      if (outcome === "成功") resolveSend({ ...startResult(firstRequest).run, conversationId: "conv-1", status: "running" });
      else rejectSend(new Error("enterprise_session_unavailable"));
    });
    const draftAttachments = within(composer).getByLabelText("待发送附件");
    expect(within(draftAttachments).queryByRole("button", { name: "查看图片 first.png" })).toBeNull();
    expect(within(draftAttachments).getByRole("button", { name: "查看图片 next.png" })).toBeTruthy();
    const retainedInput = screen.getByLabelText("发送给 AI");
    expect(retainedInput instanceof HTMLTextAreaElement ? retainedInput.value : retainedInput.textContent).toBe("下一张图片另作说明");

    if (outcome === "失败") {
      const queue = screen.getByRole("region", { name: "待发送的后续指令" });
      expect(within(queue).getByRole("button", { name: "查看图片 first.png" })).toBeTruthy();
      fireEvent.click(within(queue).getByRole("button", { name: "重试排队消息 1" }));
      await act(() => flushConversationSessions());
      expect(sessionApi.snapshot.values.queuedFollowUps).toEqual([expect.objectContaining({
        content: firstRequest.content, selectedSkillId: firstRequest.selectedSkillId, status: "queued",
        attachments: [expect.objectContaining({ attachment: expect.objectContaining({ attachmentId: "first.png" }) })],
        modelOverride: firstRequest.modelOverride, reasoningEffort: firstRequest.reasoningEffort, permissionMode: firstRequest.permissionMode,
      })]);
      expect(sends).toBe(1);
      expect(within(composer).getByRole("button", { name: "查看图片 next.png" })).toBeTruthy();
    }
  });

  test("失败的斜杠正文重试时完整保留，不读取企业应用范围", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    const base = invoke.getMockImplementation()!;
    let rejectSend!: (reason: Error) => void;
    const sendGate = new Promise((_, reject) => { rejectSend = reject; });
    let sends = 0;
    invoke.mockImplementation(async (command: string, args?: { request: Record<string, unknown> }) => {
      if (command === "ai_runtime_snapshot") return {
        ...await base(command, args),
        runs: [{ ...startResult({ agentId: "agent-1" }).run, conversationId: "conv-other", status: "running" }],
      };
      if (command === "ai_message_submit") return ++sends === 1 ? sendGate
        : { ...startResult(args!.request).run, conversationId: "conv-1", status: "running" };
      return base(command, args);
    });
    setComposerText("发送给 AI", "/客户管理 读取当前客户");
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(sends).toBe(1));
    setComposerText("发送给 AI", "新的独立草稿");
    await act(async () => rejectSend(new Error("enterprise_session_unavailable")));
    const queue = await screen.findByRole("region", { name: "待发送的后续指令" });
    expect(within(queue).getByText("/客户管理 读取当前客户")).toBeTruthy();
    fireEvent.click(within(queue).getByRole("button", { name: "重试排队消息 1" }));
    await act(() => flushConversationSessions());
    expect(sessionApi.snapshot.values.queuedFollowUps).toEqual([expect.objectContaining({
      content: "/客户管理 读取当前客户", executionTarget: "model", status: "queued",
      applicationContext: null,
    })]);
    expect(sends).toBe(1);
    expect(invoke.mock.calls.some(([command]) => command === "application_catalog_list")).toBe(false);
    expect((screen.getByLabelText("发送给 AI") as HTMLTextAreaElement).value).toBe("新的独立草稿");
  });

  test("发送等待期间可新建和归档其他会话，但不能归档正在提交的会话", async () => {
    const context = conversationContext();
    const other = { ...snapshot(context).conversations[0], conversationId: "conv-other", title: "另一条独立会话" };
    await renderWorkbench(context, { conversations: [...snapshot(context).conversations, other], messages: [existingMessage] });
    const base = invoke.getMockImplementation()!;
    let resolveSend!: (run: unknown) => void;
    const sendGate = new Promise((resolve) => { resolveSend = resolve; });
    invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "ai_message_submit") return sendGate;
      if (command === "ai_conversation_set_archived") return {
        stateRevision: 1, conversations: [{ ...other, archived: true }], changedConversationIds: [other.conversationId], failures: [],
      };
      return base(command, args);
    });
    setComposerText("发送给 AI", "当前会话提交内容");
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));

    expect((screen.getByLabelText("新建 AI 对话") as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByRole("button", { name: "归档会话：已有一条消息" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "归档会话：另一条独立会话" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("ai_conversation_set_archived", {
      request: { archived: true, conversationIds: ["conv-other"] },
    }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "归档会话：另一条独立会话" })).toBeNull());
    fireEvent.click(screen.getByLabelText("新建 AI 对话"));
    setComposerText("描述要处理的任务", "独立的新草稿");
    await act(async () => resolveSend({ status: "recorded", run: { ...startResult({}).run, conversationId: "conv-1", status: "running" } }));
    expect((screen.getByLabelText("描述要处理的任务") as HTMLTextAreaElement).value).toBe("独立的新草稿");
  });

  test("不同会话可分别提交，先完成的请求不会解锁或切走仍在提交的会话", async () => {
    const context = conversationContext();
    const other = { ...snapshot(context).conversations[0], conversationId: "conv-other", title: "另一条独立会话" };
    await renderWorkbench(context, {
      conversations: [...snapshot(context).conversations, other],
      messages: [existingMessage, { ...existingMessage, messageId: "message-other", conversationId: "conv-other" }],
    });
    const base = invoke.getMockImplementation()!;
    const resolvers = new Map<string, (run: unknown) => void>();
    invoke.mockImplementation((command: string, args?: { request: { conversationId: string } }) => {
      if (command === "ai_message_submit") return new Promise((resolve) => resolvers.set(args!.request.conversationId, resolve));
      return base(command, args);
    });
    const openOther = () => fireEvent.click(screen.getByRole("button", { name: "归档会话：另一条独立会话" })
      .closest(".ai-conversation-item")!.querySelector<HTMLButtonElement>(".ai-conversation-open")!);
    setComposerText("发送给 AI", "会话一的请求");
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(resolvers.has("conv-1")).toBe(true));
    openOther();
    setComposerText("发送给 AI", "会话二的请求");
    expect((screen.getByRole("button", { name: "发送" }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await waitFor(() => expect(resolvers.has("conv-other")).toBe(true));
    setComposerText("发送给 AI", "会话二的下一条草稿");

    await act(async () => resolvers.get("conv-1")!({ status: "recorded", run: {
      ...startResult({}).run, runId: "run-first", conversationId: "conv-1", status: "running",
    } }));
    expect((screen.getByLabelText("发送给 AI") as HTMLTextAreaElement).value).toBe("会话二的下一条草稿");
    expect((screen.getByRole("button", { name: "发送" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole("button", { name: "归档会话：另一条独立会话" })
      .closest(".ai-conversation-item")!.querySelector(".ai-conversation-open")!.getAttribute("aria-current")).toBe("true");
    await act(async () => resolvers.get("conv-other")!({ status: "recorded", run: {
      ...startResult({}).run, runId: "run-second", conversationId: "conv-other", status: "running",
    } }));
    expect((screen.getByRole("button", { name: "排队发送" }) as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByLabelText("发送给 AI") as HTMLTextAreaElement).value).toBe("会话二的下一条草稿");
    expect(invoke.mock.calls.filter(([command]) => command === "ai_message_submit")).toHaveLength(2);
  });

  test.each(["全局", "项目"] as const)("首条消息等待时从%s入口新建，旧回执不会抢走新草稿", async (entry) => {
    await renderWorkbench(conversationContext({ projectId: "proj-1", projectName: "主项目" }));
    const base = invoke.getMockImplementation()!;
    let resolveStart!: () => void;
    const startGate = new Promise<void>((resolve) => { resolveStart = resolve; });
    invoke.mockImplementation(async (command: string, args?: { request: Record<string, unknown> }) => {
      if (command === "ai_conversation_submit") {
        await startGate;
        const started = startResult(args!.request);
        return { status: "recorded", conversation: started.conversation, run: { ...started.run, status: "running" } };
      }
      return base(command, args);
    });
    const create = () => fireEvent.click(screen.getByLabelText(entry === "全局" ? "新建 AI 对话" : "在 主项目 中新建对话"));
    create();
    selectDefaultStarterAgent();
    setComposerText("描述要处理的任务", "第一条新对话消息");
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
    expect(userMessageCount("第一条新对话消息")).toBe(1);

    create();
    const input = setComposerText("描述要处理的任务", "第二条独立草稿") as HTMLTextAreaElement;
    expect(input.disabled).toBe(false);
    expect(userMessageCount("第一条新对话消息")).toBe(0);
    expect((screen.getByRole("button", { name: "开始对话" }) as HTMLButtonElement).disabled).toBe(false);
    await act(async () => resolveStart());
    expect((screen.getByLabelText("描述要处理的任务") as HTMLTextAreaElement).value).toBe("第二条独立草稿");
    expect(userMessageCount("第一条新对话消息")).toBe(0);
    expect(screen.getByRole("button", { name: "开始对话" })).toBeTruthy();
    expect(invoke.mock.calls.filter(([command]) => command === "ai_conversation_submit")).toHaveLength(1);
  });

  test("首条受理后立即解锁并排队，后台失败保留原文供恢复", async () => {
    const context = conversationContext({ projectId: "proj-1", projectName: "主项目" });
    await renderWorkbench(context);
    const base = invoke.getMockImplementation()!;
    let current = snapshot(context) as AiRuntimeSnapshot;
    invoke.mockImplementation(async (command: string, args?: { request: Record<string, unknown> }) => {
      if (command === "ai_conversation_submit") {
        const started = startResult(args!.request);
        current = { ...current, conversations: [started.conversation], messages: [], runs: [], messageSubmissions: [{
          clientMessageId: args!.request.clientMessageId as string, conversationId: started.conversation.conversationId,
          status: "preparing", reason: null, acceptedAt: Date.now(),
        }] };
        return { status: "accepted", conversation: started.conversation };
      }
      if (command === "ai_runtime_snapshot") return current;
      if (command === "ai_message_submission_dismiss") return null;
      return base(command, args);
    });
    fireEvent.click(screen.getByLabelText("新建 AI 对话"));
    selectDefaultStarterAgent();
    fireEvent.click(screen.getByRole("button", { name: "选择审批模式：请求批准" }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /^请求批准/ }));
    setComposerText("描述要处理的任务", "首条待准备的原文");
    fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
    await waitFor(() => expect((screen.getByLabelText("发送给 AI") as HTMLTextAreaElement).disabled).toBe(false));
    expect(userMessageCount("首条待准备的原文")).toBe(1);
    expect(screen.getByRole("button", { name: "选择审批模式：请求批准" })).toBeTruthy();
    setComposerText("发送给 AI", "首条准备期间的后续要求");
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
    await screen.findByRole("region", { name: "待发送的后续指令" });
    await waitFor(() => expect(sessionApi.snapshot.values.queuedFollowUps).toEqual([
      expect.objectContaining({ permissionMode: "request", modelOverride: "Qwen3.8-27B-NVFP4" }),
    ]));
    expect(invoke.mock.calls.filter(([command]) => command === "ai_conversation_submit")).toHaveLength(1);
    expect(invoke.mock.calls.filter(([command]) => command === "ai_message_submit")).toHaveLength(0);
    current = { ...current, messageSubmissions: current.messageSubmissions!.map((item) => ({ ...item, status: "failed", reason: "enterprise_session_unavailable" })) };
    fireEvent.click(await screen.findByRole("button", { name: "恢复草稿" }, { timeout: 3000 }));
    await waitFor(() => expect((screen.getByLabelText("描述要处理的任务") as HTMLTextAreaElement).value).toBe("首条待准备的原文"));
  });

  test("发送后的刷新不再顺带拉模型目录", async () => {
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    invoke.mockClear();
    invoke.mockImplementation(async (command: string) => {
      if (command === "ai_runtime_snapshot") return conversationSnapshot([existingMessage]);
      if (command === "ai_message_submit") return {};
      throw new Error(`unexpected command ${command}`);
    });

    fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "检查刷新参数" } });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));

    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_runtime_snapshot")).toBe(true));
    for (const [, args] of invoke.mock.calls.filter(([command]) => command === "ai_runtime_snapshot")) {
      expect(args ?? {}).not.toHaveProperty("refreshModels");
    }
  });
});

test("对话页卸载重挂后保留新建草稿和审批选择", async () => {
  await renderWorkbench();
  cleanup();
  const onMessage = vi.fn();
  const page = <AiWorkbenchPageImplementation workspace={workspace} hostPlatform="macos" projects={projects} files={[selectedFile]} selectedFile={null} launchRequest={null} preferredConversationId={null} onMessage={onMessage} onOpenAgents={vi.fn()} canCreateProject onCreateProject={vi.fn()} onSelectProject={vi.fn()} projectIndex={new Map()}/>;
  const shell = (show: boolean) => <ModelCatalogProvider profileId="test-profile"><WindowsSandboxProvider scopeKey="test-profile"><ConversationSessionProvider hostPlatform="macos" onMessage={onMessage}>{show ? page : <p>项目页</p>}</ConversationSessionProvider></WindowsSandboxProvider></ModelCatalogProvider>;
  const view = render(shell(true));
  await screen.findByRole("button", { name: "新建 AI 对话" });
  await act(async () => { await Promise.resolve(); });
  fireEvent.click(screen.getByRole("button", { name: "新建 AI 对话" }));
  const input = await screen.findByRole("textbox", { name: "描述要处理的任务" });
  fireEvent.change(input, { target: { value: "先保留这份草稿" } });
  fireEvent.click(screen.getByRole("button", { name: /^选择模型：/ }));
  fireEvent.click(screen.getByRole("menuitemradio", { name: /Qwen3.8 27B NVFP4.*上下文 512K/ }));
  fireEvent.click(screen.getByRole("button", { name: "选择审批模式：请求批准" }));
  fireEvent.click(screen.getByRole("menuitemradio", { name: /^请求批准/ }));
  const permissionLabel = screen.getByRole("button", { name: /选择审批模式|选择工作区权限/ }).getAttribute("aria-label");
  view.rerender(shell(false));
  view.rerender(shell(true));
  expect(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: /正在加载模型/ })).toBeNull();
  await waitFor(() => expect((screen.getByRole("textbox", { name: "描述要处理的任务" }) as HTMLTextAreaElement).value).toBe("先保留这份草稿"));
  expect(screen.getByRole("button", { name: permissionLabel! })).toBeTruthy();
  expect(invoke.mock.calls.filter(([command]) => command === "ai_message_submit")).toHaveLength(0);
  await act(() => flushConversationSessions());
  expect(sessionApi.snapshot.values.starterPermissionMode).toBe("request");
  fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(true));
  expect(invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1]?.request.permissionMode).toBe("request");
});


test.each([false, true])("恢复旧的模型只读选择时明确要求重新选择审批模式，计划状态 %s", async (planning) => {
  sessionApi.snapshot.values = {
    selectedConversationId: "conv-1", initializedSelection: "conversation:conv-1",
    starterAgentId: "agent-1", starterModelOverride: "Qwen3.8-27B-NVFP4",
    starterPermissionMode: "read-only", starterPlanning: planning,
  };
  await renderWorkbench(conversationContext(), { messages: [existingMessage] });
  const picker = await screen.findByRole("button", { name: "选择审批模式：请选择" });
  const input = setComposerText("发送给 AI", "继续执行验收");
  expect(screen.getByRole("button", { name: "发送" })).toHaveProperty("disabled", true);
  fireEvent.keyDown(input, { key: "Enter", keyCode: 13 });
  expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
  fireEvent.click(picker);
  fireEvent.click(screen.getByRole("menuitemradio", { name: /^请求批准/ }));
  fireEvent.click(screen.getByRole("button", { name: "发送" }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
  expect(invoke.mock.calls.find(([command]) => command === "ai_message_submit")?.[1]?.request).toMatchObject({ permissionMode: "request", planning: false });
});

test("普通发送先保存恢复记录，服务接收之前原内容仍可持久恢复", async () => {
  await renderWorkbench(conversationContext(), { messages: [existingMessage] });
  let release!: () => void;
  invoke.mockImplementation(async (command: string, args) => {
    if (command === "ai_runtime_snapshot") return { ...snapshot(conversationContext()), messages: [existingMessage] };
    if (command === "ai_message_submit") {
      const journal = sessionApi.snapshot.values.pendingOutgoing as Array<{ pendingId: string; content: string; recovery: unknown }>;
      expect(journal.find((item) => item.pendingId === args.request.clientMessageId)).toMatchObject({ content: "退出时也保留", recovery: { attachments: [], agentId: "agent-1" } });
      await new Promise<void>((resolve) => { release = resolve; });
      throw new Error("发送未受理");
    }
    throw new Error(`unexpected command ${command}`);
  });
  fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "退出时也保留" } });
  fireEvent.click(screen.getByRole("button", { name: "发送" }));
  await waitFor(() => expect(release).toBeDefined());
  await act(async () => release());
  await waitFor(() => expect((screen.getByLabelText("发送给 AI") as HTMLTextAreaElement).value).toBe("退出时也保留"));
});

test("重启待恢复内容需主动恢复且保留后写草稿", async () => {
  sessionApi.snapshot.values.recoveredOutgoing = [{ pendingId: "unknown-send", draftKey: "conversation:conv-1", conversationId: "conv-1", content: "上次未确认的内容", attachments: [], createdAt: 1, recovery: { create: null, attachments: [], applicationContext: null, selectedSkillId: null, agentId: "agent-1", modelOverride: null, reasoningEffort: null, permissionMode: "read-only", file: null } }];
  const onMessage = vi.fn();
  await renderWorkbench(conversationContext(), { messages: [existingMessage], onMessage });
  expect(screen.getByText("上次发送未确认")).toBeTruthy();
  fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "后写草稿" } });
  fireEvent.click(screen.getByRole("button", { name: "恢复草稿" }));
  expect((screen.getByLabelText("发送给 AI") as HTMLTextAreaElement).value).toBe("后写草稿");
  expect(onMessage).toHaveBeenCalledWith("请先处理当前草稿，再恢复这条内容。", "error");
  fireEvent.change(screen.getByLabelText("发送给 AI"), { target: { value: "" } });
  fireEvent.click(screen.getByRole("button", { name: "恢复草稿" }));
  expect((screen.getByLabelText("发送给 AI") as HTMLTextAreaElement).value).toBe("上次未确认的内容");
  expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
});


test("材料选择按名称搜索全部文件，多选确认前不改变对话上下文", async () => {
  const files = Array.from({ length: 10_000 }, (_, index) => ({ ...selectedFile, localFileId: `material-${index}`, relativePath: `资料/第${String(index).padStart(5, "0")}份材料.md` }));
  await renderWorkbench(conversationContext({ localFileId: null, relativePath: null, fileRevision: null }), { messages: [existingMessage], files });
  fireEvent.click(screen.getByRole("button", { name: "添加上下文" }));
  fireEvent.click(screen.getByRole("menuitem", { name: /工作区文件/ }));
  const dialog = within(screen.getByRole("dialog", { name: "选择工作区文件" }));
  expect(document.querySelectorAll(".starter-file-list button")).toHaveLength(100);
  fireEvent.change(dialog.getByLabelText("搜索工作区文件"), { target: { value: "09999" } });
  fireEvent.click(dialog.getByRole("button", { name: /第09999份材料/ }));
  fireEvent.change(dialog.getByLabelText("搜索工作区文件"), { target: { value: "00000" } });
  fireEvent.click(dialog.getByRole("button", { name: /第00000份材料/ }));
  expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_update")).toBe(false);
  fireEvent.click(dialog.getByRole("button", { name: "添加" }));
  expect(screen.getByLabelText("待发送文件").querySelectorAll(".ai-document-attachment")).toHaveLength(2);
  expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_update")).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "发送" }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
  const request = invoke.mock.calls.find(([command]) => command === "ai_message_submit")![1].request;
  expect(request.referencedMaterials.map((item: { localFileId: string }) => item.localFileId)).toEqual(["material-0", "material-9999"]);
});

test("页面转交打开独立预填草稿，保留原草稿且不自动发送", async () => {
  sessionApi.snapshot.values.drafts = { "new:unassigned":"原草稿" };
  await renderWorkbench(conversationContext(), {launchRequest:{token:7701,title:"应用转交",conversationType:"direct",mode:"discuss",context:conversationContext({workspaceId:null,workspaceRoot:"",projectId:null}),startFresh:true,prefill:"页面结果与对象快照"}});
  await waitFor(() => expect(document.body.textContent).toContain("页面结果与对象快照"));
  await act(() => flushConversationSessions());
  const drafts = sessionApi.snapshot.values.drafts as Record<string,string>;
  expect(Object.values(drafts)).toContain("原草稿"); expect(Object.values(drafts)).toContain("页面结果与对象快照");
  expect(invoke.mock.calls.some(([command])=>["ai_conversation_submit","ai_message_submit"].includes(command))).toBe(false);
});

test("千条历史消息首次只渲染最近百条，加载历史保留最新内容", async () => {
  const messages = Array.from({length:1000}, (_,index) => ({...existingMessage,messageId:`history-${index}`,content:`历史内容 ${index}`,createdAt:index}));
  await renderWorkbench(conversationContext(),{messages});
  expect(document.querySelectorAll(".ai-message[data-message-id]")).toHaveLength(100);
  expect(document.querySelector('[data-message-id="history-0"]')).toBeNull();
  expect(document.querySelector('[data-message-id="history-999"]')).not.toBeNull();
  fireEvent.click(screen.getByRole("button",{name:"加载更早消息"}));
  expect(document.querySelectorAll(".ai-message[data-message-id]")).toHaveLength(200);
  expect(document.querySelector('[data-message-id="history-800"]')).not.toBeNull();
});


test("新建任务使用持续对话，描述提交一次且不另建或归档起草会话", async () => {
  let state = { ...snapshot(conversationContext()), conversations: [], messages: [], runs: [] } as unknown as AiRuntimeSnapshot;
  invoke.mockImplementation(async (command: string, args: { request: Record<string, unknown> }) => {
    if (command === "ai_runtime_snapshot") return state;
    if (command === "ai_conversation_submit") {
      const started = startResult(args.request);
      state = { ...state, conversations: [started.conversation], messages: [started.message], runs: [started.run] };
      return { status: "recorded", conversation: started.conversation, run: started.run };
    }
    throw new Error(command);
  });
  render(<AiWorkbenchPage workspace={workspace} projects={projects} files={[]} selectedFile={null}
    launchRequest={{ token: 811, title: "新建任务", conversationType: "project", mode: "discuss", context: workspaceContext(workspace, null, projects[0]), projectIds: ["proj-1"] }}
    preferredConversationId={null} onMessage={vi.fn()} onOpenAgents={vi.fn()} canCreateProject onCreateProject={vi.fn()} onSelectProject={vi.fn()} projectIndex={defaultProjectIndex}/>);
  await screen.findByRole("heading", { name: "描述你要完成的事" });
  expect(screen.queryByRole("button", { name: /选择对话项目/ })).toBeNull();
  expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_submit")).toBe(false);
  expect(screen.queryByLabelText("完成标准")).toBeNull();
  selectDefaultStarterAgent();
  fireEvent.change(screen.getByLabelText("描述要处理的任务"), { target: { value: "整理本周会议记录，给出行动清单" } });
  fireEvent.click(screen.getByRole("button", { name: "开始对话" }));
  await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === "ai_conversation_submit")).toHaveLength(1));
  expect(invoke.mock.calls.find(([command]) => command === "ai_conversation_submit")?.[1].request).toMatchObject({
    content: "整理本周会议记录，给出行动清单", conversationType: "project", context: { projectId: "proj-1", localFileId: null },
  });
  expect(invoke.mock.calls.some(([command, args]) => command === "ai_conversation_update" && args?.request?.archived === true)).toBe(false);
  expect(invoke.mock.calls.some(([, args]) => args?.method === "businessTask/create")).toBe(false);
});

describe("文件与附件随本轮消息发送", () => {
  const reference = { localFileId: "file-1", relativePath, fileRevision: 3, workspaceId: "ws-1" };
  const attachment = { attachmentId: "draft-document", fileName: "验收说明.md", mimeType: "text/markdown", mediaType: "document" as const, sizeBytes: 100, localPath: "" };

  test.each([false, true])("发送请求未返回时文件和附件已离开输入框，首次发送 %s", async (first) => {
    if (first) {
      await renderEmptyWorkbench();
      selectDefaultStarterAgent();
    } else {
      await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    }
    cleanup();
    const key = first ? "new:unassigned" : "conversation:conv-1";
    sessionApi.snapshot.values.draftMaterialGroups = { [key]: [reference] };
    sessionApi.snapshot.values.draftAttachmentGroups = { [key]: [{ attachment, previewUrl: "" }] };
    if (first) { await renderEmptyWorkbench(); selectDefaultStarterAgent(); }
    else await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    const base = invoke.getMockImplementation()!;
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    invoke.mockImplementation(async (command: string, args?: { request: Record<string, unknown> }) => {
      if (command === (first ? "ai_conversation_submit" : "ai_message_submit")) {
        await gate;
        return first ? { status: "recorded", ...startResult(args!.request) } : { ...startResult(args!.request).run, conversationId: "conv-1", status: "running" };
      }
      return base(command, args);
    });
    expect(screen.getByLabelText("待发送文件")).toBeTruthy();
    expect(screen.getByLabelText("待发送附件")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: first ? "开始对话" : "发送" }));
    expect(screen.queryByLabelText("待发送文件")).toBeNull();
    expect(screen.queryByLabelText("待发送附件")).toBeNull();
    expect(screen.getByLabelText("消息文件").textContent).toContain(fileNameFromPath(relativePath));
    expect(screen.getByLabelText("消息附件").textContent).toContain("验收说明.md");
    await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === (first ? "ai_conversation_submit" : "ai_message_submit"))).toBe(true));
    await act(async () => release());
    expect(screen.queryByLabelText("待发送文件")).toBeNull();
    expect(screen.queryByLabelText("待发送附件")).toBeNull();
  });

  test("未受理的发送恢复同轮文件和附件，已记录的发送不恢复", async () => {
    sessionApi.snapshot.values.draftMaterialGroups = { "conversation:conv-1": [reference] };
    sessionApi.snapshot.values.draftAttachmentGroups = { "conversation:conv-1": [{ attachment, previewUrl: "" }] };
    await renderWorkbench(conversationContext(), { messages: [existingMessage] });
    const base = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "ai_message_submit") throw new Error("材料暂时不可用");
      return base(command, args);
    });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await screen.findByLabelText("待发送文件");
    expect(screen.getByLabelText("待发送附件").textContent).toContain("验收说明.md");
    expect(screen.queryByLabelText("消息文件")).toBeNull();
    invoke.mockImplementation(async (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "ai_message_submit") {
        const recorded = { ...existingMessage, messageId: args!.request!.clientMessageId, content: "", attachments: [attachment], referencedMaterials: [reference] };
        invoke.mockImplementation(async (next: string, nextArgs?: unknown) => next === "ai_runtime_snapshot" ? { ...snapshot(conversationContext()), messages: [existingMessage, recorded] } : base(next, nextArgs));
        throw new Error("运行启动失败");
      }
      return base(command, args);
    });
    fireEvent.click(screen.getByRole("button", { name: "发送" }));
    await screen.findByLabelText("消息文件");
    await waitFor(() => expect(screen.queryByLabelText("待发送文件")).toBeNull());
    expect(screen.queryByLabelText("待发送附件")).toBeNull();
  });

  test("运行期间添加文件可排队，编辑队列恢复文件且下一轮不重复携带", async () => {
    const active = { ...startResult({ executionTarget: "model", modelOverride: "Qwen3.8-27B-NVFP4" }).run, conversationId: "conv-1", status: "running" as const, completedAt: null, errorReason: null };
    await renderWorkbench(conversationContext(), { messages: [existingMessage], runs: [active] });
    fireEvent.click(screen.getByRole("button", { name: "添加上下文" }));
    fireEvent.click(screen.getByRole("menuitem", { name: /工作区文件/ }));
    const picker = within(screen.getByRole("dialog", { name: "选择工作区文件" }));
    fireEvent.click(picker.getByRole("button", { name: /项目目录结构与产品分类/ }));
    fireEvent.click(picker.getByRole("button", { name: "添加" }));
    fireEvent.click(screen.getByRole("button", { name: "排队发送" }));
    expect(screen.queryByLabelText("待发送文件")).toBeNull();
    const queue = screen.getByRole("region", { name: "待发送的后续指令" });
    expect(within(queue).getByLabelText("消息文件").textContent).toContain(fileNameFromPath(relativePath));
    await act(() => flushConversationSessions());
    expect(sessionApi.snapshot.values.queuedFollowUps).toEqual([expect.objectContaining({ referencedMaterials: [reference], content: "" })]);
    openQueuedEdit(queue);
    expect(screen.getByLabelText("待发送文件")).toBeTruthy();
    expect(screen.queryByLabelText("消息文件")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: `移除文件 ${fileNameFromPath(relativePath)}` }));
    expect(screen.queryByLabelText("待发送文件")).toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "ai_conversation_update")).toBe(false);
  });
});


test("目录后台更新不闪烁模型按钮，失败保留名称并阻断发送", async () => {
  await renderWorkbench(conversationContext(), { messages: [existingMessage] });
  setComposerText("发送给 AI", "保留草稿");
  expect((screen.getByRole("button", { name: "发送" }) as HTMLButtonElement).disabled).toBe(false);
  const picker = screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" });
  const width = picker.querySelector("span")?.style.width;
  const catalog = { generation: 0, revision: 2, checkedAt: Date.now(), revalidateAfter: Date.now() + 300_000, error: null, status: { configured: true, endpoint: "https://models.example.test/v1", models: modelApi.models } };
  act(() => window.dispatchEvent(new CustomEvent("model-catalog-changed", { detail: { profileId: "test-profile", catalog } })));
  expect(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" })).toBe(picker);
  expect(picker.querySelector("span")?.style.width).toBe(width);
  expect(screen.queryByRole("button", { name: /正在加载模型/ })).toBeNull();
  act(() => window.dispatchEvent(new CustomEvent("model-catalog-changed", { detail: { profileId: "test-profile", catalog: { ...catalog, revision: 3, status: null, checkedAt: null, revalidateAfter: null, error: "model_catalog_network_failed" } } })));
  expect(screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4" })).toBe(picker);
  expect(picker.querySelector("span")?.style.width).toBe(width);
  expect(screen.getByRole("alert").textContent).toContain("模型");
  setComposerText("发送给 AI", "保留草稿");
  expect((screen.getByRole("button", { name: "发送" }) as HTMLButtonElement).disabled).toBe(true);
});

test("无模型连接时可打开设置且保留当前草稿", async () => {
  await renderWorkbench(conversationContext(), { messages: [existingMessage] });
  setComposerText("发送给 AI", "保留本地资料中的草稿");
  const catalog = { generation: 1, revision: 1, checkedAt: Date.now(), revalidateAfter: null, error: null, status: { configured: false, endpoint: null, models: [] } };
  act(() => window.dispatchEvent(new CustomEvent("model-catalog-changed", { detail: { profileId: "test-profile", catalog } })));
  const guide = await screen.findByRole("button", { name: "连接模型" });
  expect(screen.getByText("尚未连接模型，草稿和附件会保留。")).toBeTruthy();
  fireEvent.click(guide);
  expect(screen.getByLabelText("发送给 AI")).toHaveProperty("value", "保留本地资料中的草稿");
  expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(false);
});

test("同名模型按连接选择，正式发送携带所选连接且不借另一连接", async () => {
  await renderWorkbench(conversationContext(), { messages: [existingMessage] });
  const previous = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === "model_execution_preflight") {
      const selection = (args as { request: { selection: unknown } }).request.selection;
      return { snapshot: selection, snapshotDigest: "component-snapshot", historyTransfer: null };
    }
    return previous(command, args);
  });
  const models = ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"].map((connectionId) => ({
    modelId: "same-model", label: "同名模型", connectionId, connectionName: "个人连接", configRevision: 1, authGeneration: 1,
    provider: "codex", description: "", source: "personal", contextWindowTokens: 16384, inputModalities: ["text"],
  }));
  act(() => window.dispatchEvent(new CustomEvent("model-catalog-changed", { detail: { profileId: "test-profile", catalog: {
    generation: 1, revision: 1, checkedAt: Date.now(), revalidateAfter: null, error: null, status: { configured: true, endpoint: null, models },
  } } })));
  fireEvent.click(screen.getByRole("button", { name: /^选择模型：/ }));
  expect(screen.getByRole("menuitemradio", { name: /个人连接 \(aaaaaaaa\) · 同名模型/ })).toBeTruthy();
  fireEvent.click(screen.getByRole("menuitemradio", { name: /个人连接 \(bbbbbbbb\) · 同名模型/ }));
  setComposerText("发送给 AI", "只向连接乙发送");
  fireEvent.click(screen.getByRole("button", { name: "发送" }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === "ai_message_submit")).toBe(true));
  const request = invoke.mock.calls.find(([command]) => command === "ai_message_submit")![1].request;
  expect(request.modelSelection).toEqual({ connectionId: models[1].connectionId, modelId: "same-model", reasoningEffort: null });
  expect(request.modelOverride).toBe("same-model");
  expect(JSON.parse(window.localStorage.getItem("cilai.workbench.ai-last-execution-selection")!).connectionId).toBe(models[1].connectionId);
});


test("目录失败保留模型默认思考深度文字与宽度", async () => {
  await renderWorkbench(conversationContext(), { messages: [existingMessage] });
  const models = (modelApi.models as AiRuntimeSnapshot["models"]).map((model) => ({ ...model, supportedReasoningEfforts: ["high"], defaultReasoningEffort: "high" }));
  const catalog = { generation: 0, revision: 2, checkedAt: Date.now(), revalidateAfter: Date.now() + 300_000, error: null, status: { configured: true, endpoint: "https://models.example.test/v1", models } };
  act(() => window.dispatchEvent(new CustomEvent("model-catalog-changed", { detail: { profileId: "test-profile", catalog } })));
  const picker = screen.getByRole("button", { name: "选择模型：Qwen3.8 27B NVFP4 较高" });
  const label = picker.textContent;
  const width = picker.querySelector("span")?.style.width;
  act(() => window.dispatchEvent(new CustomEvent("model-catalog-changed", { detail: { profileId: "test-profile", catalog: { ...catalog, revision: 3, status: null, checkedAt: null, revalidateAfter: null, error: "model_catalog_network_failed" } } })));
  expect(picker.textContent).toBe(label);
  expect(picker.querySelector("span")?.style.width).toBe(width);
});
