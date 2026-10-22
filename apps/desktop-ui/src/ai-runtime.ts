import { invoke } from "./host";
import type { ModelSelection } from "./generated/protocol";
import type { ModelHistoryTransferConsent } from "./model-history-transfer";

export type AiConversationMode = "discuss" | "edit" | "review" | "decision";
export type AiConversationType = "direct" | "project" | "task" | "deliverable" | "file";
export type AiRunStatus = "queued" | "running" | "paused" | "terminating" | "waiting_permission" | "permission_approved" | "completed" | "failed" | "terminated" | "interrupted";
export type AiApprovalMode = "request" | "auto-review" | "full-access";
export type AiReasoningEffort = "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";

export interface AiModelInfo {
  modelId: string;
  connectionId?: string | null;
  connectionName?: string | null;
  configRevision?: number | null;
  authGeneration?: number | null;
  label: string;
  provider: string;
  description: string;
  source: "system" | "platform" | "personal";
  contextWindowTokens: number | null;
  inputModalities: Array<"text" | "image" | "audio">;
  supportedReasoningEfforts?: AiReasoningEffort[];
  defaultReasoningEffort?: AiReasoningEffort | null;
  reasoningControl?: "none" | "toggle" | "effort" | null;
  reasoningEnabledEfforts?: AiReasoningEffort[];
  toolCapabilities?: {
    version: number;
    profile: "responses" | "responses-preview" | "responses-functions" | "functions";
    function_calling: boolean;
    client_tool_search: boolean;
    custom_tools: boolean;
    web_search_mode: "disabled" | "cached" | "live";
  } | null;
}

export interface AiMessageAttachment {
  attachmentId: string;
  fileName: string;
  mimeType: string;
  mediaType: "image" | "video" | "document";
  sizeBytes: number;
  localPath: string;
}

export interface ApplicationContextSelection {
  applicationId: string;
  contextIds: string[];
  toolNames: string[];
  invocationSource: "slash";
}

export interface ApplicationUiActionSnapshot {
  actionId: string;
  title: string;
  description: string;
  inputSchema: Record<string, unknown>;
  preconditions: string[];
  expectedEffects: string[];
}

export interface ApplicationAiContextSnapshot {
  applicationId: string;
  applicationName: string;
  version: string;
  pageRevision: number;
  contexts: Array<{
    contextId: string;
    contextType: string;
    schemaVersion: string;
    pageRevision: number;
    validUntil: string;
    title: string;
    summary: string;
    data: Record<string, unknown>;
    dataClassifications: string[];
    redactedFields: string[];
    availableActionIds: string[];
  }>;
  actions: ApplicationUiActionSnapshot[];
}

export interface AiAgentProfile {
  agentId: string;
  name: string;
  description: string;
  instructions: string;
  provider: string;
  model: string | null;
  skillIds: string[];
  toolPermissions: string[];
  directoryScope: "workspace-read";
  networkAccess: boolean;
  concurrencyLimit: number;
  createdAt: number;
  updatedAt: number;
}

export interface AiPermissionRequest {
  permissionRequestId: string;
  runId: string;
  agentId: string;
  workspaceRoot: string;
  directoryScope: string;
  tools: string[];
  networkAccess: boolean;
  credentialLabels: string[];
  reason: string;
  riskLevel: string;
  status: "pending" | "approved" | "rejected" | "expired" | "consumed";
  requestedAt: number;
  validUntil: number;
  decidedAt: number | null;
  decidedBy: string | null;
  decisionScope?: "turn" | "session" | null;
  protocolRequestId?: string | number | null;
  protocolMethod?: string | null;
  protocolItemId?: string | null;
  command?: string | null;
  resource?: string | null;
  requestedPermissions?: unknown;
  availableDecisions?: string[];
  applicationWrite?: AiApplicationWriteApproval | null;
}

export interface AiApplicationWriteApproval {
  confirmationId: string;
  applicationId: string;
  applicationName: string;
  toolName: string;
  toolTitle: string;
  description: string;
  riskLevel: "write" | "highRisk";
  arguments: Record<string, unknown>;
  confirmationView?: AiApplicationConfirmationView | null;
  executionStatus?: "awaiting_confirmation" | "submitting" | "succeeded" | "failed" | "outcome_unknown" | "cancelled" | "expired" | null;
  receipt?: Record<string, unknown> | null;
  failureReason?: string | null;
}

export interface AiApplicationConfirmationView {
  reviewMode?: "editableProposal" | "actionConfirmation";
  prepareTool?: string;
  title: string;
  submitLabel: string;
  impact: string;
  hostGeneratedFields?: string[];
  fields: Array<{
    name: string;
    label: string;
    visibility?: "primary" | "detail" | "technical";
    valueLabels?: Record<string, string>;
    editable?: boolean;
  }>;
}

export interface AiAutomationRule {
  modelId?: string | null;
  conversationId?: string | null;
  automationRuleId: string;
  name: string;
  triggerType: "interval";
  intervalMinutes: number;
  scheduleKind: AutomationScheduleKind;
  timeOfDayMinutes: number;
  weekday: number;
  weekdays?: number[];
  notifyOnEveryRun: boolean;
  conditionSummary: string;
  actionPrompt: string;
  workspaceId: string;
  workspaceName: string;
  workspaceRoot: string;
  projectId: string | null;
  projectName: string | null;
  agentId: string;
  riskLevel: "low";
  ruleVersion: number;
  enabled: boolean;
  nextRunAt: number | null;
  lastRunId: string | null;
  lastRunAt: number | null;
  lastError: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface AiContext {
  workspaceId: string | null;
  workspaceName: string | null;
  workspaceRoot: string;
  workspaceRoots?: string[];
  teamSpaceId?: string | null;
  projectId: string | null;
  projectName: string | null;
  phaseId: string | null;
  phaseName: string | null;
  taskId: string | null;
  taskName: string | null;
  deliverableId: string | null;
  deliverableName: string | null;
  localFileId: string | null;
  relativePath: string | null;
  fileRevision: number | null;
  referencedMaterials?: Array<{
    localFileId: string;
    relativePath: string;
    fileRevision: number;
    workspaceId?: string | null;
  }>;
}

export type AiMaterialReference = NonNullable<AiContext["referencedMaterials"]>[number];

export interface AiConversation {
  conversationId: string;
  title: string;
  conversationType: AiConversationType;
  mode: AiConversationMode;
  agentId: string | null;
  applicationHandoff?: {
    applicationId: string;
    applicationName: string;
    runId: string;
    userMessageId?: string;
    assistantMessageId?: string;
    runSnapshot?: {
      modelId: string;
      modelLabel: string;
      startedAt: number;
      completedAt: number;
      events: AiRunEvent[];
    } | null;
  } | null;
  approvalMode?: AiApprovalMode | null;
  context: AiContext;
  workClassification: {
    runId: string;
    taskId: string | null;
    taskTitle: string | null;
    conversationStatus?: "active" | "completed" | null;
    stage: "todo" | "in_progress" | "review" | "decision" | null;
    outcomeTitle?: string | null;
    outcomeSummary?: string | null;
    reason: string;
    confidence: number;
  } | null;
  archived: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface AiConversationMessage {
  messageId: string;
  conversationId: string;
  role: "user" | "assistant";
  content: string;
  attachments: AiMessageAttachment[];
  referencedMaterials?: AiMaterialReference[];
  applicationContext?: ApplicationAiContextSnapshot | null;
  runId: string | null;
  createdAt: number;
}

export interface AiAgentRun {
  modelSelection?: ModelSelection | null;
  historyConsent?: ModelHistoryTransferConsent | null;
  runId: string;
  conversationId: string;
  agentId: string;
  inputMessageId: string;
  retryOfRunId: string | null;
  superseded?: boolean;
  regenerationDraft?: { content: string; sourceRunId: string } | null;
  previousRevisions?: Array<{ runId: string; userContent: string; assistantContents: string[]; incomplete: boolean }>;
  provider: string;
  model: string | null;
  modelLabel?: string | null;
  reasoningEffort?: AiReasoningEffort | null;
  executionTarget?: "agent" | "model";
  planning?: boolean;
  status: AiRunStatus;
  threadWriterActive?: boolean;
  pid: number | null;
  startedAt: number;
  completedAt: number | null;
  output: string | null;
  partialOutput?: string | null;
  errorReason: string | null;
  classificationNonce?: string | null;
  classificationStatus?: "pending" | "applied" | "failed" | null;
  classificationError?: string | null;
  sessionId: string | null;
  harnessProtocolVersion?: number | null;
  skillIds: string[];
  toolPermissions: string[];
  permissionRequestId: string | null;
  automationRuleId: string | null;
  automationRuleVersion: number | null;
  sandboxMode: "read-only" | "workspace-write" | "danger-full-access";
  approvalMode?: AiApprovalMode | null;
  events: AiRunEvent[];
  artifacts: Array<{ artifactId: string; artifactType: string; label: string; path: string | null; version?: { digest: string; sizeBytes: number; createdAt: number } | null; feedback?: Array<{ feedbackId: string; versionDigest: string; content: string; createdAt: number }> }>;
  usage: {
    inputTokens: number; cachedInputTokens: number; outputTokens: number;
    reply?: { inputTokens: number; cachedInputTokens: number; outputTokens: number } | null;
    classification?: { inputTokens: number; cachedInputTokens: number; outputTokens: number } | null;
    context?: {
      tokens: number;
      windowTokens: number | null;
      composition?: {
        conversationTokens: number;
        systemPromptTokens: number;
        toolTokens: number;
        skillTokens: number;
        mcpTokens: number | null;
        internalTokens: number;
      } | null;
    } | null;
  };
  logTail: string[];
}

export interface AiRunEvent {
  readPaths?: string[];
  sequence: number;
  eventType: string;
  itemId?: string | null;
  phase?: "reply" | "classification";
  kind?: "status" | "message" | "reasoning" | "command" | "mcp" | "file_change" | "plan" | "context";
  status?: "started" | "completed" | "failed";
  summary: string;
  durationMs?: number | null;
  detailAvailable?: boolean;
  createdAt: number;
}

export interface AiRunEventDetail {
  runId: string;
  itemId: string;
  phase: "reply" | "classification";
  kind: string;
  status: string;
  toolName: string;
  operation: string;
  target: string | null;
  durationMs: number | null;
  parameters: string | null;
  output: string | null;
  exitCode: number | null;
  error: string | null;
  truncated: boolean;
}

export interface AiRuntimeSnapshot {
  stateRevision: number;
  models: AiModelInfo[];
  agents: AiAgentProfile[];
  conversations: AiConversation[];
  messages: AiConversationMessage[];
  runs: AiAgentRun[];
  permissionRequests: AiPermissionRequest[];
  inboxReadItemIds: string[];
  automationRules: AiAutomationRule[];
  /** 已受理但尚未落库的发送；失败时带原因，需由用户恢复或移除。 */
  messageSubmissions?: AiMessageSubmission[];
}

export interface AiMessageSubmission {
  clientMessageId: string;
  conversationId: string;
  status: "preparing" | "failed";
  reason: string | null;
  acceptedAt: number;
}

export interface AiRuntimeActivitySnapshot {
  stateRevision: number;
  conversations: AiConversation[];
  messages: AiConversationMessage[];
  runs: AiAgentRun[];
  permissionRequests: AiPermissionRequest[];
  missingRunIds: string[];
}

export interface AiModelCatalogStatus {
  configured: boolean;
  models: AiModelInfo[];
}

export type AiWindowsSandboxReadiness = "notApplicable" | "ready" | "notConfigured" | "updateRequired" | "error";

export interface AiWindowsSandboxStatus {
  supported: boolean;
  status: AiWindowsSandboxReadiness;
  errorCode: string | null;
  errorMessage: string | null;
}

export const emptyAiSnapshot: AiRuntimeSnapshot = {
  stateRevision: 0,
  models: [], agents: [], conversations: [], messages: [], runs: [], permissionRequests: [], inboxReadItemIds: [], automationRules: [],
};

export function loadAiSnapshot(refreshNativeCatalog = true): Promise<AiRuntimeSnapshot> {
  return refreshNativeCatalog
    ? invoke<AiRuntimeSnapshot>("ai_runtime_snapshot")
    : invoke<AiRuntimeSnapshot>("ai_runtime_snapshot", { refreshNativeCatalog: false });
}

export function loadAiActivitySnapshot(runIds: string[]): Promise<AiRuntimeActivitySnapshot> {
  return invoke<AiRuntimeActivitySnapshot>("ai_runtime_activity_snapshot", { runIds });
}

/** 对话运行状态需要重新读取时广播的事件，例如会话状态从服务端恢复之后。 */
export const aiRuntimeChangedEvent = "cilai-ai-runtime-changed";

export function loadAiRunEventDetail(runId: string, itemId: string): Promise<AiRunEventDetail> {
  return invoke<AiRunEventDetail>("ai_run_event_detail", { runId, itemId });
}

export interface AiMemorySettings {
  lifecycleReady: boolean;
  enabled: boolean;
  fromToolChats: boolean;
  running: boolean;
  hasMemory: boolean;
  modelLabel: string | null;
  lastError: string | null;
  phase: "available" | "empty" | "disabled";
  lastUpdatedAt: number | null;
  service?: {
    bindingId:string;bindingRevision:number;connectionId:string;connectionName:string;configRevision:number;authGeneration:number;
    modelId:string;serviceName:string;baseUrl:string|null;historyScope:string;toolChatsHistoryScope:string;mayIncurCost:boolean;consentDigest:string;
  } | null;
  serviceError?:string|null;
  nativeScopeCount?:number;
}

export type AiMemorySaveSettings = Pick<AiMemorySettings,"enabled"|"fromToolChats"> & {
  confirmation?:{consentDigest:string;fromToolChats:boolean;confirmedAt:number}|null;
};

export function loadAiMemorySettings(): Promise<AiMemorySettings> {
  return invoke<AiMemorySettings>("ai_memory_settings");
}

export function saveAiMemorySettings(settings: AiMemorySaveSettings): Promise<AiMemorySettings> {
  return invoke<AiMemorySettings>("ai_memory_settings_save", { settings });
}

export function loadAiWindowsSandboxStatus(): Promise<AiWindowsSandboxStatus> {
  return invoke<AiWindowsSandboxStatus>("ai_windows_sandbox_status");
}

export function setupAiWindowsSandbox(workspaceRoot: string | null): Promise<AiWindowsSandboxStatus> {
  return invoke<AiWindowsSandboxStatus>("ai_windows_sandbox_setup", { workspaceRoot });
}

export function createAiConversation(request: {
  title: string;
  conversationType: AiConversationType;
  mode: AiConversationMode;
  agentId: string | null;
  context: AiContext;
}): Promise<AiConversation> {
  return invoke<AiConversation>("ai_conversation_create", { request });
}

export function startAiConversation(request: {
  title: string;
  conversationType: AiConversationType;
  mode: AiConversationMode;
  agentId: string;
  context: AiContext;
  content: string;
  clientMessageId?: string;
  attachmentIds?: string[];
  referencedMaterials?: AiMaterialReference[];
  applicationContext?: ApplicationContextSelection | null;
  selectedSkillId?: string | null;
  modelOverride?: string | null;
  modelSelection?: ModelSelection | null;
  historyConsent?: ModelHistoryTransferConsent | null;
  reasoningEffort?: AiReasoningEffort | null;
  executionTarget?: "agent" | "model";
  permissionMode?: "read-only" | "workspace-write" | AiApprovalMode;
  planning?: boolean;
}, clientSentAtMs = Date.now()): Promise<{ conversation: AiConversation; run: AiAgentRun }> {
  return invoke<{ conversation: AiConversation; run: AiAgentRun }>("ai_conversation_start", { request, clientSentAtMs });
}

export function updateAiConversation(request: {
  conversationId: string;
  title: string | null;
  mode: AiConversationMode | null;
  agentId: string | null;
  approvalMode?: AiApprovalMode | null;
  context: AiContext | null;
  archived: boolean | null;
}): Promise<AiConversation> {
  return invoke<AiConversation>("ai_conversation_update", { request });
}

export interface AiConversationDeleteResult {
  deletedConversationIds: string[];
  deletedCount: number;
  cleanupWarning: boolean;
}

export function deleteAiConversations(conversationIds: string[]): Promise<AiConversationDeleteResult> {
  return invoke<AiConversationDeleteResult>("ai_conversation_delete", { request: { conversationIds } });
}

export function sendAiMessage(request: {
  conversationId: string;
  content: string;
  clientMessageId?: string;
  attachmentIds?: string[];
  referencedMaterials?: AiMaterialReference[];
  applicationContext?: ApplicationContextSelection | null;
  selectedSkillId?: string | null;
  retryOfRunId: string | null;
  modelOverride?: string | null;
  modelSelection?: ModelSelection | null;
  historyConsent?: ModelHistoryTransferConsent | null;
  reasoningEffort?: AiReasoningEffort | null;
  executionTarget?: "agent" | "model";
  permissionMode?: "read-only" | "workspace-write" | AiApprovalMode;
  planning?: boolean;
}, clientSentAtMs = Date.now()): Promise<AiAgentRun> {
  return invoke<AiAgentRun>("ai_message_send", { request, clientSentAtMs });
}

export type AiMessageSubmitResult = { status: "accepted" } | { status: "recorded"; run: AiAgentRun };

/**
 * 受理即返回：宿主只做本地核对并登记回执，企业会话、应用发现与运行启动在后台完成。
 * 回执随快照的 messageSubmissions 给出准备中或失败原因；消息落库后回执消失。
 */
export function submitAiMessage(request: Parameters<typeof sendAiMessage>[0], clientSentAtMs = Date.now()): Promise<AiMessageSubmitResult> {
  return invoke<AiMessageSubmitResult>("ai_message_submit", { request, clientSentAtMs });
}

export type AiConversationSubmitResult = AiMessageSubmitResult & { conversation: AiConversation };

export function submitAiConversation(request: Parameters<typeof startAiConversation>[0], clientSentAtMs = Date.now()): Promise<AiConversationSubmitResult> {
  return invoke<AiConversationSubmitResult>("ai_conversation_submit", { request, clientSentAtMs });
}

export function dismissAiMessageSubmission(clientMessageId: string): Promise<void> {
  return invoke<void>("ai_message_submission_dismiss", { clientMessageId });
}

export function regenerateAiMessage(request: {
  conversationId: string;
  messageId: string;
  content: string;
  modelOverride?: string | null;
  modelSelection?: ModelSelection | null;
  historyConsent?: ModelHistoryTransferConsent | null;
  reasoningEffort?: AiReasoningEffort | null;
  executionTarget?: "agent" | "model";
  permissionMode?: "read-only" | "workspace-write" | AiApprovalMode;
  planning?: boolean;
}): Promise<AiAgentRun> {
  return invoke<AiAgentRun>("ai_message_regenerate", { request });
}

export function steerAiRun(runId: string, content: string, clientMessageId: string, expectedPlanning = false): Promise<AiAgentRun> {
  return invoke<AiAgentRun>("ai_message_steer", { request: { runId, content, clientMessageId, expectedPlanning } });
}

const attachmentChunkBytes = 256 * 1024;

const documentMimes: Record<string, string> = {
  pdf: "application/pdf", txt: "text/plain", md: "text/markdown", csv: "text/csv", json: "application/json",
  docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
};
export function attachmentMime(file: File): string {
  return documentMimes[file.name.split(".").at(-1)?.toLowerCase() ?? ""] ?? file.type;
}
export function isDocumentAttachment(file: File): boolean {
  return Object.values(documentMimes).includes(attachmentMime(file));
}

export async function stageAiAttachment(file: File): Promise<AiMessageAttachment> {
  const attachment = await invoke<AiMessageAttachment>("ai_attachment_create", {
    request: { fileName: file.name || "剪贴板图片.png", mimeType: attachmentMime(file), sizeBytes: file.size },
  });
  let offset = 0;
  try {
    while (offset < file.size) {
      const end = Math.min(file.size, offset + attachmentChunkBytes);
      const bytes = Array.from(new Uint8Array(await file.slice(offset, end).arrayBuffer()));
      await invoke<AiMessageAttachment>("ai_attachment_append", {
        request: { attachmentId: attachment.attachmentId, offset, bytes, complete: end === file.size },
      });
      offset = end;
    }
    return attachment;
  } catch (error) {
    await discardAiAttachment(attachment.attachmentId).catch(() => undefined);
    throw error;
  }
}

export function discardAiAttachment(attachmentId: string): Promise<void> {
  return invoke<void>("ai_attachment_discard", { attachmentId });
}

export async function readAiAttachment(attachmentId: string): Promise<ArrayBuffer> {
  const payload = await invoke<ArrayBuffer | Uint8Array | number[]>("ai_attachment_read", { attachmentId });
  if (payload instanceof ArrayBuffer) return payload;
  const bytes = payload instanceof Uint8Array ? payload : Uint8Array.from(payload);
  const normalized = new Uint8Array(bytes.byteLength);
  normalized.set(bytes);
  return normalized.buffer;
}

export function controlAiRun(runId: string, action: "pause" | "resume" | "terminate"): Promise<AiAgentRun> {
  return invoke<AiAgentRun>("ai_run_control", { request: { runId, action } });
}

export function createAiPermissionRequest(runId: string, reason: string): Promise<AiPermissionRequest> {
  return invoke<AiPermissionRequest>("ai_permission_request_create", { request: { runId, reason } });
}

export function resolveAiPermissionRequest(permissionRequestId: string, decision: "approved" | "approved_session" | "rejected", applicationWriteArguments?: Record<string, unknown>): Promise<AiPermissionRequest> {
  return invoke<AiPermissionRequest>("ai_permission_request_resolve", { request: { permissionRequestId, decision, decidedBy: "当前用户", ...(applicationWriteArguments ? { applicationWriteArguments } : {}) } });
}

export type AutomationScheduleKind = "interval" | "daily" | "weekly";

export function saveAiAutomationRule(request: {
  modelId: string | null;
  conversationId: string | null;
  automationRuleId: string | null;
  name: string;
  intervalMinutes: number;
  scheduleKind: AutomationScheduleKind;
  /** Minutes past local midnight; used by the daily and weekly kinds. */
  timeOfDayMinutes: number;
  /** 1 = 周一 … 7 = 周日; used by the weekly kind. */
  weekday: number;
  /** Multiple weekdays for a weekly schedule; older rules fall back to `weekday`. */
  weekdays: number[];
  notifyOnEveryRun: boolean;
  conditionSummary: string;
  actionPrompt: string;
  workspaceId: string;
  workspaceName: string;
  workspaceRoot: string;
  projectId: string | null;
  projectName: string | null;
  agentId: string;
  enabled: boolean;
}): Promise<AiAutomationRule> {
  return invoke<AiAutomationRule>("ai_automation_rule_save", { request });
}

export function runAiAutomationRule(automationRuleId: string): Promise<AiAgentRun> {
  return invoke<AiAgentRun>("ai_automation_rule_run", { request: { automationRuleId } });
}

export function deleteAiAutomationRule(automationRuleId: string): Promise<void> {
  return invoke<void>("ai_automation_rule_delete", { request: { automationRuleId } });
}

export function aiErrorMessage(error: unknown): string {
  /*
   * 宿主经 IPC 抛回来的错误裹着一层 "Error invoking remote method 'host:invoke': Error: "。
   * 此前这里只剥开头的 "Error: "，裹过的一概剥不掉——于是下面每一条 startsWith 判断和末尾
   * 那张映射表全部落空，九十多条写好的中文一句都没用上，员工看到的始终是整串英文原文。
   * 三次 replace 覆盖已知的几种嵌套顺序。
   */
  const value = String(error)
    .replace(/^Error:\s*/, "")
    .replace(/^Error invoking remote method '[^']*':\s*/, "")
    .replace(/^Error:\s*/, "");
  if (value.includes("model_connection_not_configured")) return "请先连接模型。草稿和附件已保留。";
  const personalErrors: Record<string, string> = {
    model_credential_vault_unavailable: "无法访问工作台凭据库。",
    model_credential_vault_root_required: "工作台凭据目录未就绪。",
    model_credential_reentry_required: "请重新保存到工作台本地凭据库。",
    mcp_credential_reentry_required: "请重新保存到工作台本地凭据库。",
    model_credential_legacy_cleanup_required: "旧系统凭据需手工清理，工作台未确认删除。",
    model_network_credential_legacy_cleanup_required: "旧系统凭据需手工清理，工作台未确认删除。",
    mcp_credential_legacy_cleanup_required: "旧系统凭据需手工清理，工作台未确认删除。",
    model_credential_write_failed: "无法写入工作台本地凭据库，连接尚未提交。可刷新后继续或取消未完成操作。",
    model_credential_read_failed: "无法读取工作台本地凭据库，操作已停止。",
    model_credential_readback_failed: "密钥读回核对未通过，连接尚未提交。",
    model_credential_delete_failed: "本地凭据未能删除，操作保持未完成。",
    model_credential_scope_invalid: "凭据归属不匹配，操作已停止。",
    model_credential_reference_invalid: "凭据归属不匹配，执行已停止。",
    model_credential_access_failed: "无法读取此连接的工作台本地凭据。",
    model_credential_pending_value_conflict: "此待处理操作已保存不同密钥。请取消并清理后重新保存。",
    model_credential_operation_in_use: "另一宿主正在处理此资料的凭据，请稍后明确重试。",
    model_credential_operation_lock_failed: "无法取得个人凭据操作锁，操作已停止。",
    model_credential_delete_unconfirmed: "本地凭据删除尚未确认，请核对后继续清理。",
    model_credential_cleanup_stale: "凭据引用已变化，请刷新列表后重新确认清理。",
    model_credential_cleanup_in_use: "此凭据仍被使用，尚未清理。",
    model_credential_required: "请输入此连接的密钥。",
    model_credential_invalid: "此连接的密钥格式无效。",
    model_connection_in_use: "仍有任务使用此连接，请先停止任务或核对已退出任务。",
    model_connection_changed: "连接配置已变化，请刷新后重新操作。",
    model_connection_revision_conflict: "连接配置已变化，本次操作未保存。",
    model_connection_disabled: "此连接已停用。",
    model_auth_required: "此隔离运行环境尚未授权，请先完成授权。",
    model_auth_login_in_progress: "此运行环境已有授权进行中，请先完成或取消。",
    model_auth_login_not_found: "授权已结束或失效，请刷新状态。",
    model_auth_scope_in_use: "此授权仍有任务或状态核对正在使用，请等待完成后再操作。",
    model_auth_operation_not_found: "未找到此授权操作，请刷新状态。",
    model_auth_lifecycle_unconfirmed: "授权进程归属尚未确认，操作已停止。请核对未完成操作。",
    model_auth_process_alive: "授权进程尚未退出，操作保持未完成。",
    model_auth_process_unconfirmed: "无法确认授权进程已退出，操作保持未完成。",
    model_auth_login_unconfirmed: "登录结果尚未确认，请核对未完成授权操作。",
    model_auth_logout_unconfirmed: "退出尚未全部确认，请核对后明确继续退出。",
    model_auth_cancel_unconfirmed: "未能确认授权已经取消，请核对未完成操作。",
    model_auth_state_unavailable: "暂时无法读取授权状态。",
    model_auth_launch_failed: "官方授权进程未能启动。",
    model_auth_configuration_invalid: "官方授权配置未通过核对，操作已停止。",
    model_auth_process_exit_unconfirmed: "无法确认官方授权进程已退出，操作保持未完成。",
    model_runtime_home_isolation_unavailable: "当前系统无法保护原有 Codex 资料，官方授权未启动。",
    model_runtime_home_isolation_gate_required: "受控启动门版本不匹配，官方授权未启动。",
    model_runtime_home_isolation_changed: "运行目录的隔离条件已变化，执行已停止。",
    model_runtime_keyring_isolation_not_supported: "当前系统隔离无法完成官方授权凭据保存。",
    model_runtime_isolation_full_access_not_supported: "此隔离运行方式暂不支持完全访问，请选择受限权限。",
    model_runtime_file_permissions_invalid: "本次文件权限无效，执行未启动。",
    model_runtime_permissions_audit_failed: "无法核对本次文件权限，执行未启动。",
    model_runtime_tool_environment_not_isolated: "工具环境隔离未通过核对，执行未启动。",
    model_runtime_mcp_not_authorized: "检测到本次未授权的 MCP，执行未启动。",
    model_runtime_mcp_unconfirmed: "无法核对本次 MCP 范围，执行未启动。",
    model_runtime_auth_storage_not_isolated: "官方凭据隔离未通过核对，执行已停止。",
    model_runtime_system_config_not_isolated: "检测到未获准的系统配置，执行已停止。",
    model_runtime_unapproved_skill: "检测到本轮未获准的技能，执行已停止。",
    model_runtime_skills_unconfirmed: "无法核对本轮技能范围，执行已停止。",
    model_launch_gate_unavailable: "受控启动门不可用，执行未启动。",
    model_system_proxy_read_failed: "无法读取系统代理，连接已停止。",
    model_system_proxy_invalid: "系统代理配置无效，连接已停止。",
    model_system_proxy_snapshot_invalid: "系统代理配置无法核对，连接已停止。",
    model_system_proxy_platform_not_supported: "此系统尚不支持读取系统代理。",
    model_system_proxy_pac_not_supported: "暂不支持系统 PAC 代理，请明确配置其他网络方式。",
    model_system_proxy_auto_discovery_not_supported: "暂不支持自动发现代理，请明确配置其他网络方式。",
    model_system_proxy_scoped_not_supported: "暂不支持系统分域代理，连接已停止。",
    model_system_proxy_bypass_not_supported: "系统代理旁路规则暂不受支持，连接已停止。",
    model_system_proxy_socks_not_supported: "暂不支持系统 SOCKS 代理，请明确配置其他网络方式。",
    model_system_proxy_protocol_not_supported: "系统代理协议暂不受支持，连接已停止。",
    model_proxy_scheme_not_supported: "指定代理的协议暂不受支持。",
    model_network_snapshot_failed: "无法固定本次网络配置，连接已停止。",
    personal_cloud_materials_unavailable: "云端材料不可用，请改用本地文件。",
    model_network_snapshot_unavailable: "本次网络配置尚未就绪，连接已停止。",
    model_network_snapshot_changed: "本次网络配置已变化，连接已停止。",
    model_proxy_auth_not_supported: "此构建尚不支持代理认证。",
    model_network_credential_required: "请补全此连接的网络认证。",
    model_network_credential_rotation_required: "网络认证范围已变化，请重新输入。",
    model_network_credentials_changed: "网络认证已变化，本次连接已停止。",
    model_network_credential_access_failed: "无法读取此连接的网络认证。",
    model_network_credential_scope_invalid: "网络认证归属无法核对，连接已停止。",
    model_network_credential_pending_value_conflict: "此保存操作的认证内容已变化，请重新保存。",
    model_network_credential_action_invalid: "网络认证操作无效，请重新选择。",
    model_network_credential_invalid: "网络认证格式无效，请重新输入。",
    model_network_credential_readback_failed: "网络认证保存结果未确认。",
    model_network_header_invalid: "附加请求头无效，请检查名称与内容。",
    model_proxy_auth_route_invalid: "代理认证与当前网络方式不匹配。",
    model_service_auth_conflict: "服务认证方式冲突，请检查此连接。",
    model_chatgpt_headers_not_supported: "官方 ChatGPT 授权不支持附加服务请求头。",
    model_proxy_authentication_rejected: "代理未接受认证。",
    model_proxy_config_invalid: "代理配置无效。",
    model_proxy_credentials_required: "请补全代理认证。",
    model_proxy_header_timeout: "代理响应超时，连接已停止。",
    model_proxy_headers_invalid: "代理响应无效，连接已停止。",
    model_proxy_url_credentials_forbidden: "代理地址不能包含账号或密码。",
    model_proxy_url_invalid: "代理地址无效。",
    model_proxy_url_required: "请输入代理地址。",
    model_native_proxy_admission_refused: "本次网络转发未获授权。",
    model_native_proxy_auth_invalid: "本次网络转发认证无法核对。",
    model_native_proxy_connect_timeout: "网络转发连接超时。",
    model_native_proxy_exit_unconfirmed: "网络转发尚未核对退出，占用保持。",
    model_native_proxy_hostname_invalid: "网络转发目标无效。",
    model_native_proxy_route_invalid: "网络转发路径与此连接不匹配。",
    model_native_proxy_scope_revoked: "本次网络转发授权已撤销。",
    model_native_proxy_start_failed: "网络转发未能启动。",
    model_native_proxy_start_unconfirmed: "网络转发启动状态未确认。",
    model_native_proxy_state_unavailable: "无法读取网络转发状态。",
    model_native_proxy_tls_failed: "网络转发的安全连接失败。",
    model_native_proxy_tls_timeout: "网络转发的安全连接超时。",
    model_ca_certificate_invalid: "CA 证书无效，请提供公开的 PEM 证书。",
    model_ca_certificate_home_invalid: "无法核对连接的证书目录。",
    model_ca_certificate_storage_failed: "证书未能安全保存，请核对连接目录。",
    model_auth_logout_required: "请先退出此连接的授权，再停用或删除连接。",
    model_connection_kind_immutable: "连接类型不能更改，请另建连接。",
    model_endpoint_credential_replace_required: "地址已更改，请重新输入此服务的密钥。",
    model_catalog_network_failed: "无法连接此服务，模型目录未更新。",
    model_catalog_access_failed: "服务拒绝读取模型目录，请检查此连接的认证与权限。",
    model_catalog_response_invalid: "服务返回的模型目录格式不受支持。",
    model_context_window_required: "模型上下文容量未知，请先在模型信息中明确填写。",
    model_selection_missing: "请明确选择连接与模型。",
    model_purpose_binding_required: "请先配置此模型用途。",
    personal_page_ai_not_authorized: "此应用未授权页面 AI。",
    personal_page_model_not_granted: "此应用未获页面模型权限。",
    personal_page_binding_changed: "应用版本或权限已变化，请重新打开。",
    personal_page_document_closed: "页面已关闭或变化，请重新打开。",
    personal_page_frame_changed: "页面已变化，请重新打开。",
    personal_page_scope_revoked: "页面授权已撤销，请重新打开。",
    personal_page_context_expired: "页面上下文已过期，请重新提交。",
    personal_page_action_outcome_unconfirmed: "操作结果未确认，请核对回执。",
    personal_page_close_unconfirmed: "页面退出尚未确认，请核对状态。",
    personal_page_retirement_unconfirmed: "页面执行尚未确认退出。",
    personal_page_request_limit: "页面请求已达上限，请关闭后重新打开。",
    personal_page_document_limit: "页面数量已达上限，请关闭不用的页面。",
    personal_page_method_not_allowed: "此页面请求未获授权。",
    application_write_arguments_invalid: "参数不符合当前约束，请修改后确认。",
    model_purpose_disabled: "此模型用途尚未启用。",
    model_history_consent_required: "请先确认此用途发送的上下文和费用。",
    model_history_consent_stale: "连接或历史已变化，请重新确认发送范围。",
    model_thread_history_consent_required: "向此连接继续会话前，需要确认历史发送范围。",
    model_execution_snapshot_changed: "执行配置已变化，尚未发送，请重新确认。",
    model_descriptor_not_found: "模型记录不存在，请重新添加。",
    model_validation_model_required: "请选择待验证模型。",
    model_descriptor_stale: "模型记录与当前连接配置不一致，请编辑模型并保存后验证。",
    model_validation_stale: "验证对应的连接配置已变化，请重新验证。",
    model_validation_authentication_failed: "服务未接受此连接的认证。",
    model_validation_authentication_not_established: "未取得认证校验依据，无法确认凭据。",
    model_validation_network_failed: "无法连接目标服务。",
    model_validation_network_http_rejected: "目标服务拒绝了网络验证请求。",
    model_validation_model_not_in_catalog: "目录中未找到所选模型，尚未证明调用权限。",
    model_validation_streaming_delta_missing: "未观察到有效的流式内容。",
    model_validation_inference_nonce_mismatch: "模型未返回本次验证要求的内容。",
    model_validation_tool_call_missing: "未观察到本次要求的工具调用。",
    model_validation_tool_result_invalid: "工具调用结果未通过核对。",
    model_validation_cleanup_unconfirmed: "验证进程或执行占用尚未确认退出。",
    model_validation_runtime_version_mismatch: "实际运行时与锁定版本不一致。",
    model_validation_runtime_failed: "模型验证未完成，运行时返回了错误。",
    model_validation_empty_response: "本次运行已结束，但未收到可用回复。",
    model_tools_validation_required: "此模型尚未通过工具调用验证，请先在模型设置中验证。",
    model_review_purpose_not_enabled: "请先单独配置并启用自动审核用途。",
    model_memory_purpose_not_enabled: "请先单独配置并启用后台记忆用途。",
  };
  const connectionErrors: Record<string, string> = {
    MODEL_AUTH_REJECTED: "服务拒绝此连接的认证，请检查授权。",
    MODEL_RATE_LIMITED: "服务已限流，请稍后明确重试。",
    MODEL_SERVICE_UNAVAILABLE: "模型服务暂时不可用。",
    MODEL_REQUEST_REJECTED: "模型服务拒绝了此请求。",
    MODEL_SESSION_PROXY_MODEL_SCOPE_INVALID: "模型请求与已确认的执行范围不一致，发送已停止。",
    MODEL_SESSION_PROXY_FALLBACK_FORBIDDEN: "请求包含自动切换规则，发送已停止。",
  };
  const connectionCode = value.match(/\bMODEL_[A-Z_]+\b/)?.[0];
  if (connectionCode && connectionErrors[connectionCode]) return connectionErrors[connectionCode];
  const personalCode = value.match(/\bmodel_[a-z_]+\b/)?.[0];
  if (personalCode && personalErrors[personalCode]) return personalErrors[personalCode];
  if (value.startsWith("powershell_policy_failed: ")) return value.slice("powershell_policy_failed: ".length);
  // 反向验证过：改成只剥一层，上面那条 IPC 包装的用例立刻变红
  if (value.includes("ai_response_legacy_envelope_rejected")) return "检测到旧版模型会话输出，已停止发布。请重新发送以建立新会话。";
  if (value.includes("ai_state_revision_conflict")) return "对话状态已变化，本次操作未保存，请重新启动工作台后重试。";
  if (value.startsWith("runtime_action_failed:")) return `工具操作失败：${value.slice("runtime_action_failed:".length).trim()}`;
  if (value.startsWith("runtime_action_spawn_failed:")) return `无法启动工具操作：${value.slice("runtime_action_spawn_failed:".length).trim()}`;
  if (value.startsWith("runtime_action_timed_out:")) return "工具操作超过 2 分钟，已自动终止。请检查网络后重试。";
  if (value.startsWith("runtime_update_check_failed:")) return "本地工具检查异常结束，请重试。";
  if (value.startsWith("runtime_conflict_analysis_timed_out:")) return "默认大模型分析超过 2 分钟，已自动终止。";
  if (value.startsWith("runtime_conflict_analysis_spawn_failed:")) return "无法启动默认大模型，请检查默认工具是否可执行并已登录。";
  if (value.startsWith("runtime_conflict_analysis_failed:")) return `默认大模型分析失败：${value.slice("runtime_conflict_analysis_failed:".length).trim()}`;
  if (value === "runtime_conflict_analysis_empty") return "默认大模型未返回诊断结果。";
  if (value.startsWith("runtime_conflict_analysis_invalid")) return "默认大模型返回的解决方案格式无效，请重新分析。";
  if (value === "runtime_conflict_model_unavailable") return "默认大模型当前不可用，请先确认默认 AI CLI 已安装并登录。";
  if (value === "runtime_conflict_analysis_expired" || value === "runtime_conflict_analysis_stale") return "本机安装状态已经变化，请重新诊断后再确认。";
  if (value === "runtime_conflict_choice_incomplete" || value === "runtime_conflict_choice_invalid") return "请选择每个冲突工具要由 词来 使用的路径。";
  if (value.startsWith("runtime_install_conflict:")) return "检测到重复安装或路径冲突，请先处理冲突后再继续。";
  if (value.startsWith("runtime_package_manager_unavailable:")) return `未检测到所需的 ${value.slice("runtime_package_manager_unavailable:".length)}，无法执行此操作。`;
  if (value.includes("thread_owner_busy")) return "上一轮仍在收尾，请稍后继续。";
  if (value.includes("thread_owner_recovery_required")) return "上一轮异常中断，执行状态需要核验后才能继续。";
  if (value.includes("message_delivery_uncertain")) return "消息接收状态尚未确认，已暂停发送，避免重复执行。";
  if (value.includes("thread_binding_blocked")) return "对话接续未完成，已暂停发送。原记录已保留。";
  if (value.includes("thread_binding_invalid")) return "对话绑定的模型线程记录无效，已暂停发送。原记录已保留。";
  if (value.includes("thread_edit_prefix_unavailable")) return "这条补充消息与前文属于同一轮，目前无法单独编辑。";
  if (value.includes("history_migration_required") || value.includes("history_attachment_")) return "历史上下文尚未完整恢复，请检查客户端版本和原附件。";
  if (value.includes("model_history_incompatible")) return "所选模型无法处理这段对话中的附件，请选择支持图像的模型。";
  if (value.includes("thread_recovery_") || value.includes("thread_owner_identity_unavailable")) return "暂时无法核对原执行状态，已保留对话，请稍后重试。";
  if (value.includes("history_item_too_large")) return "单条历史内容过大，尚未完成接续，请先处理原附件。";
  if (value.includes("thread_message_mapping_missing")) return "这条历史消息缺少原生轮次记录，暂时无法编辑。";
  if (value.includes("ai_message_identity_conflict")) return "同一条消息的内容已变化，请作为新消息发送。";
  if (value === "codex_harness_steer_timed_out") return "补充内容的接收状态尚未确认，请勿重复发送。";
  if (value.startsWith("codex_harness_steer_failed:")) return "当前回复暂时无法接收引导，内容未提交，可排队为下一条。";
  if (value === "codex_harness_turn_not_active") return "当前回复无法接收补充。";
  if (value === "windows_sandbox_not_configured") return "Windows 安全沙箱尚未初始化，请先在模型设置中完成初始化。";
  if (value === "windows_sandbox_update_required") return "Windows 安全沙箱需要更新，请在模型设置中重新初始化。";
  if (value === "windows_sandbox_full_access_forbidden") return "Windows 本地模型必须在安全沙箱中运行，不能使用完全访问权限。";
  if (value === "ai_full_access_unsupported_platform") return "当前平台不支持本地模型完全访问权限。";
  if (value === "windows_sandbox_setup_in_progress") return "Windows 安全沙箱正在初始化，请等待当前操作完成。";
  if (value.startsWith("windows_sandbox_helper_missing")) return "Windows 安全沙箱组件不完整，请重新安装当前版本的 词来。";
  if (value.startsWith("windows_sandbox_setup_failed:")) return "Windows 安全沙箱初始化失败，请检查 UAC、组策略，或联系 IT 管理员。";
  if (value.startsWith("windows_sandbox_")) return `Windows 安全沙箱不可用：${value.split(":").slice(1).join(":").trim() || "请重新检查状态。"}`;
  if (value.startsWith("codex_harness_bundled_runtime_")) return "应用内置的模型执行服务不可用，请重新安装当前版本的 词来。";
  if (value.startsWith("codex_harness_model_catalog_")) return "应用内置的模型配置无法加载，请重新安装当前版本的 词来。";
  if (value.startsWith("codex_harness_")) return `直接模型运行启动失败：${value.split(":").slice(1).join(":").trim() || "请检查模型网关配置。"}`;
  if (value.startsWith("conversation_isolated_workspace_create_failed")) return "无法准备当前对话的产物空间，请检查本机存储后重试。";
  if (value === "cloud_material_size_missing") return "无法确认云端文件大小，不能加入对话。";
  if (value === "cloud_material_too_large") return "云端文件超过 512 MB，不能加入对话。";
  if (value.startsWith("conversation_isolated_material_copy_failed:")) return `无法将所选文件加入当前对话：${value.slice("conversation_isolated_material_copy_failed:".length).trim()}`;
  if (value.startsWith("conversation_delete_automation_bound:")) return `该会话仍被自动化“${value.slice("conversation_delete_automation_bound:".length)}”使用，请先调整自动化。`;
  if (value === "application_skill_not_installed") return "当前应用所需的 Skill 尚未安装或未启用，请在技能管理中安装并启用后重试。";
  if (value === "application_skill_version_mismatch") return "当前应用所需的 Skill 版本不匹配，请在技能管理中更新后重试。";
  if (value === "application_skill_selection_conflict") return "当前应用已绑定固定 Skill，请移除手动选择的其他 Skill 后重试。";
  if (value === "application_skill_requires_model_harness") return "当前应用绑定了必需 Skill，只能使用模型运行。";
  if (value === "application_skill_binding_invalid") return "当前应用的 Skill 绑定配置无效，请联系管理员检查应用清单。";
  if (value === "application_skill_snapshot_unavailable") return "当前应用所需的 Skill 无法加载，请在技能管理中检查后重试。";
  if (value.startsWith("application_skill_")) return "当前应用所需的 Skill 不可用，请在技能管理中检查安装、启用状态和版本后重试。";
  const labels: Record<string, string> = {
    capture_text_only_required: "电脑仅接收文本材料，请在采集端完成识别。",
    capture_source_mismatch: "材料来源与当前设备不一致。",
    capture_manifest_invalid: "文字材料格式无效，请在采集端核对。",
    capture_share_selection_invalid: "请选择本次要分享的文字材料。",
    capture_text_speech_receipt_mismatch: "文字与原转写回执不一致，请核对原记录。",
    capture_text_speech_receipt_invalid: "原转写回执无法核验，请保留原记录。",
    capture_text_speech_receipt_timeout: "原转写核验超时，请稍后查询原记录。",
    capture_hardware_speech_response_invalid: "后台授权回执不匹配，请核对后撤销本次授权。",
    capture_hardware_speech_request_invalid: "请选择有效硬件和语音模型。",
    capture_model_unavailable: "当前模型不可用，请检查模型授权。",
    capture_agent_unavailable: "所选 Agent 已不可用，请重新选择。",
    capture_execution_config_invalid: "执行配置不匹配，请核对后重试。",
    capture_analysis_legacy_configuration_missing: "旧整理配置缺失，请查看原对话。",
    capture_handoff_legacy_configuration_missing: "旧委派配置不可用于新执行，请查看原对话。",
    capture_handoff_target_mismatch: "执行目标与原委派不一致。",
    capture_handoff_run_mismatch: "执行记录与原委派不一致。",
    capture_analysis_authorization_timeout: "授权核验超时，请查看原整理记录。",
    capture_analysis_target_conflict: "整理目标已变化，请核对原配置。",
    capture_receiver_not_running: "请先开启接收。",
    capture_device_results_permission_invalid: "请选择记录、权限和有效期限。",
    capture_device_results_permission_denied: "结果许可已失效，请重新核对。",
    capture_device_results_permission_conflict: "结果许可已变化，请刷新。",
    capture_device_results_permission_already_active: "请先撤销原结果许可。",
    capture_device_results_revision_conflict: "记录已变化，请刷新后核对。",
    capture_device_results_binding_changed: "设备绑定已变化，请重新核对。",
    capture_device_results_authorization_unavailable: "暂时无法核验设备授权。",
    capture_device_results_models_unavailable: "暂时无法读取模型目录。",
    capture_device_results_model_unavailable: "所选模型当前不可用。",
    capture_device_results_intent_unavailable: "当前材料不支持此处理。",
    capture_device_results_operation_pending: "原处理尚未结束，请先核对。",
    capture_device_results_material_too_large: "材料超出本次整理范围。",
    capture_device_results_version_conflict: "内容版本已变化，请重新核对。",
    ai_run_planning_mismatch: "此消息与当前回复的计划模式不同，请保留排队发送。",
    model_catalog_access_revoked: "模型使用资格已变更，请刷新目录后重新选择。",
    ai_planning_requires_model: "计划模式需要选择模型。",
    ai_document_requires_model: "文档材料需要选择模型。",
    APPLICATION_INVOCATION_REQUIRED: "主对话只能通过 /应用 显式启用应用工具。",
    APPLICATION_TOOL_SELECTION_INVALID: "所选应用工具已不可用，请重新输入 / 选择。",
    APP_SESSION_INVALID: "应用页面会话已失效，请返回应用刷新后重新选择。",
    ORIGIN_NOT_ALLOWED: "当前应用页面来源不在允许范围内。",
    PERMISSION_DENIED: "当前企业、成员或应用没有读取该页面上下文的权限。",
    CONTEXT_INVALID: "所选应用页面上下文已变化或不再可用，请重新选择。",
    PAGE_REVISION_CONFLICT: "应用页面状态已变化，请刷新上下文后重试。",
    ACTION_NOT_FOUND: "该页面动作当前不可用，请刷新页面上下文后重试。",
    BUSINESS_RESULT_UNKNOWN: "页面未在规定时间内确认动作结果，请检查当前页面状态。",
    application_webview_not_found: "应用页面已关闭，请重新打开后再选择上下文。",
    ai_application_context_unvalidated: "应用页面上下文未通过工作台校验，请重新选择。",
    ai_application_action_unvalidated: "应用页面动作未通过工作台校验，请重新选择。",
    conversation_project_required: "请先选择新对话所属的项目。",
    conversation_session_changed: "当前企业或输入会话已变化，请重新发起转交。",
    application_handoff_missing: "转交记录已不存在，请从原回答重新发起。",
    application_handoff_source_missing: "原转交记录缺少模型或时间信息，请在应用中重新生成回答后转交。",
    application_handoff_context_missing: "转交来源资料不可用，请保留当前对话并核查原记录。",
    application_handoff_context_mismatch: "转交来源与当前对话不一致，请保留原记录并核查。",
    application_handoff_context_invalid: "转交来源资料格式无效，请保留原记录并核查。",
    application_handoff_output_changed: "回答校验失败，未创建对话，请核对原回答。",
    application_handoff_records_invalid: "转交内容校验失败，未创建对话。",
    application_handoff_persistence_pending: "对话保存结果尚未确认，请重试核对后再切换企业。",
    application_handoff_storage_unavailable: "正式对话存储当前不可用，请重试。",
    conversation_project_workspace_mismatch: "所选文件夹不属于该项目，请重新选择项目。",
    conversation_project_workspaces_invalid: "项目工作区信息不完整，请刷新项目后重试。",
    conversation_project_workspace_unavailable: "项目中的工作区当前不可用，请检查文件夹连接状态。",
    conversation_project_immutable: "已有对话的项目归属不能通过切换工作区修改，请在目标项目新建对话。",
    conversation_not_found: "该对话已不存在，请刷新列表。",
    conversation_archive_ids_invalid: "请重新选择会话，每次最多处理 1,000 条。",
    conversation_archive_not_attempted: "前面的会话未能完成操作，此会话尚未处理。",
    conversation_delete_selection_required: "请先选择要删除的已归档会话。",
    conversation_delete_requires_archive: "只能彻底删除已归档会话，请刷新后重试。",
    conversation_delete_run_active: "所选会话仍有未结束的运行，暂时不能删除。",
    conversation_delete_duplicate: "删除清单包含重复会话，请重新选择。",
    conversation_delete_id_invalid: "删除清单中的会话标识无效，请刷新后重试。",
    conversation_delete_storage_id_invalid: "会话的本机存储标识无效，未执行删除。",
    conversation_delete_attachment_path_invalid: "会话附件的本机存储位置异常，未执行删除。",
    conversation_isolated_material_invalid: "所选文件路径无效，请重新选择文件。",
    conversation_isolated_material_unavailable: "所选文件当前不可用，请刷新工作区后重新选择。",
    conversation_agent_not_found: "请在对话框中选择可用的模型。",
    ai_local_agents_removed: "本地智能体已下线，请改用模型执行。",
    automation_model_required: "自动化任务需要选择模型。",
    conversation_run_already_active: "当前对话已有运行正在执行，请等待完成或先终止。",
    application_package_identity_invalid: "企业能力目录中的插件或 Skill 身份配置无效，请联系管理员检查能力编号和版本。",
    message_submission_in_progress: "这条消息仍在准备执行，请稍候。",
    ai_run_steer_not_supported:"当前回复不支持运行中引导，可将内容排队为下一条消息。",
    ai_run_steer_not_active: "当前回复暂不能接收补充，请先恢复回复或处理审批。",
    ai_message_required: "请输入本次请求。",
    ai_message_too_large: "消息超过 20,000 字符，请缩小本次请求范围。",
    ai_message_regenerate_not_found: "当前对话没有可重新编辑的用户消息。",
    ai_message_regenerate_latest_only: "只能重新编辑当前对话中最近一条用户消息。",
    ai_message_regenerate_run_not_found: "最近一条消息没有可重新生成的运行记录。",
    ai_model_required: "请选择一个模型后再启动直接模型运行。",
    ai_memory_busy: "当前有对话或维护操作正在进行，请稍后删除记忆。",
    ai_memory_enable_consent_required: "启用前请确认实际记忆服务、历史发送范围和费用。",
    ai_memory_enable_consent_stale: "记忆服务或范围已变化，请刷新后重新确认。",
    ai_memory_disabled: "后台记忆已停用。",
    ai_memory_preferences_changed: "记忆范围已变化，旧执行授权已撤销。",
    model_memory_binding_required: "请先单独配置后台记忆用途。",
    ai_memory_path_denied: "本机记忆路径无法核对，操作已停止。",
    ai_memory_reset_preview_required: "请先核对当前资料的全部记忆范围，再确认删除。",
    ai_memory_reset_failed: "记忆删除失败，请重试。",
    harness_runtime_retire_unconfirmed: "运行进程尚未确认停止，暂时无法切换身份或清除记忆。",
    ai_memory_read_failed: "无法读取本机记忆状态。",
    ai_review_models_empty: "当前没有可用的审核与记忆模型，请先配置可用模型。",
    ai_review_model_unavailable: "审核与记忆模型已不可用，请在设置中重新选择。",
    ai_review_model_reasoning_unavailable: "审核与记忆模型未配置思考能力，请在平台完成模型配置后重新选择。",
    ai_model_unavailable: "所选模型不在当前平台模型目录中。",
    ai_attachment_invalid: "附件为空、过大或文件名无效。",
    ai_attachment_type_unsupported: "仅支持 PNG、JPEG、WebP、GIF 图片，以及不超过 60 秒的 MP4、MOV、WebM 短视频。",
    ai_attachment_count_exceeded: "一次最多添加 8 个附件。",
    ai_attachment_not_supported: "当前执行方式或模型尚未接通该附件类型，请检查模型能力和本地运行时。",
    ai_attachment_content_invalid: "附件内容与文件类型不一致。",
    ai_video_processor_unavailable: "未检测到 ffmpeg/ffprobe，暂时不能处理短视频。",
    ai_video_duration_exceeded: "短视频不能超过 60 秒。",
    ai_video_invalid: "无法识别该视频文件。",
    ai_execution_target_invalid: "执行方式无效，请重新选择智能体或模型。",
    agent_run_control_not_allowed: "当前运行状态不支持此操作。",
    permission_request_already_pending: "该运行已有待处理的权限请求。",
    permission_request_expired: "权限请求已过期，请重新发起。",
    automation_interval_invalid: "自动化间隔必须在 15 分钟到 7 天之间。",
    automation_rule_content_required: "请填写规则名称和动作说明。",
    automation_rule_not_found: "该自动化任务已不存在，请刷新列表。",
    automation_previous_run_active: "上次任务尚未结束，本次已跳过。可在触发记录中核对。",
    automation_missed_during_sleep: "睡眠期间错过计划，未补跑。",
    automation_claim_owner_active: "原宿主仍在运行，占用保持。",
    automation_claim_process_unconfirmed: "无法核对相关进程退出，占用保持。",
    automation_claim_model_lease_active: "相关模型任务尚未核对退出，占用保持。",
    automation_claim_restored_read_only: "恢复历史只读，不可变更旧任务占用。",
    automation_claim_instance_changed: "任务所属资料实例已变化，不能变更占用。",
    automation_trigger_already_accepted: "该触发已受理，不会重复执行。",
    automation_schedule_changed: "计划已变化，请刷新后核对。",
    automation_project_required: "请先选择执行项目。",
    automation_conversation_invalid: "执行对话不可用，请重新选择该项目下未归档的对话。",
    ai_workspace_unavailable: "当前工作区目录不可访问，无法启动 AI 运行。",
    runtime_action_invalid: "无效的工具操作。",
    runtime_action_not_supported: "当前工具没有经过验证的自动安装或升级方式。",
    runtime_provider_unknown: "未识别该本地 AI 工具。",
    runtime_already_installed: "该工具已安装，无需重复安装。",
    runtime_not_installed: "该工具尚未安装，无法执行升级。",
  };
  return labels[value] ?? value;
}
