import { ConversationInput, ConversationFields, conversationRunLabel, conversationModelLabel } from "@cilai/app-ui/conversation";
import { skillMessageText } from "./skill-message";
import { markdownText } from "./markdown-text";
import { useModelCatalog, ModelCatalogNotice } from "./use-model-catalog";
import { useWindowsSandbox } from "./use-windows-sandbox";
import { usePageActive } from "./page-visibility";
import { displayModelId } from "./model-label";
import { loadWorkspaceFileSnapshot } from "./workspace-file-pages";
import type { WorkReuseIntent } from "./work-reuse";
import { useConversationSessionFlush, useConversationSessionState } from "./conversation-session";
import { conversationFileArtifacts, type ConversationArtifact } from "./ai-artifacts";
import { ArtifactVisual, ImageArtifactLightbox, imageArtifactMimeType } from "./artifact-preview";
export { conversationFileArtifacts } from "./ai-artifacts";
import { SkillInstallationStatus, useSkillInstallations } from "./skill-installations";
import { conversationArchiveChanged, mergeArchivedConversations, setConversationsArchived, type ConversationArchiveChange } from "./conversation-archive";
import { Fragment, type CSSProperties, type ReactNode, type SetStateAction, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "./page-portal";
import { randomUuid } from "./uuid";
import { useModelHistoryTransfer, type ModelHistoryTransferConsent } from "./model-history-transfer";
import { invoke } from "./host";
import type { DecisionRequestCreateResult, ModelSelection, SkillConversationCatalogItem, SkillConversationCatalogResult, WorkspaceFileItem, WorkspaceSummary } from "./generated/protocol";
import { rpc, sha256 } from "./app-server-rpc";
import { describeServerError } from "./server-error";
import {
  aiErrorMessage,
  controlAiRun,
  discardAiAttachment,
  emptyAiSnapshot,
  loadAiActivitySnapshot,
  loadAiRunEventDetail,
  loadAiSnapshot, aiRuntimeChangedEvent,
  regenerateAiMessage,
  readAiAttachment,
  resolveAiPermissionRequest,
  submitAiMessage,
  dismissAiMessageSubmission,
  submitAiConversation,
  isDocumentAttachment,
  stageAiAttachment,
  steerAiRun,
  updateAiConversation,
  type AiAgentRun,
  type AiRunEvent,
  type AiRunEventDetail,
  type AiApprovalMode,
  type AiContext,
  type AiConversation,
  type AiConversationMode,
  type AiConversationType,
  type AiRuntimeSnapshot,
  type AiWindowsSandboxStatus,
  type AiMessageAttachment,
  type AiMaterialReference,
  type AiPermissionRequest,
  type AiReasoningEffort,
  type ApplicationContextSelection,
} from "./ai-runtime";
import { writeClipboard } from "./clipboard";
import { Select } from "./select";
import { useModal } from "./use-modal";
import { AiBrowserToggle, type AiBrowserState } from "./ai-browser";
import { ConversationNavigator, type NavigatorMessage } from "./conversation-navigator";
import { conversationMinWidth } from "./ai-layout";
import "./ai-workbench.css";
import { ContextMenu, openContextMenu, type ContextMenuItem, type ContextMenuState } from "./context-menu";
import type { MessageSink } from "./app-message";
import { clockMoment, formatCount, timelineMoment } from "./datetime";
import { ConfirmDialog } from "./confirm-dialog";
import { WorkspacePermissions } from "./workspace-permissions";
import { RichMenu, RichMenuEmpty, RichMenuItem, RichMenuSection, RichMenuSeparator } from "./rich-menu";
import { ArchiveIcon, MoreIcon, PanelToggleIcon, TrashIcon } from "./icons";
import { MarkdownContent } from "./markdown-content";
import { workspaceRelativePath, workspacePathsEqual } from "./workspace-path";
export { workspaceRelativePath } from "./workspace-path";
type ArtifactContextHandler = (event: React.MouseEvent, runId: string, artifact: ConversationArtifact) => void;
import { hostPlatformUi } from "./host-platform-ui";
import { isPreviewableFile, isPreviewablePath } from "./workspace-state";
import { unclassifiedConversationContext } from "./ai-workbench-context";
export { workspaceContext } from "./ai-workbench-context";

export interface AiLaunchRequest {
  token: number;
  title: string;
  conversationType: AiConversationType;
  mode: AiConversationMode;
  context: AiContext;
  projectIds?: string[];
  selectedSkillId?: string;
  prefill?: string;
  startFresh?: boolean;
}

const PERSISTENT_CREATOR_SKILL_IDS = new Set([
  "system.skill-creator",
  "system.capability-creator",
]);

interface Props {
  /** 浏览器模块的开合。入口放在对话工具栏里：AI 浏览通常由对话触发，就近才找得到。 */
  aiBrowser?: {
    open: boolean;
    state: AiBrowserState;
    conversationId: string | null;
    onToggle: () => void;
    onConversationChange: (conversationId: string | null) => void;
    /** 本页最少要多宽。侧栏能拖也能收起，只有这里算得准，主界面据此决定浏览器让到什么程度。 */
    onMinWidthChange: (width: number) => void;
  };
  workspace: WorkspaceSummary | null;
  workspaces?: WorkspaceSummary[];
  hostPlatform: string | null;
  projects: AiProjectOption[];
  files: WorkspaceFileItem[];
  selectedFile: WorkspaceFileItem | null;
  launchRequest: AiLaunchRequest | null;
  preferredConversationId: string | null;
  onPreferredConversationConsumed?: (conversationId: string) => void;
  profileId?: string | null;
  onMessage: MessageSink;
  /** 工作区到项目的对照。会话上下文常常只带工作区，项目身份得从这里补。 */
  projectIndex: Map<string, { projectId: string; displayName: string }>;
  onOpenAgents: () => void;
  onReuseWork?: (kind: "skill" | "automation", intent: WorkReuseIntent) => void;
  canCreateProject: boolean;
  projectCreationOpen?: boolean;
  onCreateProject: () => void;
  onOpenFile?: (localFileId: string, workspaceId: string) => void;
  onOpenWorkspacePath?: (relativePath: string, workspaceId: string) => Promise<boolean>;
  onLaunchConsumed?: (token: number) => void;
  onSelectProject: (projectId: string) => void;
  onOpenSkill?: (skillId: string) => void;
}

export interface AiProjectOption {
  projectId: string;
  displayName: string;
  primaryWorkspaceId: string | null;
  primaryWorkspaceName: string | null;
  primaryWorkspaceRoot: string | null;
  primaryWorkspaceKind?: string | null;
  primaryTeamSpaceId?: string | null;
  workspaceIds: string[];
}

function projectWorkspaceAvailable(project: AiProjectOption) {
  return project.workspaceIds.length === 0 || Boolean(project.primaryWorkspaceId && (project.primaryWorkspaceRoot || project.primaryTeamSpaceId));
}

function projectWorkspaceLabel(project: AiProjectOption) {
  return project.primaryTeamSpaceId ? "云端空间" : project.primaryWorkspaceRoot ?? "未连接文件夹";
}

const pinnedProjectsStorageKey = "cilai.workbench.ai-pinned-projects";
const lastExecutionSelectionStorageKey = "cilai.workbench.ai-last-execution-selection";
const lastViewedConversationStorageKey = "cilai.workbench.ai-last-viewed-conversation";
type ExecutionSelection = { agentId: string | null; modelId: string | null; connectionId?: string | null; reasoningEffort: AiReasoningEffort | null };

function catalogModel(models: AiRuntimeSnapshot["models"], modelId: string | null, connectionId?: string | null) {
  const matches = models.filter((model) => model.modelId === modelId && (model.connectionId ?? null) === (connectionId ?? null));
  return matches.length === 1 ? matches[0] : null;
}

function catalogModelKey(model: AiRuntimeSnapshot["models"][number]) {
  return JSON.stringify([model.connectionId ?? null, model.modelId]);
}

function catalogModelLabel(model: AiRuntimeSnapshot["models"][number], models: AiRuntimeSnapshot["models"] = []) {
  if (!model.connectionId) return model.label;
  const sameName = models.some((item) => item.connectionId !== model.connectionId && item.connectionName === model.connectionName);
  const connection = model.connectionName
    ? `${model.connectionName}${sameName ? ` (${model.connectionId.slice(0, 8)})` : ""}`
    : model.connectionId.slice(0, 8);
  return `${connection} · ${model.label}`;
}
const reasoningEffortLabels: Record<AiReasoningEffort, string> = {
  none: "关闭",
  minimal: "极低",
  low: "较低",
  medium: "标准",
  high: "较高",
  xhigh: "更高",
  max: "最高",
};
const reasoningEfforts = Object.keys(reasoningEffortLabels) as AiReasoningEffort[];

function reasoningLabel(effort: AiReasoningEffort, model: AiRuntimeSnapshot["models"][number] | null) {
  return model?.reasoningControl === "toggle"
    ? (model.reasoningEnabledEfforts?.includes(effort) ? "开启" : "关闭")
    : reasoningEffortLabels[effort];
}
/** 滑块直径，同时也是滑轨高度。位置换算按它把首尾档位收进轨道两端，两处必须一致。 */
const reasoningSliderSize = 32;

function reasoningSliderPosition(index: number, count: number) {
  if (count <= 1) return "50%";
  const ratio = Math.min(1, Math.max(0, index / (count - 1)));
  const percentage = Number((ratio * 100).toFixed(4));
  const offset = Number(((0.5 - ratio) * reasoningSliderSize).toFixed(4));
  return `calc(${percentage}% + ${offset}px)`;
}

function syncReasoningThumbHover(input: HTMLInputElement, clientX: number, clientY: number) {
  const track = input.parentElement;
  if (!track) return;
  const bounds = input.getBoundingClientRect();
  const min = Number(input.min);
  const max = Number(input.max);
  const value = Number(input.value);
  const ratio = max > min ? Math.min(1, Math.max(0, (value - min) / (max - min))) : 0.5;
  const radius = reasoningSliderSize / 2;
  const centerX = bounds.left + radius + ratio * Math.max(0, bounds.width - reasoningSliderSize);
  const centerY = bounds.top + bounds.height / 2;
  track.toggleAttribute("data-thumb-hover", Math.hypot(clientX - centerX, clientY - centerY) <= radius);
}

function executionSelectionStorageKey(profileId?: string | null) {
  return profileId ? `${lastExecutionSelectionStorageKey}:${profileId}` : lastExecutionSelectionStorageKey;
}

function readLastViewedConversationId(): string | null {
  const value = window.localStorage.getItem(lastViewedConversationStorageKey)?.trim();
  return value || null;
}

function readLastExecutionSelection(profileId?: string | null): ExecutionSelection | null {
  const raw = window.localStorage.getItem(executionSelectionStorageKey(profileId));
  if (raw === null) return null;
  const value = JSON.parse(raw) as ExecutionSelection | null;
  const storedEffort: unknown = value && "reasoningEffort" in value ? value.reasoningEffort : null;
  if (!value || (value.agentId !== null && typeof value.agentId !== "string") || (value.modelId !== null && typeof value.modelId !== "string") || (storedEffort !== null && (typeof storedEffort !== "string" || !reasoningEfforts.includes(storedEffort as AiReasoningEffort)))) {
    throw new Error("保存的智能体或模型选择格式无效，请重新选择。");
  }
  return { ...value, reasoningEffort: storedEffort as AiReasoningEffort | null };
}
const conversationRailWidthStorageKey = "cilai.workbench.ai-conversation-rail-width";
const defaultConversationRailWidth = 304;
const minConversationRailWidth = 280;
const maxConversationRailWidth = 520;
const conversationRailKeyboardStep = 16;
const conversationInfoDockWidth = 1040;
const latestOutputThreshold = 48;
const activeRunRefreshInterval = 400;
/** 发送已受理、运行尚未出现时，按此间隔重读快照，等运行出现或受理失败。 */
const submissionRefreshInterval = 600;

function mergeSnapshotEntities<T>(current: T[], updates: T[], key: (item: T) => string, missing = new Set<string>()): T[] {
  const byId = new Map(updates.map((item) => [key(item), item]));
  const kept = current.filter((item) => !missing.has(key(item))).map((item) => byId.get(key(item)) ?? item);
  const existing = new Set(kept.map(key));
  return [...kept, ...[...byId].filter(([id]) => !existing.has(id)).map(([, item]) => item)];
}

function clampConversationRailWidth(width: number) {
  return Math.min(maxConversationRailWidth, Math.max(minConversationRailWidth, Math.round(width)));
}

export function adaptiveControlWidth(value: string, minimumCells: number, maximumCells: number) {
  const cells = [...value].reduce((total, character) => total + ((character.codePointAt(0) ?? 0) > 0xff ? 2 : 1), 0);
  return `${Math.min(maximumCells, Math.max(minimumCells, cells + 1))}ch`;
}

export function assistantMessageBody(content: string, nonce?: string | null): string {
  if (nonce) {
    for (const match of content.matchAll(/\n\s*(?:```json\s*)?\{/g)) {
      let rest = content.slice(match.index + match[0].length);
      const header: Record<string, unknown> = {};
      for (let field = 0; field < 3; field++) {
        const pair = rest.match(/^\s*("(?:[^"\\]|\\.)*")\s*:\s*("(?:[^"\\]|\\.)*"|\d+)\s*([,}])/s);
        if (!pair) break;
        try {
          const key = JSON.parse(pair[1]) as string;
          if (!["schemaVersion", "nonce", "reply"].includes(key)) break;
          header[key] = JSON.parse(pair[2]);
          if (header.schemaVersion === 1 && header.nonce === nonce) return content.slice(0, match.index).trimEnd();
        } catch { break; }
        rest = rest.slice(pair[0].length);
      }
    }
  }
  try {
    const value = JSON.parse(content) as Record<string, unknown>;
    if (nonce && value?.schemaVersion === 1 && value.nonce === nonce && typeof value.reply === "string") return value.reply;
    const keys = Object.keys(value).sort();
    const legacyKeys = ["classification", "nonce", "reply", "schemaVersion"];
    if (
      keys.length === legacyKeys.length
      && keys.every((key, index) => key === legacyKeys[index])
      && value.schemaVersion === 1
      && typeof value.nonce === "string"
      && value.classification !== null
      && typeof value.classification === "object"
      && typeof value.reply === "string"
      && value.reply.trim() !== ""
    ) {
      return value.reply;
    }
  } catch {
    return content;
  }
  return content;
}

function readConversationRailWidth() {
  const stored = window.localStorage.getItem(conversationRailWidthStorageKey);
  if (stored === null || stored.trim() === "") return defaultConversationRailWidth;
  const width = Number(stored);
  return Number.isFinite(width) ? clampConversationRailWidth(width) : defaultConversationRailWidth;
}

/** 尚未落地的运行。它们由对话区那张进行中卡片表达，不再进入下方的运行清单。 */
const activeRunStatuses = ["queued", "running", "paused", "terminating", "waiting_permission"];

function hasReplyEnded(run: AiAgentRun | null) {
  return Boolean(run && (["completed", "failed", "terminated", "interrupted"].includes(run.status)
    || run.events.some((event) => (!event.phase || event.phase === "reply")
      && (event.eventType === "turn.completed" || event.eventType === "turn.failed"))
    || run.events.some((event) => event.eventType === "harness.classification_started")));
}

function canSteerReply(run: AiAgentRun | null) {
  if (!run || run.executionTarget !== "model" || run.status !== "running") return false;
  const replyEvents = run.events.filter((event) => !event.phase || event.phase === "reply");
  return replyEvents.some((event) => event.eventType === "turn.started")
    && !hasReplyEnded(run);
}

function isSupportedMedia(file: File) {
  return ["image/png", "image/jpeg", "image/webp", "image/gif", "video/mp4", "video/quicktime", "video/webm"].includes(file.type);
}

function isBackgroundToolDiscoveryEvent(event: AiRunEvent) {
  return ["application_discovery.", "enterprise_tool_snapshot.", "tool_search."].some((prefix) =>
    event.eventType.startsWith(prefix),
  );
}

function neutralRunProgress() {
  return "正在处理…";
}

export function runDurationLabel(elapsedMs: number) {
  const totalSeconds = Math.max(0, Math.floor(elapsedMs / 1_000));
  const seconds = totalSeconds % 60;
  const totalMinutes = Math.floor(totalSeconds / 60);
  const minutes = totalMinutes % 60;
  const hours = Math.floor(totalMinutes / 60);
  return hours > 0
    ? `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`
    : `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

export function RunElapsedTime({ completedAt, startedAt }: { completedAt: number | null; startedAt: number }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (completedAt !== null) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [completedAt, startedAt]);
  const elapsedMs = Math.max(0, (completedAt ?? now) - startedAt);
  const duration = runDurationLabel(elapsedMs);
  const stateLabel = completedAt === null ? "运行中" : "用时";
  return <time aria-label={`${stateLabel} ${duration}`} className="ai-run-elapsed" dateTime={`PT${Math.floor(elapsedMs / 1_000)}S`}>{stateLabel} {duration}</time>;
}

export function visibleRunProgress(run: AiAgentRun) {
  if (run.status === "waiting_permission") return "等待你确认后继续";
  if (run.status === "paused") return "已暂停";
  if (run.status === "terminating") return "正在停止…";
  const event = [...run.events]
    .reverse()
    .find(
      (item) =>
        !isBackgroundToolDiscoveryEvent(item) &&
        !["run.queued", "harness.started", "harness.thread_ready", "thread.tokenUsage.updated"].includes(item.eventType),
    );
  if (!event) return neutralRunProgress();
  const progress = event.summary
    .replaceAll("模型 Harness", "模型")
    .replace("模型开始处理本轮请求", "正在理解你的请求…")
    .replace("模型开始分析", "正在分析…")
    .replace("模型完成分析", "分析完成")
    .replace("模型开始生成回复", "正在生成回复…")
    .replace("模型完成生成回复", "回复已生成")
    .replace("开始执行工作区命令", "正在执行工作区命令…")
    .replace("完成执行工作区命令", "命令执行完成");
  return progress;
}

export type QueuedFollowUp = {
  queueId: string;
  /** 排队时刻。服务端记下这条的时刻必定不早于它，失败重取快照时据此认出已落库的那条。 */
  queuedAt: number;
  conversationId: string;
  content: string;
  attachments: DraftAttachment[];
  referencedMaterials?: AiMaterialReference[];
  applicationContext: ApplicationContextSelection | null;
  selectedSkillId?: string | null;
  requiresApplicationResolution?: boolean;
  modelOverride: string | null;
  modelSelection?: ModelSelection | null;
  historyConsent?: ModelHistoryTransferConsent | null;
  reasoningEffort: AiReasoningEffort | null;
  executionTarget: "agent" | "model";
  permissionMode: StarterPermissionMode;
  planning?: boolean;
  status: "queued" | "sending" | "steering" | "deleting" | "failed";
  steeringRunId?: string;
  transcriptCommitted?: boolean;
  sentMessageId?: string;
  error?: string;
};

type PendingOutgoingMessage = {
  pendingId: string;
  draftKey: string;
  /** 新建对话尚未拿到编号时为空，建成后立即回填，失败则整条撤掉。 */
  conversationId: string | null;
  content: string;
  attachments: AiMessageAttachment[];
  referencedMaterials?: AiMaterialReference[];
  createdAt: number;
  recovery?: {
    create: PendingConversationCreate | null;
    attachments: DraftAttachment[];
    referencedMaterials?: AiMaterialReference[];
    applicationContext: ApplicationContextSelection | null;
    selectedSkillId: string | null;
    agentId: string | null;
    modelOverride: string | null;
    modelSelection?: ModelSelection | null;
    historyConsent?: ModelHistoryTransferConsent | null;
    reasoningEffort: AiReasoningEffort | null;
    permissionMode: StarterPermissionMode;
  planning?: boolean;
    file: WorkspaceFileItem | null;
  };
};

type PendingRegeneration = {
  conversationId: string;
  messageId: string;
  sourceRunId: string | null;
  content: string;
  submittedAt: number;
};

type SkillInvocationChoice = {
  kind: "skill";
  skillId: string;
  skillName: string;
  source: SkillConversationCatalogItem["source"];
  command: string;
  description: string;
};

type SlashInvocationChoice = SkillInvocationChoice;

function normalizedInvocationText(value: string): string {
  return value.trim().toLocaleLowerCase().replaceAll(" ", "");
}

function applicationInvocationQuery(value: string): string | null {
  const match = value.match(/^\/([^/\s]*)$/u);
  return match?.[1] ?? null;
}

function removeApplicationInvocationQuery(value: string): string {
  return value.replace(/^\/[^/\s]*$/u, "");
}

function skillInvocationChoices(
  query: string,
  skills: SkillConversationCatalogItem[],
): SkillInvocationChoice[] {
  const normalizedQuery = normalizedInvocationText(query);
  return skills.filter((skill) => {
    if (!normalizedQuery) return true;
    return normalizedInvocationText(`${skill.name} ${skill.description}`).includes(normalizedQuery);
  }).slice(0, 12).map((skill) => ({
    kind: "skill" as const,
    skillId: skill.skillId,
    skillName: skill.name,
    source: skill.source,
    command: `/${skill.name}`,
    description: skill.description || "本轮明确使用该技能",
  }));
}

const starterBusinesses = [
  {
    id: "search",
    name: "查资料",
    description: "查询项目、文件、制度和业务信息",
    prompt: "请基于当前工作区和已添加的上下文，查询并整理以下信息：",
  },
  {
    id: "write",
    name: "写材料",
    description: "撰写报告、方案、总结和正式文档",
    prompt: "请基于当前工作区和已添加的上下文，撰写以下材料：",
  },
  {
    id: "fill",
    name: "填报信息",
    description: "整理并填写表单、台账和业务数据",
    prompt: "请基于当前工作区和已添加的上下文，完成以下信息填报：",
  },
  {
    id: "advance",
    name: "推进事项",
    description: "拆解任务、跟踪进度并推动事项落地",
    prompt: "请基于当前工作区和已添加的上下文，推进以下事项：",
  },
] as const;

type StarterBusinessId = typeof starterBusinesses[number]["id"];
type StarterPermissionMode = "read-only" | "workspace-write" | AiApprovalMode;

export interface DraftAttachment {
  attachment: AiMessageAttachment;
  previewUrl: string;
}

interface PendingConversationCreate {
  title: string;
  conversationType: AiConversationType;
  mode: AiConversationMode;
  context: AiContext;
  projectIds?: string[];
  projectSelection: "editable" | "fixed";
  draftKey: string;
  selectedSkillId?: string;
  prefill?: string;
}


const agentPermissionOptions: Array<{ value: StarterPermissionMode; label: string; shortLabel: string; description: string; shortcut: string }> = [
  { value: "read-only", label: "工作区只读", shortLabel: "工作区只读", description: "读取和分析文件，不允许修改", shortcut: "1" },
  { value: "workspace-write", label: "运行前确认修改", shortLabel: "运行前确认修改", description: "确认后，本次运行可创建或编辑文件", shortcut: "2" },
];
const modelApprovalOptions: Array<{ value: StarterPermissionMode; label: string; shortLabel: string; description: string; shortcut: string }> = [
  { value: "request", label: "请求批准", shortLabel: "请求批准", description: "编辑外部文件和使用互联网时始终询问", shortcut: "1" },
  { value: "auto-review", label: "帮我批准", shortLabel: "帮我批准", description: "仅对检测到的风险操作请求批准", shortcut: "2" },
  { value: "full-access", label: "完全访问权限", shortLabel: "完全访问", description: "可不受限制地访问互联网和你电脑上的任何文件", shortcut: "3" },
];

export function modelApprovalOptionsForHost(hostPlatform: string | null | undefined) {
  return hostPlatform === "macos"
    ? modelApprovalOptions
    : modelApprovalOptions.filter((option) => option.value !== "full-access");
}

function isModelApprovalMode(value: StarterPermissionMode | null | undefined): value is AiApprovalMode {
  return value === "request" || value === "auto-review" || value === "full-access";
}

function conversationApprovalMode(conversation: AiConversation | null, runs: AiAgentRun[]): AiApprovalMode {
  if (isModelApprovalMode(conversation?.approvalMode)) return conversation.approvalMode;
  const latestModelRun = [...runs].reverse().find((run) => run.executionTarget === "model" && isModelApprovalMode(run.approvalMode));
  return isModelApprovalMode(latestModelRun?.approvalMode) ? latestModelRun.approvalMode : "request";
}

export function AiWorkbenchPage(props: Props) {
  const pageActive = usePageActive();
  /*
   * 当前对话上报给主界面，浏览器据此决定呈不呈现。离开对话页时报空——切走了就不该
   * 再看到上一个对话开出来的浏览器。
   */
  const reportConversation = props.aiBrowser?.onConversationChange;
  const platformUi = hostPlatformUi(props.hostPlatform);
  const [snapshot, setSnapshot] = useState<AiRuntimeSnapshot>(emptyAiSnapshot);
  const [snapshotLoaded, setSnapshotLoaded] = useState(false);
  const [snapshotError, setSnapshotError] = useState<string | null>(null);
  const catalog = useModelCatalog(snapshotLoaded);
  const snapshotRequest = useRef(0);
  const snapshotInFlight = useRef(false);
  // null 表示没有后续刷新；true 的目录核对需求优先于仅轮询本机状态的 false。
  const snapshotRefreshQueued = useRef<boolean | null>(null);
  const snapshotRefreshWaiters = useRef<Array<(value: AiRuntimeSnapshot | null) => void>>([]);
  /*
   * 已提交但尚未出现在快照里的用户消息。
   *
   * 发送要经过宿主命令、运行启动和一次快照往返，这期间界面上原本什么都没有，
   * 用户看着自己刚发的内容凭空消失。这些条目只用于呈现，落到快照里就撤掉。
   */
  const pageMounted = useRef(true);
  useEffect(() => { pageMounted.current = true; return () => { pageMounted.current = false; }; }, []);
  const flushConversationSession = useConversationSessionFlush();
  const [runtimeRevision, setRuntimeRevision] = useConversationSessionState("runtimeRevision", 0);
  const observedRuntimeRevision = useRef(runtimeRevision);
  const [pendingOutgoing, setPendingOutgoing] = useConversationSessionState<PendingOutgoingMessage[]>("pendingOutgoing", []);
  const [recoveredOutgoing, setRecoveredOutgoing] = useConversationSessionState<PendingOutgoingMessage[]>("recoveredOutgoing", []);
  const [pendingRegeneration, setPendingRegeneration] = useConversationSessionState<PendingRegeneration | null>("pendingRegeneration", null);
  const pendingOutgoingSequence = useRef(0);
  const workspaceRef = useRef(props.workspace);
  workspaceRef.current = props.workspace;
  const [selectedConversationId, setSelectedConversationId] = useConversationSessionState<string | null>("selectedConversationId", readLastViewedConversationId);
  useEffect(() => {
    if (!pageActive) return;
    reportConversation?.(selectedConversationId);
    // 离开对话页时报空：切走了就不该再看到上一个对话开出来的浏览器
    return () => reportConversation?.(null);
  }, [reportConversation, selectedConversationId, pageActive]);
  const [drafts, setDraftsState] = useConversationSessionState<Record<string, string>>("drafts", {});
  const [draftsRef] = useConversationSessionState("draftsRef", () => ({ current: drafts }));
  const setDrafts = useCallback((value: SetStateAction<Record<string, string>>) => {
    const next = typeof value === "function" ? value(draftsRef.current) : value;
    draftsRef.current = next;
    setDraftsState(next);
  }, []);
  const [editingMessageId, setEditingMessageId] = useState<string | null>(null);
  const [messageEditDraft, setMessageEditDraft] = useState("");
  const [queuedFollowUps, setQueuedFollowUps] = useConversationSessionState<QueuedFollowUp[]>("queuedFollowUps", []);
  const [initializedSelection, setInitializedSelection] = useConversationSessionState<string | null>("initializedSelection", null);
  const [starterAgentId, setStarterAgentId] = useConversationSessionState<string | null>("starterAgentId", null);
  const [starterBusinessId, setStarterBusinessId] = useConversationSessionState<StarterBusinessId | null>("starterBusinessId", null);
  const [starterFile, setStarterFile] = useConversationSessionState<WorkspaceFileItem | null>("starterFile", null);
  const [starterModelOverride, setStarterModelOverride] = useConversationSessionState<string | null>("starterModelOverride", null);
  const [starterConnectionId, setStarterConnectionId] = useConversationSessionState<string | null>("starterConnectionId", null);
  const [starterReasoningEffort, setStarterReasoningEffort] = useConversationSessionState<AiReasoningEffort | null>("starterReasoningEffort", null);
  const [starterPermissionMode, setStarterPermissionMode] = useConversationSessionState<StarterPermissionMode>("starterPermissionMode", "read-only");
  const effectiveStarterPermissionMode = starterPermissionMode;
  const starterPermissionSelectionInvalid = !(starterModelOverride ? modelApprovalOptionsForHost(props.hostPlatform) : agentPermissionOptions)
    .some((option) => option.value === effectiveStarterPermissionMode);
  const {status:windowsSandboxStatus,loading:windowsSandboxLoading,settingUp:windowsSandboxSettingUp} = useWindowsSandbox(props.hostPlatform);
  const [filePickerOpen, setFilePickerOpen] = useState(false);
  const [applicationContextSelections, setApplicationContextSelections] = useConversationSessionState<Record<string, ApplicationContextSelection | undefined>>("applicationContextSelections", {});
  const [conversationSkills, setConversationSkills] = useState<SkillConversationCatalogItem[]>([]);
  const [skillSelections, setSkillSelections] = useConversationSessionState<Record<string, string | undefined>>("skillSelections", {});
  const [draftMaterialGroups, setDraftMaterialGroups] = useConversationSessionState<Record<string, AiMaterialReference[]>>("draftMaterialGroups", {});
  const [draftAttachmentGroups, setDraftAttachmentGroups] = useConversationSessionState<Record<string, DraftAttachment[]>>("draftAttachmentGroups", {});
  const [confirmWriteSend, setConfirmWriteSend] = useState(false);
  const [actionBusy, setBusy] = useState(false);
  const [sendingDraftKeys, setSendingDraftKeys] = useConversationSessionState<Set<string>>("sendingDraftKeys", new Set());
  const [sendingDrafts] = useConversationSessionState("sendingDrafts", () => ({ current: new Set<string>() }));
  const [newDraftSequence] = useConversationSessionState("newDraftSequence", () => ({ current: 0 }));
  const [archivingIds, setArchivingIds] = useState<Set<string>>(new Set());
  const archivingConversations = useRef(new Set<string>());
  const latestArchiveRevision = useRef(0);
  useEffect(() => {
    const changed = (event: Event) => {
      const result = (event as CustomEvent<ConversationArchiveChange>).detail;
      latestArchiveRevision.current = Math.max(latestArchiveRevision.current, result.stateRevision);
      setSnapshot((current) => mergeArchivedConversations(current, result));
    };
    window.addEventListener(conversationArchiveChanged, changed);
    return () => window.removeEventListener(conversationArchiveChanged, changed);
  }, []);
  const [conversationInfoOpen, setConversationInfoOpen] = useState(false);
  const [conversationInfoDocked, setConversationInfoDocked] = useState(false);
  const [contextMenu, setContextMenu] = useState<ContextMenuState>(null);
  const [openProjectMenuKey, setOpenProjectMenuKey] = useState<string | null>(null);
  const [renamingConversation, setRenamingConversation] = useState<AiConversation | null>(null);
  const [conversationName, setConversationName] = useState("");
  const beginConversationRename = (item: AiConversation) => { setConversationName(item.title); setRenamingConversation(item); };
  const [pendingArchive, setPendingArchive] = useState<{ project: string; ids: string[] } | null>(null);
  const [pendingCreate, setPendingCreate] = useConversationSessionState<PendingConversationCreate | null>("pendingCreate", null);
  const [projectPickerOpen, setProjectPickerOpen] = useState(false);
  // The conversation area is where the work happens; the inspector can step aside on demand.
  const [savingDecisionMessageId, setSavingDecisionMessageId] = useState<string | null>(null);
  const [conversationRailWidth, setConversationRailWidth] = useState(readConversationRailWidth);
  const [conversationRailOpen, setConversationRailOpen] = useState(true);
  const [resizingConversationRail, setResizingConversationRail] = useState(false);
  const [followingLatestOutput, setFollowingLatestOutput] = useState(true);
  const [unseenOutput, setUnseenOutput] = useState(false);
  const [consumedLaunch] = useConversationSessionState("consumedLaunch", () => ({ current: null as number | null }));
  const messageList = useRef<HTMLDivElement>(null);
  const messageListUserScrollIntent = useRef(false);
  const draftBox = useRef<HTMLTextAreaElement | HTMLSpanElement>(null);
  const messageEditBox = useRef<HTMLTextAreaElement>(null);
  const mediaPicker = useRef<HTMLInputElement>(null);
  const chatStage = useRef<HTMLElement>(null);
  const conversationInfoDockedRef = useRef<boolean | null>(null);
  const reportMinWidth = props.aiBrowser?.onMinWidthChange;
  useEffect(() => {
    reportMinWidth?.(conversationMinWidth({ railWidth: conversationRailWidth, railOpen: conversationRailOpen }));
  }, [conversationRailOpen, conversationRailWidth, reportMinWidth]);
  const conversationRailWidthRef = useRef(conversationRailWidth);
  const conversationRailResizeStart = useRef<{ clientX: number; width: number } | null>(null);

  const draftKey = pendingCreate?.draftKey
    ?? (selectedConversationId
      ? `conversation:${selectedConversationId}`
      : "new:unassigned");
  const [currentDraftKey] = useConversationSessionState("currentDraftKey", () => ({ current: draftKey }));
  currentDraftKey.current = draftKey;
  const historyTransfer = useModelHistoryTransfer(draftKey);
  const busy = actionBusy || historyTransfer.busy || sendingDraftKeys.has(draftKey)
    || Boolean(selectedConversationId && archivingIds.has(selectedConversationId));
  const beginSending = (key: string) => {
    if (sendingDrafts.current.has(key)) return false;
    sendingDrafts.current.add(key);
    setSendingDraftKeys(new Set(sendingDrafts.current));
    return true;
  };
  const finishSending = (key: string) => {
    sendingDrafts.current.delete(key);
    setSendingDraftKeys(new Set(sendingDrafts.current));
  };
  const availableNewDraftKey = (key: string) => sendingDrafts.current.has(key)
    ? `${key}:${++newDraftSequence.current}` : key;
  const draft = drafts[draftKey] ?? "";
  const draftAttachments = draftAttachmentGroups[draftKey] ?? [];
  const applicationContext = applicationContextSelections[draftKey] ?? null;
  const selectedSkillId = skillSelections[draftKey] ?? null;
  const selectedConversationSkill = conversationSkills.find((skill) => skill.skillId === selectedSkillId) ?? null;
  const slashInvocationQuery = applicationInvocationQuery(draft);
  const slashInvocationChoices = slashInvocationQuery === null
    ? []
    : skillInvocationChoices(slashInvocationQuery, conversationSkills);
  const setDraft = useCallback((value: SetStateAction<string>) => {
    setDrafts((current) => {
      const existing = current[draftKey] ?? "";
      const next = typeof value === "function" ? value(existing) : value;
      return { ...current, [draftKey]: next };
    });
  }, [draftKey]);
  const setDraftAttachments = useCallback((value: SetStateAction<DraftAttachment[]>) => {
    setDraftAttachmentGroups((current) => {
      const existing = current[draftKey] ?? [];
      const next = typeof value === "function" ? value(existing) : value;
      return { ...current, [draftKey]: next };
    });
  }, [draftKey]);

  useEffect(() => {
    let active = true;
    setConversationSkills([]);
    setSkillSelections({});
    void rpc<SkillConversationCatalogResult>("skill/conversationCatalog", {}).then((result) => {
      if (!Array.isArray(result.skills)) throw new Error("skill_catalog_invalid");
      if (active) setConversationSkills(result.skills);
    }).catch((error) => {
      if (active) props.onMessage(`技能列表读取失败：${aiErrorMessage(error)}`, "error");
    });
    return () => { active = false; };
  }, [props.profileId]);

  useEffect(() => {
    if (!pageActive) return;
    const stage = chatStage.current;
    if (!stage || typeof ResizeObserver === "undefined") return;
    const updateLayout = (width: number) => {
      const docked = width > conversationInfoDockWidth;
      setConversationInfoDocked(docked);
      if (conversationInfoDockedRef.current === null || conversationInfoDockedRef.current !== docked) {
        setConversationInfoOpen(docked);
      }
      conversationInfoDockedRef.current = docked;
    };
    updateLayout(stage.getBoundingClientRect().width);
    const observer = new ResizeObserver(([entry]) => updateLayout(entry.contentRect.width));
    observer.observe(stage);
    return () => observer.disconnect();
  }, [pageActive]);

  const applyConversationRailWidth = useCallback((width: number, persist: boolean) => {
    const next = clampConversationRailWidth(width);
    conversationRailWidthRef.current = next;
    setConversationRailWidth(next);
    if (persist) window.localStorage.setItem(conversationRailWidthStorageKey, String(next));
  }, []);

  useEffect(() => {
    if (!resizingConversationRail) return;
    const move = (event: PointerEvent) => {
      const start = conversationRailResizeStart.current;
      if (!start) return;
      applyConversationRailWidth(start.width + event.clientX - start.clientX, false);
    };
    const finish = () => {
      window.localStorage.setItem(conversationRailWidthStorageKey, String(conversationRailWidthRef.current));
      conversationRailResizeStart.current = null;
      setResizingConversationRail(false);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
    };
  }, [applyConversationRailWidth, resizingConversationRail]);

  const startConversationRailResize = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    conversationRailResizeStart.current = { clientX: event.clientX, width: conversationRailWidthRef.current };
    setResizingConversationRail(true);
    event.preventDefault();
  };

  const resizeConversationRailWithKeyboard = (event: React.KeyboardEvent<HTMLDivElement>) => {
    let next: number | null = null;
    if (event.key === "ArrowLeft") next = conversationRailWidthRef.current - conversationRailKeyboardStep;
    if (event.key === "ArrowRight") next = conversationRailWidthRef.current + conversationRailKeyboardStep;
    if (event.key === "Home") next = minConversationRailWidth;
    if (event.key === "End") next = maxConversationRailWidth;
    if (next === null) return;
    event.preventDefault();
    applyConversationRailWidth(next, true);
  };
  // 普通输入仍由 textarea 承接；选中应用或技能后改用行内编辑结构并随内容自然长高。
  // textarea 按内容调整高度，上限交给 CSS 的 max-height。
  useLayoutEffect(() => {
    const node = draftBox.current;
    if (!(node instanceof HTMLTextAreaElement)) return;
    node.style.height = "auto";
    node.style.height = `${node.scrollHeight + 1}px`;
    const output = messageList.current;
    if (output && followingLatestOutputRef.current) output.scrollTop = output.scrollHeight;
  }, [draft]);
  useLayoutEffect(() => {
    const node = messageEditBox.current;
    if (!node || !editingMessageId) return;
    node.style.height = "auto";
    node.style.height = `${node.scrollHeight}px`;
  }, [editingMessageId, messageEditDraft]);
  useEffect(() => {
    setEditingMessageId(null);
    setMessageEditDraft("");
    setStarterBusinessId(null);
    setStarterFile(null);
  }, [props.workspace?.workspaceId]);
  useEffect(() => {
    setEditingMessageId(null);
    setMessageEditDraft("");
  }, [selectedConversationId]);

  const refresh = async (refreshNativeCatalog = true): Promise<AiRuntimeSnapshot | null> => {
    /*
     * 已有刷新在途时不另发一次，改为排到它后面，并把本次调用挂在等待队列上。
     * 从前这里直接返回，调用方的 await 立刻兑现却什么也没拿到——发送完成后据此
     * 判断"消息是否已被服务端记下"，判断的其实是刷新前的旧快照。
     */
    if (snapshotInFlight.current) {
      snapshotRefreshQueued.current = snapshotRefreshQueued.current === true || refreshNativeCatalog;
      return new Promise<AiRuntimeSnapshot | null>((resolve) => { snapshotRefreshWaiters.current.push(resolve); });
    }
    snapshotInFlight.current = true;
    const requestedWorkspace = workspaceRef.current;
    const request = ++snapshotRequest.current;
    let delivered: AiRuntimeSnapshot | null = null;
    try {
      const result = await loadAiSnapshot(refreshNativeCatalog);
      if (request !== snapshotRequest.current || requestedWorkspace?.workspaceId !== workspaceRef.current?.workspaceId) return null;
      if (result.stateRevision < latestArchiveRevision.current) {
        snapshotRefreshQueued.current = true;
        return null;
      }
      delivered = result;
      setSnapshot(result);
      setSnapshotLoaded(true);
      setSnapshotError(null);
      setSelectedConversationId((current) => {
        const currentConversation = result.conversations.find((item) => item.conversationId === current);
        if (currentConversation && !currentConversation.archived) return current;
        const remembered = readLastViewedConversationId();
        const rememberedConversation = result.conversations.find((item) => item.conversationId === remembered);
        if (rememberedConversation && !rememberedConversation.archived) return remembered;
        window.localStorage.removeItem(lastViewedConversationStorageKey);
        return null;
      });
    } catch (error) {
      setSnapshotError(aiErrorMessage(error));
    } finally {
      snapshotInFlight.current = false;
      if (snapshotRefreshQueued.current !== null) {
        const queuedNativeCatalog = snapshotRefreshQueued.current;
        snapshotRefreshQueued.current = null;
        void refresh(queuedNativeCatalog);
      } else {
        const waiters = snapshotRefreshWaiters.current;
        snapshotRefreshWaiters.current = [];
        for (const resolve of waiters) resolve(delivered);
      }
    }
    return delivered;
  };

  useEffect(() => {
    if (observedRuntimeRevision.current === runtimeRevision) return;
    observedRuntimeRevision.current = runtimeRevision;
    void refresh();
  }, [runtimeRevision]);

  useEffect(() => {
    void refresh();
    const changed = () => { void refresh(); };
    window.addEventListener(aiRuntimeChangedEvent, changed);
    return () => { snapshotRequest.current += 1; window.removeEventListener(aiRuntimeChangedEvent, changed); };
  }, [props.workspace?.workspaceId]);
  const [consumedPreferredConversation, setConsumedPreferredConversation] = useConversationSessionState<string | null>("consumedPreferredConversation", null);
  useEffect(() => {
    if (!props.preferredConversationId) setConsumedPreferredConversation(null);
    if (props.preferredConversationId && props.preferredConversationId !== consumedPreferredConversation && snapshot.conversations.some((item) => item.conversationId === props.preferredConversationId)) {
      setPendingCreate(null);
      setSelectedConversationId(props.preferredConversationId);
      setConsumedPreferredConversation(props.preferredConversationId);
      props.onPreferredConversationConsumed?.(props.preferredConversationId);
    }
  }, [props.preferredConversationId, snapshot.conversations.length]);
  useEffect(() => {
    if (selectedConversationId) window.localStorage.setItem(lastViewedConversationStorageKey, selectedConversationId);
  }, [selectedConversationId]);
  useEffect(() => {
    if (!snapshotLoaded || selectedConversationId || pendingCreate || props.preferredConversationId || props.launchRequest) return;
    setPendingCreate((current) => current ?? {
      title: "新对话",
      conversationType: "direct",
      mode: "discuss",
      context: unclassifiedConversationContext(),
      projectSelection: "editable",
      draftKey: "new:unassigned",
    });
  }, [snapshotLoaded, selectedConversationId, pendingCreate, props.preferredConversationId, props.launchRequest, props.workspace]);
  const activeRunIdsKey = snapshot.runs.filter((run) => run.threadWriterActive || activeRunStatuses.includes(run.status)).map((run) => run.runId).join("|");
  useEffect(() => {
    if (!activeRunIdsKey) return;
    const runIds = activeRunIdsKey.split("|");
    let cancelled = false;
    let timer: number;
    let reportedFailure = false;
    const poll = async () => {
      try {
        const activities = await Promise.all(Array.from(
          { length: Math.ceil(runIds.length / 32) },
          (_, index) => loadAiActivitySnapshot(runIds.slice(index * 32, (index + 1) * 32)),
        ));
        if (cancelled || activities.some((activity) => activity.stateRevision < latestArchiveRevision.current)) return;
        const missing = new Set(activities.flatMap((activity) => activity.missingRunIds));
        const conversations = activities.flatMap((activity) => activity.conversations);
        const messages = activities.flatMap((activity) => activity.messages);
        const runs = activities.flatMap((activity) => activity.runs);
        const permissionRequests = activities.flatMap((activity) => activity.permissionRequests);
        setSnapshot((current) => ({
          ...current,
          stateRevision: Math.max(current.stateRevision, ...activities.map((activity) => activity.stateRevision)),
          conversations: mergeSnapshotEntities(current.conversations, conversations, (item) => item.conversationId),
          messages: mergeSnapshotEntities(current.messages, messages, (item) => item.messageId),
          runs: mergeSnapshotEntities(current.runs, runs, (item) => item.runId, missing),
          permissionRequests: mergeSnapshotEntities(current.permissionRequests, permissionRequests, (item) => item.permissionRequestId),
        }));
        if (missing.size || runs.some((run) => !activeRunStatuses.includes(run.status))) void refresh();
      } catch (error) {
        if (!cancelled && !reportedFailure) {
          reportedFailure = true;
          props.onMessage(aiErrorMessage(error), "error");
        }
      } finally {
        if (!cancelled) timer = window.setTimeout(() => void poll(), activeRunRefreshInterval);
      }
    };
    timer = window.setTimeout(() => void poll(), activeRunRefreshInterval);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [activeRunIdsKey, props.workspace?.workspaceId]);

  const conversations = useMemo(() => snapshot.conversations
    .filter((item) => !item.archived)
    .sort((left, right) => right.updatedAt - left.updatedAt), [snapshot.conversations, props.workspace?.workspaceId]);
  const messagesByConversation = useMemo(() => {
    const groups = new Map<string, AiRuntimeSnapshot["messages"]>();
    for (const message of snapshot.messages) {
      const group = groups.get(message.conversationId);
      if (group) group.push(message); else groups.set(message.conversationId, [message]);
    }
    for (const group of groups.values()) group.sort((left, right) => left.createdAt - right.createdAt);
    return groups;
  }, [snapshot.messages]);
  const persistedConversation = snapshot.conversations.find((item) => item.conversationId === selectedConversationId) ?? null;
  const conversation = pendingCreate ? null : persistedConversation;
  /*
   * 新建对话期间，上一个选中对话的消息、运行和排队内容都不属于眼前这段对话。
   * 起始状态与进行中的对话共用同一个消息区，这里不挡住，旧对话的内容就会混进新对话里。
   */
  const viewedConversationId = pendingCreate ? null : selectedConversationId;
  const messages = viewedConversationId ? messagesByConversation.get(viewedConversationId) ?? [] : [];
  useEffect(() => {
    setPendingOutgoing((current) => {
      const remaining = current.filter((item) => !snapshot.messages.some((message) => message.role === "user" && message.messageId === item.pendingId && (!item.conversationId || message.conversationId === item.conversationId)));
      return remaining.length === current.length ? current : remaining;
    });
  }, [snapshot.messages]);
  const submissionsById = useMemo(() => new Map((snapshot.messageSubmissions ?? []).map((item) => [item.clientMessageId, item])), [snapshot.messageSubmissions]);
  const pendingOutgoingInView = useMemo(() => pendingOutgoing.filter((item) => (
    pendingCreate ? item.conversationId === null && item.draftKey === draftKey : item.conversationId === selectedConversationId
  )), [pendingOutgoing, selectedConversationId, pendingCreate, draftKey]);
  /* 受理后在落库前失败的发送不留在对话正文里，改在输入框上方给出原因与恢复入口。 */
  const failedOutgoing = useMemo(() => pendingOutgoingInView.filter((item) => submissionsById.get(item.pendingId)?.status === "failed"), [pendingOutgoingInView, submissionsById]);
  const visiblePendingOutgoing = useMemo(() => pendingOutgoingInView.filter((item) => submissionsById.get(item.pendingId)?.status !== "failed"), [pendingOutgoingInView, submissionsById]);
  /* 宿主已受理、正在后台准备执行的发送。期间对话还没有运行，但同一对话的后续内容必须排队。 */
  const conversationSubmitting = Boolean(selectedConversationId && !pendingCreate && pendingOutgoing.some((item) => item.conversationId === selectedConversationId
    && submissionsById.get(item.pendingId)?.status === "preparing"));
  const anySubmissionPreparing = (snapshot.messageSubmissions ?? []).some((item) => item.status === "preparing");
  useEffect(() => {
    if (!anySubmissionPreparing) return;
    const timer = window.setInterval(() => { void refresh(false); }, submissionRefreshInterval);
    return () => window.clearInterval(timer);
  }, [anySubmissionPreparing, props.workspace?.workspaceId]);
  const visiblePendingRegeneration = pendingRegeneration && pendingRegeneration.conversationId === viewedConversationId
    && messages.some((message) => message.messageId === pendingRegeneration.messageId
      && message.runId === pendingRegeneration.sourceRunId)
    ? pendingRegeneration : null;
  const visibleSubmittedFollowUps = useMemo(() => queuedFollowUps.filter((item) => item.conversationId === viewedConversationId
    && item.transcriptCommitted
    && !snapshot.messages.some((message) => message.messageId === (item.sentMessageId ?? item.queueId))),
  [queuedFollowUps, viewedConversationId, snapshot.messages]);
  const visibleMessages = useMemo(() => {
    const persisted = visiblePendingRegeneration
      ? messages.slice(0, messages.findIndex((message) => message.messageId === visiblePendingRegeneration.messageId) + 1)
        .map((message) => message.messageId === visiblePendingRegeneration.messageId
          ? { ...message, content: visiblePendingRegeneration.content, runId: null }
          : message)
      : messages;
    const outgoing = visiblePendingOutgoing.map((item) => ({
      messageId: item.pendingId,
      conversationId: item.conversationId ?? "",
      role: "user" as const,
      content: item.content,
      attachments: item.attachments,
      referencedMaterials: item.referencedMaterials,
      runId: null,
      createdAt: item.createdAt,
    }));
    const submitted = visibleSubmittedFollowUps.map((item) => ({
      messageId: item.sentMessageId ?? item.queueId,
      conversationId: item.conversationId,
      role: "user" as const,
      content: item.content,
      attachments: item.attachments.map((attachment) => attachment.attachment),
      referencedMaterials: item.referencedMaterials,
      runId: item.steeringRunId ?? null,
      createdAt: item.queuedAt,
    }));
    return [...persisted, ...outgoing, ...submitted];
  }, [messages, visiblePendingOutgoing, visibleSubmittedFollowUps, visiblePendingRegeneration]);
  const [renderedMessageCount, setRenderedMessageCount] = useState(100);
  const historyScrollRestore = useRef<{ height: number; top: number } | null>(null);
  const pendingMessageJump = useRef<string | null>(null);
  const renderedMessages = visibleMessages.slice(-renderedMessageCount);
  useEffect(() => { setRenderedMessageCount(100); historyScrollRestore.current = null; pendingMessageJump.current = null; }, [viewedConversationId]);
  useLayoutEffect(() => {
    const root = messageList.current;
    if (!root) return;
    if (pendingMessageJump.current) {
      const target = root.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(pendingMessageJump.current)}"]`);
      if (target) root.scrollTop += target.getBoundingClientRect().top - root.getBoundingClientRect().top - 28;
      pendingMessageJump.current = null;
    } else if (historyScrollRestore.current) {
      root.scrollTop = historyScrollRestore.current.top + root.scrollHeight - historyScrollRestore.current.height;
      historyScrollRestore.current = null;
    }
  }, [renderedMessageCount]);
  const conversationRuns = useMemo(() => snapshot.runs
    .filter((run) => run.conversationId === viewedConversationId && !run.superseded)
    .sort((left, right) => left.startedAt - right.startedAt), [snapshot.runs, viewedConversationId]);
  const handoff = conversation?.applicationHandoff;
  const handoffUserId = handoff?.userMessageId ?? (handoff ? messages.find((message) => message.role === "user" && !message.runId)?.messageId : undefined);
  const handoffAssistantId = handoff?.assistantMessageId ?? (handoff ? messages.find((message) => message.role === "assistant" && !message.runId)?.messageId : undefined);
  const rememberedApprovalMode = conversationApprovalMode(conversation, conversationRuns);
  const latestEditableUserMessageId = [...messages].reverse().find((message) =>
    message.role === "user" && conversationRuns.some((run) => run.inputMessageId === message.messageId && !run.superseded))?.messageId ?? null;
  const activeRun = conversationRuns.find((run) => activeRunStatuses.includes(run.status) && run.status !== "queued")
    ?? conversationRuns.find((run) => run.status === "queued") ?? null;
  const activeImageGenerationProgress = activeRun ? imageGenerationProgress(activeRun) : null;
  const waitingForRunFeedback = (visiblePendingOutgoing.length > 0 || Boolean(visiblePendingRegeneration)) && !activeRun;
  const threadDraining = conversationRuns.some((run) => run.threadWriterActive && !activeRunStatuses.includes(run.status));
  const selectableSlashInvocationChoices = activeRun ? [] : slashInvocationChoices;
  const latestConversationRun = conversationRuns.reduce<AiAgentRun | null>((latest, run) => !latest || run.startedAt > latest.startedAt ? run : latest, null);
  const pendingApproval = activeRun?.permissionRequestId
    ? snapshot.permissionRequests.find((item) => item.permissionRequestId === activeRun.permissionRequestId && item.status === "pending") ?? null
    : null;
  const applicationApprovalsByRun = useMemo(() => {
    const groups = new Map<string, AiPermissionRequest[]>();
    for (const request of snapshot.permissionRequests) {
      if (!request.applicationWrite || request.status === "pending") continue;
      const group = groups.get(request.runId);
      if (group) group.push(request); else groups.set(request.runId, [request]);
    }
    for (const group of groups.values()) group.sort((left, right) => left.requestedAt - right.requestedAt);
    return groups;
  }, [snapshot.permissionRequests]);
  const conversationStarted = Boolean(conversation && (visibleMessages.length > 0 || activeRun || conversationSubmitting || pendingOutgoingInView.length > 0));
  /*
   * 消息区显示对话记录还是起始引导。新建对话点了发送、对话编号还没回来时，conversation 仍为空，
   * 但已经有一条本地消息要显示——这一刻就得切到对话记录，之后对话建成也不再换结构。
   */
  const threadVisible = visibleMessages.length > 0 || Boolean(activeRun);
  const selectedAgent = snapshot.agents.find((agent) => agent.agentId === starterAgentId) ?? null;
  const selectedModel = catalogModel(catalog.models, starterModelOverride, starterConnectionId);
  const selectedModelReasoningEfforts = selectedModel?.supportedReasoningEfforts ?? [];
  const starterReasoningSelectionInvalid = Boolean(
    selectedModel && starterReasoningEffort && !selectedModelReasoningEfforts.includes(starterReasoningEffort),
  );
  const effectiveStarterReasoningEffort = starterReasoningSelectionInvalid
    ? null
    : selectedModelReasoningEfforts.length
      ? (starterReasoningEffort
      ? starterReasoningEffort
      : selectedModel?.defaultReasoningEffort ?? null)
      : null;
  const modelSandboxReady = props.hostPlatform === "macos"
    || (props.hostPlatform === "windows" && windowsSandboxStatus?.status === "ready");
  const selectedRuntimeReady = Boolean(starterModelOverride && selectedModel) && modelSandboxReady;
  const starterAgent = snapshot.agents.find((agent) => agent.agentId === starterAgentId) ?? null;
  const starterModel = selectedModel;
  const modelSelection: ModelSelection | null = starterConnectionId && starterModelOverride
    ? { connectionId: starterConnectionId, modelId: starterModelOverride, reasoningEffort: effectiveStarterReasoningEffort }
    : null;
  const obtainHistoryConsent = async (selection: ModelSelection | null | undefined, conversationId: string | null): Promise<ModelHistoryTransferConsent | null | false> => {
    try { const approval = await historyTransfer.approve(selection, conversationId); return approval.allowed ? approval.consent : false; }
    catch (reason) { props.onMessage(aiErrorMessage(reason), "error"); return false; }
  };
  const starterRuntimeReady = Boolean(starterModelOverride && starterModel) && modelSandboxReady;
  const showWindowsSandboxGate = props.hostPlatform === "windows"
    && Boolean(starterModelOverride)
    && windowsSandboxStatus?.status !== "ready";
  const selectedStarterProjectId = pendingCreate?.context.projectId ?? conversation?.context.projectId ?? null;
  const selectedStarterProject = props.projects.find((project) => project.projectId === selectedStarterProjectId) ?? null;
  const starterProjectEditable = pendingCreate?.projectSelection === "editable";
  const starterPlaceholder = starterProjectEditable ? "描述任务、补充约束或粘贴内容…" : "输入问题、补充约束或下一步要求…";
  const conversationProjectId = conversation?.context.projectId ?? null;
  const conversationProject = props.projects.find((project) => project.projectId === conversationProjectId) ?? null;
  const conversationInfoContext = conversation
    ? { ...conversation.context, projectId: conversationProjectId, projectName: conversationProject?.displayName ?? conversation.context.projectName }
    : null;
  const permissionWorkspaceId = pendingCreate?.context.workspaceId ?? conversation?.context.workspaceId ?? null;
  const permissionWorkspace = (props.workspaces ?? (props.workspace ? [props.workspace] : []))
    .find((item) => item.workspaceId === permissionWorkspaceId && item.workspaceKind === "local");
  const initialMaterialContext = pendingCreate?.context ?? (!conversationStarted ? conversation?.context : null);
  const draftMaterials = draftMaterialGroups[draftKey] ?? (initialMaterialContext ? materialReferences(initialMaterialContext).map((item) => ({ ...item, workspaceId: item.workspaceId ?? initialMaterialContext.workspaceId })) : []);
  const setDraftMaterials = (materials: AiMaterialReference[]) => setDraftMaterialGroups((current) => ({ ...current, [draftKey]: materials }));
  const effectiveModel = selectedModel ?? starterModel;
  const canAttachMedia = Boolean(starterModelOverride && effectiveModel?.inputModalities.includes("image"));
  const canAttachVideo = canAttachMedia;
  const mediaUnavailableReason = starterModelOverride
    ? "当前模型不在可用目录中，或目录未声明支持图片输入。"
    : "请先选择支持图片的模型。";
  const mediaDescription = canAttachMedia
    ? canAttachVideo
      ? "图片原样发送；60 秒内短视频按关键帧发送"
      : "图片作为图像随本轮发送给 Codex"
    : mediaUnavailableReason;
  const activeRunProgress = activeRun
    ? visibleRunProgress(activeRun)
    : neutralRunProgress();
  const activeRunStartedAt = activeRun?.startedAt ?? visiblePendingOutgoing.at(-1)?.createdAt ?? visiblePendingRegeneration?.submittedAt ?? null;
  const canSteerActiveReply = canSteerReply(activeRun) && !pendingApproval;
  const supplementTarget = activeRun ?? latestConversationRun;
  const supplementUnavailableReason = pendingApproval ? "请先处理审批"
    : conversationSubmitting ? "等待当前消息开始执行"
    : canSteerActiveReply || hasReplyEnded(supplementTarget) ? null
    : activeRun?.status === "paused" ? "当前回复已暂停，请先恢复"
    : activeRun?.status === "terminating" ? "等待当前回复停止"
    : activeRun?.executionTarget === "agent" ? "当前执行不支持补充，内容将按队列发送"
    : "等待当前回复开始";
  const conversationHasPendingFollowUps = queuedFollowUps.some((item) => item.conversationId === conversation?.conversationId);
  const conversationQueuedFollowUps = queuedFollowUps.filter((item) => item.conversationId === conversation?.conversationId && !item.transcriptCommitted);
  /* 队列里还有等待派发或正在派发的内容时，新输入排在它们后面，不能抢先直接发送。 */
  const followUpsAhead = conversationQueuedFollowUps.some((item) => item.status === "queued" || item.status === "sending");
  const mustQueue = conversationSubmitting || followUpsAhead;
  useEffect(() => {
    const recordedIds = new Set(snapshot.messages.map((message) => message.messageId));
    setQueuedFollowUps((current) => {
      const next = current.filter((item) => !recordedIds.has(item.sentMessageId ?? item.queueId));
      return next.length === current.length && next.every((item, index) => item === current[index]) ? current : next;
    });
  }, [snapshot.messages, snapshot.runs]);
  const setOutputFollowing = useCallback((following: boolean) => {
    followingLatestOutputRef.current = following;
    setFollowingLatestOutput(following);
    if (following) setUnseenOutput(false);
  }, []);
  const markMessageListUserScroll = useCallback(() => {
    messageListUserScrollIntent.current = true;
  }, []);

  /*
   * 导航条的数据：每一次发问配上它得到的回复。回复按运行标识配对，而不是按"下一条
   * 助手消息"——一次发问可能被重发，中间还会插入审批卡片，按位置取会配错。
   */
  const [activeNavigatorMessageId, setActiveNavigatorMessageId] = useState<string | null>(null);
  const navigatorUpdateFrame = useRef<number | null>(null);
  const navigatorMessages: NavigatorMessage[] = useMemo(() => messages
    .filter((message) => message.role === "user")
    .map((message) => ({
      messageId: message.messageId,
      content: message.messageId === visiblePendingRegeneration?.messageId
        ? visiblePendingRegeneration.content : message.content,
      reply: message.messageId === visiblePendingRegeneration?.messageId ? undefined : message.runId
        ? messages.find((candidate) => candidate.role === "assistant" && candidate.runId === message.runId)?.content
        : undefined,
    })), [messages, visiblePendingRegeneration]);

  const jumpToMessage = useCallback((messageId: string) => {
    const root = messageList.current;
    const target = root?.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(messageId)}"]`);
    if (!root) return;
    if (!target) {
      const index = visibleMessages.findIndex((message) => message.messageId === messageId);
      if (index >= 0) { markMessageListUserScroll(); pendingMessageJump.current = messageId; setRenderedMessageCount(Math.max(100, visibleMessages.length - index)); setActiveNavigatorMessageId(messageId); }
      return;
    }
    // 跳转是员工自己发起的，此后不该再被自动滚到底部抢走位置
    markMessageListUserScroll();
    root.scrollTo({ top: root.scrollTop + target.getBoundingClientRect().top - root.getBoundingClientRect().top - 28, behavior: "smooth" });
    setActiveNavigatorMessageId(messageId);
  }, [markMessageListUserScroll, visibleMessages]);

  const updateNavigatorFromViewport = useCallback(() => {
    const root = messageList.current;
    if (!root || !navigatorMessages.length) {
      setActiveNavigatorMessageId(null);
      return;
    }
    if (root.scrollHeight - root.scrollTop - root.clientHeight <= latestOutputThreshold) {
      setActiveNavigatorMessageId(navigatorMessages.at(-1)!.messageId);
      return;
    }
    const readingLine = root.getBoundingClientRect().top + root.clientHeight / 2;
    const messageIds = new Set(navigatorMessages.map((message) => message.messageId));
    let current = navigatorMessages[0].messageId;
    for (const node of root.querySelectorAll<HTMLElement>(".ai-message.user[data-message-id]")) {
      const messageId = node.dataset.messageId;
      if (!messageId || !messageIds.has(messageId)) continue;
      if (node.getBoundingClientRect().top > readingLine) break;
      current = messageId;
    }
    setActiveNavigatorMessageId(current);
  }, [navigatorMessages]);
  const scheduleNavigatorUpdate = useCallback(() => {
    if (navigatorUpdateFrame.current !== null) return;
    navigatorUpdateFrame.current = window.requestAnimationFrame(() => {
      navigatorUpdateFrame.current = null;
      updateNavigatorFromViewport();
    });
  }, [updateNavigatorFromViewport]);
  useLayoutEffect(() => {
    scheduleNavigatorUpdate();
    return () => {
      if (navigatorUpdateFrame.current !== null) window.cancelAnimationFrame(navigatorUpdateFrame.current);
      navigatorUpdateFrame.current = null;
    };
  }, [selectedConversationId, navigatorMessages, visibleMessages.length, activeRun?.partialOutput, conversationInfoOpen, conversationInfoDocked, conversationRailOpen, conversationRailWidth, scheduleNavigatorUpdate]);
  useEffect(() => {
    window.addEventListener("resize", scheduleNavigatorUpdate);
    return () => window.removeEventListener("resize", scheduleNavigatorUpdate);
  }, [scheduleNavigatorUpdate]);

  const followingLatestOutputRef = useRef(true);
  const handleMessageListScroll = useCallback(() => {
    if (!pageActive) return;
    const node = messageList.current;
    if (!node) return;
    scheduleNavigatorUpdate();
    const following = node.scrollHeight - node.scrollTop - node.clientHeight <= latestOutputThreshold;
    if (following || messageListUserScrollIntent.current) setOutputFollowing(following);
    messageListUserScrollIntent.current = false;
  }, [scheduleNavigatorUpdate, setOutputFollowing, pageActive]);
  useLayoutEffect(() => {
    if (!pageActive) return;
    const node = messageList.current;
    if (node && followingLatestOutputRef.current) node.scrollTop = node.scrollHeight;
    scheduleNavigatorUpdate();
  },[pageActive,scheduleNavigatorUpdate]);
  const handleMessageListKeyDown = useCallback((event: React.KeyboardEvent<HTMLDivElement>) => {
    if (["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "].includes(event.key)) {
      markMessageListUserScroll();
    }
  }, [markMessageListUserScroll]);
  const returnToLatestOutput = useCallback(() => {
    const node = messageList.current;
    if (!node) return;
    setOutputFollowing(true);
    node.scrollTo({ top: node.scrollHeight, behavior: "auto" });
  }, [setOutputFollowing]);
  const linkedConversationPath = useCallback((href: string) => conversation
    ? workspaceRelativePath(href, conversation.context.workspaceRoot)
    : null, [conversation]);
  const linkedConversationFile = useCallback((href: string) => {
    const relativePath = linkedConversationPath(href);
    return relativePath ? props.files.find((file) => workspacePathsEqual(file.relativePath, relativePath, conversation?.context.workspaceRoot ?? "")) ?? null : null;
  }, [conversation, linkedConversationPath, props.files]);
  const openConversationLink = useCallback(async (href: string) => {
    if (!conversation) return;
    const relativePath = linkedConversationPath(href);
    if (!relativePath) return;
    const file = linkedConversationFile(href);
    if (isPreviewablePath(relativePath)) {
      if (!conversation.context.workspaceId) {
        props.onMessage("无法在文件页打开：当前会话未绑定工作区。", "error");
        return;
      }
      if (file && isPreviewableFile(file)) {
        props.onOpenFile?.(file.localFileId, conversation.context.workspaceId);
        return;
      }
      try {
        const opened = await props.onOpenWorkspacePath?.(relativePath, conversation.context.workspaceId);
        if (!opened) props.onMessage("无法在文件页打开：文件不存在或尚未完成索引。", "error");
      } catch (error) { props.onMessage(`无法打开文件：${String(error)}`, "error"); }
      return;
    }
    if (isImagePath(relativePath)) {
      if (conversation.context.teamSpaceId) {
        props.onMessage("云端文件请在文件页预览。", "error");
        return;
      }
      if (!platformUi) {
        props.onMessage("当前平台不支持打开图片。", "error");
        return;
      }
      void invoke("workspace_open_item", {
        localRoot: conversation.context.workspaceRoot,
        relativePath,
        openMode: "default",
      }).catch((error) => props.onMessage(`无法打开图片：${String(error)}`, "error"));
      return;
    }
    if (conversation.context.teamSpaceId) {
      props.onMessage("云端文件请在文件页预览。", "error");
      return;
    }
    if (!platformUi) {
      props.onMessage("当前平台不支持文件定位。", "error");
      return;
    }
    void invoke("workspace_open_item", {
      localRoot: conversation.context.workspaceRoot,
      relativePath,
      openMode: "finder",
    }).catch((error) => props.onMessage(`无法在${platformUi.fileManagerName}中显示：${String(error)}`, "error"));
  }, [conversation, linkedConversationFile, linkedConversationPath, platformUi, props.onMessage, props.onOpenFile, props.onOpenWorkspacePath]);
  const canOpenConversationLink = useCallback((href: string) => linkedConversationPath(href) !== null, [linkedConversationPath]);
  const openConversationLinkContextMenu = useCallback((event: React.MouseEvent, href: string) => {
    if (!conversation) return;
    const relativePath = linkedConversationPath(href);
    if (!relativePath) return;
    const local = !conversation.context.teamSpaceId;
    const openDefault = () => {
      void invoke("workspace_open_item", {
        localRoot: conversation.context.workspaceRoot,
        relativePath,
        openMode: "default",
      }).catch((error) => props.onMessage(`无法打开文件：${String(error)}`, "error"));
    };
    const items: ContextMenuItem[] = [{
      label: "打开文件",
      onSelect: () => isPreviewablePath(relativePath) || !local || !platformUi ? void openConversationLink(href) : openDefault(),
    }];
    if (local) {
      if (platformUi) items.push({ label: "默认程序打开", onSelect: openDefault });
      items.push(
        { separator: true },
        {
        label: "另存为…",
        onSelect: () => {
          void invoke<boolean>("workspace_save_copy", {
            localRoot: conversation.context.workspaceRoot,
            relativePath,
          }).then((saved) => { if (saved) props.onMessage("已保存副本", "info"); })
            .catch((error) => props.onMessage(`另存失败：${String(error)}`, "error"));
        },
      }, {
        label: "复制路径",
        onSelect: () => {
          void writeClipboard(workspaceAbsolutePath(conversation.context.workspaceRoot, relativePath))
            .then(() => props.onMessage("路径已复制", "info"))
            .catch((error) => props.onMessage(`复制失败：${String(error)}`, "error"));
        },
      }, {
        label: "复制文件内容",
        onSelect: () => {
          void invoke("workspace_copy_item_content", {
            localRoot: conversation.context.workspaceRoot,
            relativePath,
          }).then(() => props.onMessage("文件内容已复制", "info"))
            .catch((error) => props.onMessage(`复制失败：${String(error)}`, "error"));
        },
      });
      if (platformUi) items.push(
        { separator: true }, {
          label: `在${platformUi.fileManagerName}中显示`,
          onSelect: () => {
            void invoke("workspace_open_item", {
              localRoot: conversation.context.workspaceRoot,
              relativePath,
              openMode: "finder",
            }).catch((error) => props.onMessage(`无法在${platformUi.fileManagerName}中显示：${String(error)}`, "error"));
          },
        },
      );
    }
    setContextMenu(openContextMenu(event, items));
  }, [conversation, linkedConversationPath, openConversationLink, platformUi, props.onMessage]);
  const openConversationArtifact = useCallback(async (runId: string, artifact: ConversationArtifact) => {
    if (!conversation || !artifact.path) return;
    const relativePath = artifact.version ? null : workspaceRelativePath(artifact.path, conversation.context.workspaceRoot);
    if (relativePath && isPreviewablePath(relativePath) && conversation.context.workspaceId) {
      const file = props.files.find((candidate) => workspacePathsEqual(candidate.relativePath, relativePath, conversation.context.workspaceRoot));
      if (file && isPreviewableFile(file)) {
        props.onOpenFile?.(file.localFileId, conversation.context.workspaceId);
        return;
      }
      try {
        if (!await props.onOpenWorkspacePath?.(relativePath, conversation.context.workspaceId)) props.onMessage("无法在文件页打开：文件不存在或尚未完成索引。", "error");
      } catch (error) { props.onMessage(`无法打开产物：${String(error)}`, "error"); }
      return;
    }
    if (!platformUi) {
      props.onMessage("当前平台不支持文件定位。", "error");
      return;
    }
    void invoke("ai_artifact_open", { runId, artifactId: artifact.artifactId, openMode: "default" })
      .catch((error) => props.onMessage(`无法打开产物：${String(error)}`, "error"));
  }, [conversation, platformUi, props.files, props.onMessage, props.onOpenFile, props.onOpenWorkspacePath]);
  const openArtifactContextMenu = useCallback((event: React.MouseEvent, runId: string, artifact: ConversationArtifact) => {
    if (!artifact.path) return;
    if (!artifact.version && linkedConversationPath(artifact.path)) {
      openConversationLinkContextMenu(event, artifact.path);
      return;
    }
    const open = (openMode: "default" | "finder") => {
      void invoke("ai_artifact_open", { runId, artifactId: artifact.artifactId, openMode })
        .catch((error) => props.onMessage(`无法打开产物：${String(error)}`, "error"));
    };
    const items: ContextMenuItem[] = [{ label: "打开文件", onSelect: () => open("default") }];
    items.push({ label: "复制路径", onSelect: () => {
      const relativePath = linkedConversationPath(artifact.path!);
      const path = relativePath && conversation ? workspaceAbsolutePath(conversation.context.workspaceRoot, relativePath) : artifact.path!;
      void writeClipboard(path).then(() => props.onMessage("路径已复制", "info"))
        .catch((error) => props.onMessage(`复制失败：${String(error)}`, "error"));
    } });
    if (platformUi) items.push({ label: `在${platformUi.fileManagerName}中显示`, onSelect: () => open("finder") });
    setContextMenu(openContextMenu(event, items));
  }, [conversation, linkedConversationPath, openConversationLinkContextMenu, platformUi, props.onMessage]);
  useLayoutEffect(() => {
    setOutputFollowing(true);
    const node = messageList.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [selectedConversationId, setOutputFollowing]);
  useLayoutEffect(() => {
    const node = messageList.current;
    if (!node) return;
    if (followingLatestOutputRef.current) {
      node.scrollTop = node.scrollHeight;
      setUnseenOutput(false);
    } else {
      setUnseenOutput(true);
    }
  }, [visibleMessages.length, visibleSubmittedFollowUps.length, visiblePendingRegeneration?.messageId, waitingForRunFeedback, activeRun?.status, activeRun?.events.at(-1)?.sequence, activeRun?.partialOutput, conversationQueuedFollowUps.length]);

  const addMediaFiles = async (files: File[]) => {
    if (files.some(isDocumentAttachment) && !starterModelOverride) { props.onMessage("文档材料需要选择模型。", "error"); return; }
    if (!canAttachMedia && files.some((file) => !isDocumentAttachment(file))) {
      props.onMessage(mediaUnavailableReason, "error");
      return;
    }
    if (!canAttachVideo && files.some((file) => file.type.startsWith("video/"))) {
      props.onMessage("本地 Codex 智能体当前只接收图片。", "error");
      return;
    }
    const available = Math.max(0, 8 - draftAttachments.length);
    if (!available) {
      props.onMessage("一次最多添加 8 个附件。", "error");
      return;
    }
    setBusy(true);
    const staged: DraftAttachment[] = [];
    try {
      for (const file of files.slice(0, available)) {
        const attachment = await stageAiAttachment(file);
        staged.push({ attachment, previewUrl: URL.createObjectURL(file) });
      }
      setDraftAttachments((current) => [...current, ...staged]);
    } catch (error) {
      await Promise.all(staged.map((item) => discardAiAttachment(item.attachment.attachmentId).catch(() => undefined)));
      staged.forEach((item) => URL.revokeObjectURL(item.previewUrl));
      props.onMessage(aiErrorMessage(error), "error");
    } finally {
      setBusy(false);
      if (mediaPicker.current) mediaPicker.current.value = "";
    }
  };

  const removeDraftAttachment = async (attachmentId: string) => {
    const target = draftAttachments.find((item) => item.attachment.attachmentId === attachmentId);
    if (!target) return;
    try {
      await discardAiAttachment(attachmentId);
      URL.revokeObjectURL(target.previewUrl);
      setDraftAttachments((current) => current.filter((item) => item.attachment.attachmentId !== attachmentId));
    } catch (error) {
      props.onMessage(aiErrorMessage(error), "error");
    }
  };

  const selectSlashInvocation = (choice: SlashInvocationChoice) => {
    if (activeRun) return;
    if (!selectConversationSkill(choice.skillId)) return;
    setDraft((current) => removeApplicationInvocationQuery(current));
    requestAnimationFrame(() => draftBox.current?.focus());
  };

  const prepareMessageContext = async (
    content: string,
    selectedContext = applicationContext,
  ): Promise<{ content: string; applicationContext: null }> => {
    if (selectedContext) throw new Error("旧应用引用已停用，请移除后发送。");
    return { content, applicationContext: null };
  };
  useEffect(() => {
    if (!snapshotLoaded || !catalog.loaded || initializedSelection === draftKey) return;
    setInitializedSelection(draftKey);
    const latestRun = snapshot.runs
      .filter((run) => run.conversationId === conversation?.conversationId)
      .sort((left, right) => right.startedAt - left.startedAt)[0];
    let selection: ExecutionSelection = {
      agentId: conversation?.agentId ?? null,
      modelId: latestRun?.executionTarget === "model" ? latestRun.model : null,
      connectionId: latestRun?.modelSelection?.connectionId ?? null,
      reasoningEffort: latestRun?.executionTarget === "model" ? latestRun.reasoningEffort ?? null : null,
    };
    // 会话没有可沿用的模型运行时，按员工上次选择的模型进入；会话自身的执行档案保持不变
    if (!selection.modelId && !(latestRun && conversation?.agentId)) {
      try {
        const remembered = readLastExecutionSelection(props.profileId);
        if (remembered?.modelId) {
          if (!catalogModel(catalog.models, remembered.modelId, remembered.connectionId)) {
            props.onMessage("上次选择的模型已不可用，请重新选择。", "error");
          } else {
            const rememberedAgentId = remembered.agentId && snapshot.agents.some((agent) => agent.agentId === remembered.agentId) ? remembered.agentId : null;
            selection = { ...remembered, agentId: conversation?.agentId ?? rememberedAgentId };
          }
        }
      } catch (error) {
        props.onMessage(`无法恢复上次选择：${aiErrorMessage(error)}`, "error");
      }
    }
    setStarterAgentId(selection.agentId);
    setStarterModelOverride(selection.modelId);
    setStarterConnectionId(selection.connectionId ?? null);
    setStarterReasoningEffort(selection.reasoningEffort);
    setStarterBusinessId(null);
    setStarterFile(null);
    setStarterPermissionMode(selection.modelId
      ? rememberedApprovalMode
      : latestRun?.sandboxMode === "workspace-write" ? "workspace-write" : "read-only");
  }, [draftKey, snapshotLoaded, catalog.loaded, props.profileId]);

  const rememberExecutionSelection = (selection: ExecutionSelection) => {
    try {
      window.localStorage.setItem(executionSelectionStorageKey(props.profileId), JSON.stringify(selection));
    } catch (error) {
      props.onMessage(`无法保存本次选择：${aiErrorMessage(error)}`, "error");
    }
  };

  const selectStarterModel = (modelId: string, agentId: string, reasoningEffort: AiReasoningEffort | null, connectionId?: string | null) => {
    if (agentId) setStarterAgentId(agentId);
    setStarterModelOverride(modelId);
    setStarterConnectionId(connectionId ?? null);
    setStarterReasoningEffort(reasoningEffort);
    rememberExecutionSelection({ agentId, modelId, reasoningEffort, ...(connectionId ? { connectionId } : {}) });
    if (!modelApprovalOptionsForHost(props.hostPlatform).some((option) => option.value === starterPermissionMode)) {
      setStarterPermissionMode(rememberedApprovalMode);
    }
  };

  const selectConversationSkill = (skillId: string | null) => {
    if (skillId && !starterModelOverride) {
      props.onMessage("请先选择模型，再使用技能。", "error");
      return false;
    }
    setSkillSelections((current) => ({ ...current, [draftKey]: skillId ?? undefined }));
    return true;
  };

  const removeSelectedConversationSkill = () => {
    selectConversationSkill(null);
    requestAnimationFrame(() => draftBox.current?.focus());
  };

  const selectStarterPermissionMode = (permissionMode: StarterPermissionMode) => {
    setStarterPermissionMode(permissionMode);
    if (!conversation || !starterModelOverride || !isModelApprovalMode(permissionMode)) return;
    setSnapshot((current) => ({
      ...current,
      conversations: current.conversations.map((item) => item.conversationId === conversation.conversationId
        ? { ...item, approvalMode: permissionMode }
        : item),
    }));
    void updateAiConversation({
      conversationId: conversation.conversationId,
      title: null,
      mode: null,
      agentId: null,
      approvalMode: permissionMode,
      context: null,
      archived: null,
    }).catch((error) => {
      props.onMessage(`无法保存审批模式：${aiErrorMessage(error)}`, "error");
      void refresh();
    });
  };

  const selectStarterBusiness = (businessId: StarterBusinessId) => {
    const business = starterBusinesses.find((item) => item.id === businessId)!;
    const previous = starterBusinesses.find((item) => item.id === starterBusinessId);
    setStarterBusinessId(businessId);
    setDraft((current) => {
      const trimmed = current.trim();
      const retained = previous && trimmed.startsWith(previous.prompt)
        ? trimmed.slice(previous.prompt.length).trimStart()
        : trimmed;
      return retained ? `${business.prompt}\n${retained}` : business.prompt;
    });
    draftBox.current?.focus();
  };

  const newConversation = () => {
    const key = availableNewDraftKey("new:unassigned");
    setPendingCreate({
      title: "新对话",
      conversationType: "direct",
      mode: "discuss",
      context: unclassifiedConversationContext(),
      projectSelection: "editable",
      draftKey: key,
    });
    setSelectedConversationId(null);
    setStarterFile(null);
    setProjectPickerOpen(false);
    queueMicrotask(() => draftBox.current?.focus());
    return key;
  };

  const openConversation = (conversationId: string) => {
    setPendingCreate(null);
    setProjectPickerOpen(false);
    setSelectedConversationId(conversationId);
  };

  const confirmCreate = (project: AiProjectOption, requestedCreate = pendingCreate, projectSelection: "editable" | "fixed" = requestedCreate?.projectSelection ?? "fixed") => {
    if (actionBusy || (requestedCreate && sendingDrafts.current.has(requestedCreate.draftKey)) || !projectWorkspaceAvailable(project)) return;
    const context: AiContext = requestedCreate?.context.projectId === project.projectId
      ? requestedCreate.context
      : requestedCreate?.projectIds?.includes(project.projectId) && requestedCreate.context.workspaceId !== null && project.workspaceIds.includes(requestedCreate.context.workspaceId)
        ? { ...requestedCreate.context, projectId: project.projectId, projectName: project.displayName }
        : { workspaceId: project.primaryWorkspaceId, workspaceName: project.primaryWorkspaceName, workspaceRoot: project.primaryWorkspaceRoot ?? "", teamSpaceId: project.primaryTeamSpaceId ?? null, projectId: project.projectId, projectName: project.displayName, phaseId: null, phaseName: null, taskId: null, taskName: null, deliverableId: null, deliverableName: null, localFileId: null, relativePath: null, fileRevision: null, referencedMaterials: [] };
    setPendingCreate({
      title: requestedCreate?.title ?? "新对话",
      conversationType: requestedCreate?.conversationType ?? "direct",
      mode: requestedCreate?.mode ?? "discuss",
      context,
      projectIds: requestedCreate?.projectIds,
      projectSelection,
      draftKey: requestedCreate?.draftKey ?? availableNewDraftKey(`new:${project.projectId}`),
      selectedSkillId: requestedCreate?.selectedSkillId,
      prefill: requestedCreate?.prefill,
    });
    setSelectedConversationId(null);
    props.onSelectProject(project.projectId);
    setProjectPickerOpen(false);
  };

  useEffect(() => {
    if (!snapshotLoaded || !props.launchRequest || consumedLaunch.current === props.launchRequest.token) return;
    if (props.launchRequest.startFresh) {
      consumedLaunch.current = props.launchRequest.token;
      const key = newConversation();
      if (props.launchRequest.prefill) {
        const content = props.launchRequest.prefill;
        const title = props.launchRequest.title;
        const prefillKey = `new:launch:${props.launchRequest.token}`;
        setDrafts((current) => ({ ...current, [prefillKey]: content }));
        setPendingCreate((current) => current?.draftKey === key ? { ...current, title, draftKey:prefillKey } : current);
      }
      props.onLaunchConsumed?.(props.launchRequest.token);
      return;
    }
    const requestedCreate = {
      title: props.launchRequest.title,
      conversationType: props.launchRequest.conversationType,
      mode: props.launchRequest.mode,
      context: props.launchRequest.context,
      projectIds: props.launchRequest.projectIds,
      projectSelection: "fixed" as const,
      draftKey: `new:launch:${props.launchRequest.token}`,
      selectedSkillId: props.launchRequest.selectedSkillId,
      prefill: props.launchRequest.prefill,
    };
    consumedLaunch.current = props.launchRequest.token;
    props.onLaunchConsumed?.(props.launchRequest.token);
    if (requestedCreate.prefill) {
      setDrafts((current) => ({ ...current, [requestedCreate.draftKey]: requestedCreate.prefill ?? "" }));
    }
    if (requestedCreate.selectedSkillId) {
      setSkillSelections((current) => ({ ...current, [requestedCreate.draftKey]: requestedCreate.selectedSkillId }));
    }
    const candidates = requestedCreate.projectIds
      ? props.projects.filter((project) => requestedCreate.projectIds?.includes(project.projectId))
      : [];
    if (requestedCreate.projectIds && candidates.length === 0) {
      props.onMessage("当前文件没有可用于对话的关联项目。", "error");
      return;
    }
    setPendingCreate(requestedCreate);
    if (candidates.length === 1) {
      confirmCreate(candidates[0], requestedCreate);
      return;
    }
    if (!requestedCreate.projectIds && requestedCreate.context.workspaceId) {
      setProjectPickerOpen(false);
      return;
    }
    setProjectPickerOpen(true);
  }, [props.launchRequest?.token, snapshotLoaded, props.projects]);

  /*
   * 把刚启动的运行并入本地快照，不等整份快照刷新：运行一进入快照，回复轮询就接手
   * 拉取这个会话的消息与进度。运行若在返回时已经结束，就不会有轮询，改为补一次刷新。
   */
  const adoptStartedRun = (run: AiAgentRun, updatedConversation?: AiConversation) => {
    if (!pageMounted.current) { setRuntimeRevision((current) => current + 1); return; }
    setSnapshot((current) => ({
      ...current,
      conversations: updatedConversation
        ? [...current.conversations.filter((item) => item.conversationId !== updatedConversation.conversationId), updatedConversation]
        : current.conversations,
      runs: [...current.runs.filter((item) => item.runId !== run.runId), run],
    }));
    if (!activeRunStatuses.includes(run.status)) void refresh();
  };

  const adoptSubmission = (pendingId: string, conversationId: string, updatedConversation?: AiConversation) => {
    setSnapshot((current) => ({ ...current,
      conversations: updatedConversation
        ? [...current.conversations.filter((item) => item.conversationId !== conversationId), updatedConversation]
        : current.conversations,
      messageSubmissions: [
        ...(current.messageSubmissions ?? []).filter((item) => item.clientMessageId !== pendingId),
        { clientMessageId: pendingId, conversationId, status: "preparing", reason: null, acceptedAt: Date.now() },
      ],
    }));
  };

  const startConversation = async (writeConfirmed = false) => {
    if (starterPermissionSelectionInvalid) return;
    if (starterReasoningSelectionInvalid) {
      props.onMessage("当前思考深度已不受所选模型支持，请重新选择。", "error");
      return;
    }
    if ((!conversation && !pendingCreate) || !starterAgentId || !starterRuntimeReady || (!draft.trim() && !draftAttachments.length && !draftMaterials.length) || busy) return;
    if (!starterModelOverride && starterPermissionMode === "workspace-write" && !writeConfirmed) {
      setConfirmWriteSend(true);
      return;
    }
    const agent = snapshot.agents.find((item) => item.agentId === starterAgentId);
    const model = catalogModel(catalog.models, starterModelOverride, starterConnectionId);
    if (!agent || !model) {
      props.onMessage("请先选择可用的模型。", "error");
      return;
    }
    const historyConsent = modelSelection ? await obtainHistoryConsent(modelSelection, conversation?.conversationId ?? null) : null;
    if (historyConsent === false) return;
    const originalContent = draft.trim();
    let targetConversationId = conversation?.conversationId ?? null;
    const activeDraftKey = draftKey;
    if (!beginSending(activeDraftKey)) return;
    pendingOutgoingSequence.current += 1;
    const pendingId = `pending-message-${Date.now()}-${pendingOutgoingSequence.current}`;
    const submittedAt = Date.now();
    let submittedContent = originalContent;
    let accepted = false;
    // 草稿在提交那一刻就清掉：留着会和下面这条本地条目在界面上重复成两份同样的内容。
    setDrafts((current) => {
      const next = { ...current };
      delete next[activeDraftKey];
      return next;
    });
    setPendingOutgoing((current) => [...current, {
      pendingId,
      draftKey: activeDraftKey,
      conversationId: targetConversationId,
      content: originalContent,
      attachments: draftAttachments.map((item) => item.attachment),
      referencedMaterials: draftMaterials,
      createdAt: submittedAt,
      recovery: { create: pendingCreate, attachments: draftAttachments, referencedMaterials: draftMaterials, applicationContext, selectedSkillId: selectedSkillId ?? null, agentId: starterAgentId, modelOverride: starterModelOverride, ...(modelSelection ? { modelSelection } : {}), ...(historyConsent ? { historyConsent } : {}), reasoningEffort: effectiveStarterReasoningEffort, planning: false, permissionMode: effectiveStarterPermissionMode, file: starterFile },
    }]);
    setDraftAttachments([]);
    setDraftMaterials([]);
    let failure: unknown = null;
    try {
      await flushConversationSession();
      const invocation = await prepareMessageContext(originalContent);
      const content = invocation.content;
      const selectedApplicationContext = invocation.applicationContext;
      if (content !== submittedContent) {
        submittedContent = content;
        setPendingOutgoing((current) => current.map((item) => item.pendingId === pendingId ? { ...item, content } : item));
      }
      if (!conversation && pendingCreate) {
        const context = { ...pendingCreate.context, localFileId: null, relativePath: null, fileRevision: null, referencedMaterials: [] };
        const started = await submitAiConversation({
          clientMessageId: pendingId,
          title: conversationTitleFromFirstMessage(content, draftAttachments[0]?.attachment.fileName ?? (draftMaterials[0] ? fileNameFromPath(draftMaterials[0].relativePath) : undefined)),
          conversationType: pendingCreate.conversationType,
          mode: pendingCreate.mode,
          agentId: starterAgentId,
          context,
          content,
          attachmentIds: draftAttachments.map((item) => item.attachment.attachmentId), referencedMaterials: draftMaterials,
          applicationContext: selectedApplicationContext,
          selectedSkillId,
          modelOverride: starterModelOverride, ...(modelSelection ? { modelSelection } : {}), ...(historyConsent ? { historyConsent } : {}),
          reasoningEffort: effectiveStarterReasoningEffort,
          executionTarget: starterModelOverride || selectedApplicationContext || selectedSkillId ? "model" : "agent",
          planning: false, permissionMode: effectiveStarterPermissionMode,
        }, submittedAt);
        targetConversationId = started.conversation.conversationId;
        // 会话编号到手才能把本地条目挂到它名下，否则切走再回来这条就跟丢了。
        setPendingOutgoing((current) => current.map((item) => item.pendingId === pendingId ? { ...item, conversationId: targetConversationId } : item));
        // 选中态必须和清空待建草稿在同一批更新里提交：finally 里的 refresh 是个 await，
        // 会让出执行让 React 先跑 effect；那一刻若草稿已清空而选中态还没写入，
        // 自动新建草稿的 effect 会立刻补一个新草稿回来，界面就会卡在落地页。
        if (currentDraftKey.current === activeDraftKey) {
          setSelectedConversationId(targetConversationId);
          // 草稿转成已受理的会话时沿用当前选择；此时还没有运行供初始化 effect 恢复。
          setInitializedSelection(`conversation:${targetConversationId}`);
        }
        if (started.status === "recorded") {
          adoptStartedRun(started.run, started.conversation);
        } else {
          accepted = true;
          adoptSubmission(pendingId, targetConversationId, started.conversation);
        }
      } else if (conversation) {
        const updated = await updateAiConversation({
          conversationId: conversation.conversationId,
          title: messages.length ? null : conversationTitleFromFirstMessage(content, draftAttachments[0]?.attachment.fileName ?? (draftMaterials[0] ? fileNameFromPath(draftMaterials[0].relativePath) : undefined)),
          mode: null,
          agentId: starterAgentId === conversation.agentId ? null : starterAgentId,
          context: null,
          archived: null,
        });
        const submitted = await submitAiMessage({
          clientMessageId: pendingId,
          conversationId: conversation.conversationId,
          content,
          attachmentIds: draftAttachments.map((item) => item.attachment.attachmentId), referencedMaterials: draftMaterials,
          applicationContext: selectedApplicationContext,
          selectedSkillId,
          retryOfRunId: null,
          modelOverride: starterModelOverride, ...(modelSelection ? { modelSelection } : {}), ...(historyConsent ? { historyConsent } : {}),
          reasoningEffort: effectiveStarterReasoningEffort,
          executionTarget: starterModelOverride || selectedApplicationContext || selectedSkillId ? "model" : "agent",
          planning: false, permissionMode: effectiveStarterPermissionMode,
        }, submittedAt);
        if (submitted.status === "recorded") {
          adoptStartedRun(submitted.run, updated);
        } else {
          accepted = true;
          adoptSubmission(pendingId, conversation.conversationId, updated);
        }
      }
      setDrafts((current) => {
        const next = { ...current };
        delete next[activeDraftKey];
        return next;
      });
      if (!accepted) draftAttachments.forEach((item) => URL.revokeObjectURL(item.previewUrl));
      setDraftAttachmentGroups((current) => {
        const next = { ...current };
        delete next[activeDraftKey];
        return next;
      });
      setApplicationContextSelections((current) => {
        const next = { ...current };
        delete next[activeDraftKey];
        return next;
      });
      setSkillSelections((current) => {
        const next = { ...current };
        delete next[activeDraftKey];
        if (selectedSkillId && PERSISTENT_CREATOR_SKILL_IDS.has(selectedSkillId) && targetConversationId) {
          next[`conversation:${targetConversationId}`] = selectedSkillId;
        }
        return next;
      });
      if (currentDraftKey.current === activeDraftKey) {
        setStarterBusinessId(null);
        setStarterFile(null);
        setPendingCreate(null);
      }
    } catch (error) {
      failure = error;
    } finally {
      /*
       * 成功时不再等整份快照：运行已并入本地快照，回复轮询会带回这条用户消息，
       * 本地条目随即与之配对撤掉。只有失败才需要重取快照，判断消息是否已被记下。
       */
      if (failure) {
        const latest = await refresh();
        props.onMessage(aiErrorMessage(failure), "error");
        /*
         * 草稿只在服务端没记下这条消息时才退回输入框。运行启动失败时消息早已写入，
         * 退回草稿会引着用户再发一遍，服务端就多一条一模一样的。
         */
        const recorded = Boolean(latest?.messages.some((message) => message.role === "user" && message.messageId === pendingId));
        if (!recorded) {
          setDrafts((current) => ({ ...current, [activeDraftKey]: originalContent }));
          setDraftAttachments((current) => [...draftAttachments, ...current]);
          setDraftMaterialGroups((current) => ({ ...current, [activeDraftKey]: [...draftMaterials, ...(current[activeDraftKey] ?? [])] }));
        } else {
          draftAttachments.forEach((item) => URL.revokeObjectURL(item.previewUrl));
        }
        setPendingOutgoing((current) => current.filter((item) => item.pendingId !== pendingId));
      }
      finishSending(activeDraftKey);
      if (accepted) void refresh(false);
    }
  };

  const send = async () => {
    if (starterPermissionSelectionInvalid) return;
    // 运行时不可执行时按钮本来就是禁用的，这里再挡一次：回车提交和按钮禁用是两条路径。
    if (!conversation || (!draft.trim() && !draftAttachments.length && !draftMaterials.length) || busy || pendingApproval || !selectedRuntimeReady) return;
    if (editingMessageId) {
      setEditingMessageId(null);
      setMessageEditDraft("");
    }
    const originalContent = draft.trim();
    if (activeRun || threadDraining || mustQueue) {
      await queueFollowUp(originalContent);
      return;
    }
    const historyConsent = modelSelection ? await obtainHistoryConsent(modelSelection, conversation.conversationId) : null;
    if (historyConsent === false) return;
    const activeDraftKey = draftKey;
    if (!beginSending(activeDraftKey)) return;
    setDraft("");
    pendingOutgoingSequence.current += 1;
    const pendingId = `pending-message-${Date.now()}-${pendingOutgoingSequence.current}`;
    const submittedAt = Date.now();
    let submittedContent = originalContent;
    let submittedApplicationContext = applicationContext;
    let invocationResolved = false;
    setPendingOutgoing((current) => [...current, {
      pendingId,
      draftKey: activeDraftKey,
      conversationId: conversation.conversationId,
      content: originalContent,
      attachments: draftAttachments.map((item) => item.attachment),
      referencedMaterials: draftMaterials,
      createdAt: submittedAt,
      recovery: { create: pendingCreate, attachments: draftAttachments, referencedMaterials: draftMaterials, applicationContext, selectedSkillId: selectedSkillId ?? null, agentId: starterAgentId, modelOverride: starterModelOverride, ...(modelSelection ? { modelSelection } : {}), ...(historyConsent ? { historyConsent } : {}), reasoningEffort: effectiveStarterReasoningEffort, planning: false, permissionMode: effectiveStarterPermissionMode, file: starterFile },
    }]);
    setDraftAttachments([]);
    setDraftMaterials([]);
    try {
      await flushConversationSession();
      const invocation = await prepareMessageContext(originalContent);
      const content = invocation.content;
      const selectedApplicationContext = invocation.applicationContext;
      submittedApplicationContext = selectedApplicationContext;
      invocationResolved = true;
      if (content !== submittedContent) {
        // 斜杠调用会被改写成服务端实际记下的文本，本地条目要跟着改，否则配不上而重影。
        submittedContent = content;
        setPendingOutgoing((current) => current.map((item) => item.pendingId === pendingId ? { ...item, content } : item));
      }
      if (starterAgentId && starterAgentId !== conversation.agentId) {
        await updateAiConversation({ conversationId: conversation.conversationId, title: null, mode: null, agentId: starterAgentId, context: null, archived: null });
      }
      /*
       * 宿主受理即返回：企业会话校验、应用发现和运行启动在后台完成，这段时间输入框照常可用，
       * 后续内容进入排队。消息落库后本地条目由快照自动替换；落库前失败时回执给出原因，
       * 这条内容以"发送失败"保留，可恢复为草稿。缩略图要留给恢复使用，此时不回收。
       */
      const submitted = await submitAiMessage({ clientMessageId: pendingId, conversationId: conversation.conversationId, content, attachmentIds: draftAttachments.map((item) => item.attachment.attachmentId), referencedMaterials: draftMaterials, applicationContext: selectedApplicationContext, selectedSkillId, retryOfRunId: null, modelOverride: starterModelOverride, ...(modelSelection ? { modelSelection } : {}), ...(historyConsent ? { historyConsent } : {}), reasoningEffort: effectiveStarterReasoningEffort, executionTarget: starterModelOverride || selectedApplicationContext || selectedSkillId ? "model" : "agent", planning: false, permissionMode: effectiveStarterPermissionMode }, submittedAt);
      if (submitted.status === "recorded") {
        adoptStartedRun(submitted.run);
        draftAttachments.forEach((item) => URL.revokeObjectURL(item.previewUrl));
      } else {
        // 先在本地记下回执，不等下一次快照：这一刻起同一对话的新内容就要进入排队
        const acceptedConversationId = conversation.conversationId;
        setSnapshot((current) => ({ ...current, messageSubmissions: [
          ...(current.messageSubmissions ?? []).filter((item) => item.clientMessageId !== pendingId),
          { clientMessageId: pendingId, conversationId: acceptedConversationId, status: "preparing", reason: null, acceptedAt: Date.now() },
        ] }));
        void refresh(false);
      }
      setDraftAttachments((current) => current.filter((item) => !draftAttachments.some((sent) => sent.attachment.attachmentId === item.attachment.attachmentId)));
      setApplicationContextSelections((current) => {
        const next = { ...current };
        delete next[draftKey];
        return next;
      });
      setSkillSelections((current) => {
        if (selectedSkillId && PERSISTENT_CREATOR_SKILL_IDS.has(selectedSkillId)) return current;
        const next = { ...current };
        delete next[draftKey];
        return next;
      });
    } catch (error) {
      props.onMessage(aiErrorMessage(error), "error");
      /*
       * 失败后必须重取一次快照。运行启动失败时，用户消息在此之前就已写入服务端，
       * 只有运行被标成失败；不刷新的话这条消息就卡在服务端看不见，直到下一次发送
       * 成功才连同新消息一起冒出来——界面上看起来就是一次发出去两三条。
       *
       * 草稿是否退回也由这次快照决定：已经记下的不退回，否则用户照着输入框里
       * 残留的内容再发一遍，服务端就多一条一模一样的消息。
       */
      const latest = await refresh();
      const recorded = Boolean(latest?.messages.some((message) => message.conversationId === conversation.conversationId
        && message.role === "user" && message.messageId === pendingId));
      if (!recorded) {
        if (!draftsRef.current[activeDraftKey]) {
          setDraft(originalContent);
          setDraftAttachments((current) => [...draftAttachments, ...current]);
          setDraftMaterialGroups((current) => ({ ...current, [activeDraftKey]: [...draftMaterials, ...(current[activeDraftKey] ?? [])] }));
        } else {
          setQueuedFollowUps((current) => [...current, {
            queueId: pendingId,
            queuedAt: submittedAt,
            conversationId: conversation.conversationId,
            content: submittedContent,
            attachments: draftAttachments,
            referencedMaterials: draftMaterials,
            applicationContext: submittedApplicationContext,
            requiresApplicationResolution: !invocationResolved,
            selectedSkillId,
            modelOverride: starterModelOverride, ...(modelSelection ? { modelSelection } : {}), ...(historyConsent ? { historyConsent } : {}),
            reasoningEffort: effectiveStarterReasoningEffort,
            executionTarget: starterModelOverride || submittedApplicationContext || selectedSkillId ? "model" : "agent",
            planning: false, permissionMode: effectiveStarterPermissionMode,
            status: "failed",
          }]);
          setDraftAttachments((current) => current.filter((item) => !draftAttachments.some((sent) => sent.attachment.attachmentId === item.attachment.attachmentId)));
        }
      } else {
        draftAttachments.forEach((item) => URL.revokeObjectURL(item.previewUrl));
        setDraftAttachments((current) => current.filter((item) => !draftAttachments.some((sent) => sent.attachment.attachmentId === item.attachment.attachmentId)));
      }
      setPendingOutgoing((current) => current.filter((item) => item.pendingId !== pendingId));
    } finally { finishSending(activeDraftKey); }
  };

  const restoreOutgoing = (item: PendingOutgoingMessage) => {
    const recovery = item.recovery;
    if (!recovery) { props.onMessage("发送记录缺少上下文，请复制内容后重新组织。", "error"); return; }
    const key = item.conversationId ? `conversation:${item.conversationId}` : item.draftKey;
    if (draftsRef.current[key]?.trim() || draftAttachmentGroups[key]?.length || draftMaterialGroups[key]?.length || (pendingCreate && pendingCreate.draftKey !== key && draft.trim())) {
      props.onMessage("请先处理当前草稿，再恢复这条内容。", "error"); return;
    }
    if (item.conversationId && !snapshot.conversations.some((entry) => entry.conversationId === item.conversationId && !entry.archived)) {
      props.onMessage("原对话当前不可用，请复制内容后重新组织。", "error"); return;
    }
    setDrafts((current) => ({ ...current, [key]: item.content }));
    setDraftAttachmentGroups((current) => ({ ...current, [key]: recovery.attachments }));
    setDraftMaterialGroups((current) => ({ ...current, [key]: recovery.referencedMaterials ?? [] }));
    setApplicationContextSelections((current) => ({ ...current, [key]: recovery.applicationContext ?? undefined }));
    setSkillSelections((current) => ({ ...current, [key]: recovery.selectedSkillId ?? undefined }));
    setStarterAgentId(recovery.agentId);
    setStarterModelOverride(recovery.modelOverride);
    setStarterConnectionId(recovery.modelSelection?.connectionId ?? null);
    setStarterReasoningEffort(recovery.reasoningEffort);
    setStarterPermissionMode(recovery.permissionMode);
    setStarterFile(recovery.file);
    setPendingCreate(item.conversationId ? null : recovery.create);
    setSelectedConversationId(item.conversationId);
    setInitializedSelection(key);
    setRecoveredOutgoing((current) => current.filter((entry) => entry.pendingId !== item.pendingId));
    if (submissionsById.get(item.pendingId)?.status === "failed") discardFailedOutgoing(item);
  };

  /* 受理失败的发送：移除本地条目并清掉宿主回执。内容已恢复为草稿或用户明确放弃。 */
  const discardFailedOutgoing = (item: PendingOutgoingMessage) => {
    setPendingOutgoing((current) => current.filter((entry) => entry.pendingId !== item.pendingId));
    setSnapshot((current) => ({ ...current, messageSubmissions: (current.messageSubmissions ?? []).filter((entry) => entry.clientMessageId !== item.pendingId) }));
    void dismissAiMessageSubmission(item.pendingId).catch((error) => props.onMessage(aiErrorMessage(error), "error"));
  };

  const copyMessage = async (content: string) => {
    try {
      await writeClipboard(content);
      props.onMessage("对话内容已复制。");
    } catch {
      props.onMessage("复制对话内容失败。", "error");
    }
  };

  const regenerateMessage = async (messageId: string) => {
    if (starterPermissionSelectionInvalid) return;
    const content = messageEditDraft.trim();
    if (!conversation || messageId !== latestEditableUserMessageId || !content || busy || activeRun || threadDraining || conversationSubmitting || conversationHasPendingFollowUps || !selectedRuntimeReady) return;
    const historyConsent = modelSelection ? await obtainHistoryConsent(modelSelection, conversation.conversationId) : null;
    if (historyConsent === false) return;
    const sourceRunId = messages.find((message) => message.messageId === messageId)?.runId ?? null;
    setPendingRegeneration({ conversationId: conversation.conversationId, messageId, sourceRunId, content, submittedAt: Date.now() });
    setEditingMessageId(null);
    setBusy(true);
    try {
      if (starterAgentId && starterAgentId !== conversation.agentId) {
        await updateAiConversation({ conversationId: conversation.conversationId, title: null, mode: null, agentId: starterAgentId, context: null, archived: null });
      }
      const run = await regenerateAiMessage({
        conversationId: conversation.conversationId,
        messageId,
        content,
        modelOverride: starterModelOverride, ...(modelSelection ? { modelSelection } : {}), ...(historyConsent ? { historyConsent } : {}),
        reasoningEffort: effectiveStarterReasoningEffort,
        executionTarget: starterModelOverride ? "model" : "agent",
        planning: false, permissionMode: effectiveStarterPermissionMode,
      });
      setMessageEditDraft("");
      setSnapshot((current) => {
        const messageIndex = current.messages.findIndex((item) => item.messageId === messageId);
        const replacedRunIds = new Set(current.runs
          .filter((item) => item.conversationId === conversation.conversationId && item.inputMessageId === messageId && item.runId !== run.runId)
          .map((item) => item.runId));
        return {
          ...current,
          messages: current.messages
            .filter((item, index) => messageIndex < 0 || index <= messageIndex || item.conversationId !== conversation.conversationId)
            .map((item) => item.messageId === messageId ? { ...item, content, runId: run.runId } : item),
          runs: [...current.runs.filter((item) => item.runId !== run.runId).map((item) => replacedRunIds.has(item.runId) ? { ...item, superseded: true } : item), run],
        };
      });
      setPendingRegeneration(null);
      await refresh();
    } catch (error) {
      props.onMessage(aiErrorMessage(error), "error");
      const latest = await refresh();
      const persisted = latest?.messages.find((message) => message.conversationId === conversation.conversationId
        && message.messageId === messageId);
      if (!persisted || persisted.runId === sourceRunId) setEditingMessageId(messageId);
      else setMessageEditDraft("");
      setPendingRegeneration(null);
    } finally {
      setBusy(false);
    }
  };

  const queueFollowUp = async (content = draft.trim()) => {
    if (starterPermissionSelectionInvalid) return;
    if (!conversation || (!activeRun && !threadDraining && !mustQueue) || (starterModelOverride && !modelSandboxReady) || (!content && !draftAttachments.length && !draftMaterials.length) || busy) return;
    const historyConsent = modelSelection ? await obtainHistoryConsent(modelSelection, conversation.conversationId) : null;
    if (historyConsent === false) return;
    let queuedContent = content;
    let queuedApplicationContext = applicationContext;
    try {
      if (applicationContext || content.trimStart().startsWith("/")) {
        const invocation = await prepareMessageContext(content);
        queuedContent = invocation.content;
        queuedApplicationContext = invocation.applicationContext;
      }
    } catch (error) {
      props.onMessage(aiErrorMessage(error), "error");
      return;
    }
    const attachments = draftAttachments;
    setDraft("");
    setDraftAttachments([]);
    setDraftMaterials([]);
    setApplicationContextSelections((current) => {
      const next = { ...current };
      delete next[draftKey];
      return next;
    });
    const queuedAt = Date.now();
    setQueuedFollowUps((current) => [...current, {
      queueId: `queued-follow-up-${randomUuid()}`,
      queuedAt,
      conversationId: conversation.conversationId,
      content: queuedContent,
      attachments,
      referencedMaterials: draftMaterials,
      applicationContext: queuedApplicationContext,
      selectedSkillId,
      modelOverride: starterModelOverride, ...(modelSelection ? { modelSelection } : {}), ...(historyConsent ? { historyConsent } : {}),
      reasoningEffort: effectiveStarterReasoningEffort,
      executionTarget: starterModelOverride || queuedApplicationContext || selectedSkillId ? "model" : "agent",
      planning: false, permissionMode: effectiveStarterPermissionMode,
      status: "queued",
    }]);
  };

  const retryQueuedFollowUp = async (queueId: string) => {
    const queued = queuedFollowUps.find((item) => item.queueId === queueId && item.status === "failed");
    if (!queued) return;
    try {
      const invocation = await prepareMessageContext(queued.content, queued.applicationContext);
      const historyConsent = queued.modelSelection ? await obtainHistoryConsent(queued.modelSelection, queued.conversationId) : null;
      if (historyConsent === false) return;
      setQueuedFollowUps((current) => current.map((item) => item === queued ? {
        ...item, ...invocation, requiresApplicationResolution: false,
        historyConsent,
        executionTarget: invocation.applicationContext || item.selectedSkillId ? "model" : item.executionTarget,
        status: "queued",
      } : item));
    } catch (error) { props.onMessage(aiErrorMessage(error), "error"); }
  };

  const editQueuedFollowUp = (queueId: string) => {
    const queued = queuedFollowUps.find((item) => item.queueId === queueId && item.conversationId === conversation?.conversationId);
    if (!queued || busy || queued.status === "sending" || queued.status === "steering") return;
    if (draft || draftAttachments.length || draftMaterials.length) {
      props.onMessage("请先发送或清空当前草稿。", "error");
      return;
    }
    setQueuedFollowUps((current) => current.filter((item) => item.queueId !== queueId));
    setStarterModelOverride(queued.modelOverride);
    setStarterConnectionId(queued.modelSelection?.connectionId ?? null);
    setStarterReasoningEffort(queued.reasoningEffort);
    setStarterPermissionMode(queued.permissionMode);
    setDraft(queued.content);
    setDraftAttachments(queued.attachments);
    setDraftMaterials(queued.referencedMaterials ?? []);
    setSkillSelections((current) => ({ ...current, [draftKey]: queued.selectedSkillId ?? undefined }));
    setApplicationContextSelections((current) => ({ ...current, [draftKey]: queued.applicationContext ?? undefined }));
    requestAnimationFrame(() => {
      const input = draftBox.current;
      if (!input) return;
      input.focus();
      if (input instanceof HTMLTextAreaElement) {
        input.setSelectionRange(input.value.length, input.value.length);
      } else {
        placeCaretAtEnd(input);
      }
      input.scrollIntoView({ block: "nearest" });
    });
  };

  const steerQueuedFollowUp = async (queueId: string) => {
    const queued = queuedFollowUps.find((item) => item.queueId === queueId);
    if (!queued || queued.executionTarget !== "model" || queued.attachments.length || queued.referencedMaterials?.length || queued.applicationContext || queued.status === "sending" || queued.status === "steering" || busy) return;
    if (!supplementTarget || supplementUnavailableReason) return;
    setOutputFollowing(true);
    setBusy(true);
    setQueuedFollowUps((current) => current.map((item) => item.queueId === queueId ? { ...item, status: "steering", steeringRunId: supplementTarget.runId, transcriptCommitted: true } : item));
    try {
      await flushConversationSession();
      const run = await steerAiRun(supplementTarget.runId, queued.content, queued.queueId, queued.planning ?? false);
      setSnapshot((current) => ({ ...current, runs: [...current.runs.filter((item) => item.runId !== run.runId), run] }));
      setQueuedFollowUps((current) => current.map((item) => item.queueId === queueId
        ? { ...item, steeringRunId: run.runId, sentMessageId: queued.queueId } : item));
    } catch (error) {
      const timedOut = String(error).includes("codex_harness_steer_timed_out");
      if (timedOut) {
        props.onMessage(aiErrorMessage(error), "info");
      } else {
        setQueuedFollowUps((current) => current.map((item) => item.queueId === queueId
          ? { ...item, status: "failed", steeringRunId: undefined, transcriptCommitted: false } : item));
        props.onMessage(`补充回复失败：${aiErrorMessage(error)}`, "error");
      }
    } finally {
      await refresh();
      setBusy(false);
    }
  };

  const deleteQueuedFollowUp = async (queueId: string) => {
    if (busy) return;
    let queued: QueuedFollowUp | undefined;
    setQueuedFollowUps((current) => current.map((item) => {
      if (item.queueId !== queueId || !["queued", "failed"].includes(item.status)) return item;
      queued = item;
      return { ...item, status: "deleting" };
    }));
    if (!queued) return;
    setBusy(true);
    try {
      await flushConversationSession();
      await Promise.all(queued.attachments.map((item) => discardAiAttachment(item.attachment.attachmentId)));
      queued.attachments.forEach((item) => URL.revokeObjectURL(item.previewUrl));
      setQueuedFollowUps((current) => current.filter((item) => item.queueId !== queueId));
    } catch (error) {
      setQueuedFollowUps((current) => current.map((item) => item.queueId === queueId ? { ...item, status: "failed" } : item));
      props.onMessage(aiErrorMessage(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const stopActiveRun = async () => {
    if (!activeRun || busy) return;
    setBusy(true);
    try {
      await controlAiRun(activeRun.runId, "terminate");
      await refresh();
    } catch (error) {
      props.onMessage(aiErrorMessage(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const resolveApproval = async (decision: "approved" | "approved_session" | "rejected", applicationWriteArguments?: Record<string, unknown>) => {
    if (!pendingApproval || busy) return;
    setBusy(true);
    try {
      await resolveAiPermissionRequest(pendingApproval.permissionRequestId, decision, applicationWriteArguments);
      await refresh();
    } catch (error) {
      props.onMessage(aiErrorMessage(error), "error");
    } finally {
      setBusy(false);
    }
  };


  const updateContext = async (context: AiContext) => {
    if (!conversation) return;
    try {
      await updateAiConversation({ conversationId: conversation.conversationId, title: null, mode: null, agentId: null, context, archived: null });
      await refresh();
    } catch (error) { props.onMessage(aiErrorMessage(error), "error"); }
  };

  const archiveConversations = async (ids: string[]) => {
    if (ids.length === 0 || ids.some((id) => archivingConversations.current.has(id) || sendingDrafts.current.has(`conversation:${id}`))) return;
    ids.forEach((id) => archivingConversations.current.add(id));
    setArchivingIds(new Set(archivingConversations.current));
    try {
      const result = await setConversationsArchived(ids, true);
      const archivedIds = new Set(result.conversations.filter((item) => item.archived).map((item) => item.conversationId));
      setSelectedConversationId((current) => current && archivedIds.has(current) ? null : current);
      if (result.failures.length) props.onMessage(`已归档 ${archivedIds.size} 条，${result.failures.length} 条未完成：${aiErrorMessage(result.failures[0].reason)}`, "error");
    } catch (error) {
      props.onMessage(`归档失败：${aiErrorMessage(error)}`, "error");
    } finally {
      ids.forEach((id) => archivingConversations.current.delete(id));
      setArchivingIds(new Set(archivingConversations.current));
      setPendingArchive(null);
    }
  };

  const archiveProjectConversations = (ids: string[]) => archiveConversations(ids);
  const archiveConversation = (conversationId?: string) => {
    const target = conversationId ?? conversation?.conversationId;
    return archiveConversations(target ? [target] : []);
  };

  const saveDecisionCard = async (messageId: string, content: string) => {
    if (!conversation?.context.projectId || savingDecisionMessageId) return;
    const card = parseDecisionCard(content);
    if (!card) {
      props.onMessage("当前 AI 回复不满足决策卡的六个必填部分，请让 AI 按固定标题重新输出。", "error");
      return;
    }
    setSavingDecisionMessageId(messageId);
    try {
      const clientOperationId = `desktop-ai-decision-${randomUuid()}`;
      const requestFingerprint = await sha256([conversation.context.projectId, conversation.context.phaseId ?? "", conversation.context.deliverableId ?? "", card.question, card.options, card.impact, card.evidence, card.recommendation, card.consequence].join("\n"));
      await rpc<DecisionRequestCreateResult>("decisionRequest/create", {
        projectId: conversation.context.projectId,
        phaseId: conversation.context.phaseId,
        deliverableId: conversation.context.deliverableId,
        title: card.question.split("\n")[0].slice(0, 120),
        question: card.question,
        options: card.options,
        impact: card.impact,
        evidence: card.evidence,
        recommendation: card.recommendation,
        consequence: card.consequence,
        decisionOwnerLabel: "项目负责人",
        requesterLabel: "当前用户",
        priority: "high",
        dueAt: null,
        clientOperationId,
        requestFingerprint,
      });
      props.onMessage("决策卡已从 AI 回复显式保存，现在可在“验收与决策”中处理。");
    } catch (error) {
      props.onMessage(aiErrorMessage(error), "error");
    } finally {
      setSavingDecisionMessageId(null);
    }
  };

  const completedSkillRunIds = snapshot.runs.filter((run) => run.conversationId === conversation?.conversationId && run.status === "completed" && !run.superseded).slice(-256).map((run) => run.runId);
  const { installations: skillInstallations, error: skillStatusError, retry: retrySkillInstallation } = useSkillInstallations(completedSkillRunIds, props.profileId);
  const installedSkillKey = Object.values(skillInstallations).filter((item) => item.status === "installed").map((item) => item.skillId).sort().join(",");
  useEffect(() => {
    if (!installedSkillKey) return;
    setSkillSelections((current) => {
      if (current[draftKey] !== "system.skill-creator") return current;
      const next = { ...current }; delete next[draftKey]; return next;
    });
    let active = true;
    void rpc<SkillConversationCatalogResult>("skill/conversationCatalog", {}).then((result) => {
      if (active) setConversationSkills(result.skills);
    }).catch((error) => { if (active) props.onMessage(describeServerError(error), "error"); });
    return () => { active = false; };
  }, [installedSkillKey, props.profileId]);

  // 一个项目下的会话可能很多。默认只露出最近几条，其余收在"展开其余"后面，
  // 免得一个活跃项目把别的项目整个挤出视野。
  const conversationsPerProject = 6;
  // 置顶表达的是"我常用这个项目"，关掉应用不该忘记，所以落到本地存储。
  const [pinnedProjects, setPinnedProjects] = useState<Set<string>>(() => {
    const raw = window.localStorage.getItem(pinnedProjectsStorageKey);
    return new Set(raw === null ? [] : (JSON.parse(raw) as string[]));
  });
  const togglePinned = useCallback((project: string) => {
    setPinnedProjects((current) => {
      const next = new Set(current);
      if (next.has(project)) next.delete(project); else next.add(project);
      window.localStorage.setItem(pinnedProjectsStorageKey, JSON.stringify([...next]));
      return next;
    });
  }, []);
  const [collapsedProjects, setCollapsedProjects] = useState<Set<string>>(new Set());
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(new Set());
  const toggleProject = useCallback((project: string) => {
    setCollapsedProjects((current) => {
      const next = new Set(current);
      if (next.has(project)) next.delete(project); else next.add(project);
      return next;
    });
  }, []);
  const expandProject = useCallback((project: string) => {
    setExpandedProjects((current) => new Set(current).add(project));
  }, []);

  // 会话按项目归属分组，侧栏只负责组织会话，不承接项目编辑和文件管理操作。
  const conversationsByProject = useMemo(() => {
    const groups = new Map<string, { projectKey: string; project: string; items: typeof conversations }>();
    for (const project of props.projects) {
      if (projectWorkspaceAvailable(project)) {
        groups.set(project.projectId, { projectKey: project.projectId, project: project.displayName, items: [] });
      }
    }
    for (const item of conversations) {
      const currentProject = item.context.projectId
        ? props.projects.find((project) => project.projectId === item.context.projectId)
        : undefined;
      const project = currentProject?.displayName ?? item.context.projectName ?? "未分类";
      const projectKey = item.context.projectId ?? "unassigned";
      const bucket = groups.get(projectKey);
      if (bucket) {
        bucket.items.push(item);
      } else {
        groups.set(projectKey, { projectKey, project, items: [item] });
      }
    }
    // 置顶的项目排在前面，其余保持会话本身的时间顺序。
    return [...groups.values()].sort((left, right) =>
      Number(pinnedProjects.has(right.projectKey)) - Number(pinnedProjects.has(left.projectKey)));
  }, [conversations, pinnedProjects, props.projectIndex, props.projects]);

  /** 有运行在跑的会话。集合先算好，免得每行都去遍历一遍运行列表。 */
  const runningConversationIds = useMemo(() => new Set(
    snapshot.runs
      .filter((run) => activeRunStatuses.includes(run.status))
      .map((run) => run.conversationId)), [snapshot.runs]);

  const projectMenuItems = (group: { projectKey: string; project: string; items: typeof conversations }) => [
    { label: pinnedProjects.has(group.projectKey) ? "取消置顶" : "置顶", onSelect: () => togglePinned(group.projectKey) },
    ...(group.items.length ? [
      { separator: true as const },
      { label: `归档这里的 ${group.items.length} 条会话`, onSelect: () => setPendingArchive({ project: group.project, ids: group.items.map((item) => item.conversationId) }) },
    ] : []),
  ];

  const pendingReplyAnchor = activeRun ? visibleMessages.map((message) =>
    message.runId === activeRun.runId || message.messageId === activeRun.inputMessageId).lastIndexOf(true) : -1;
  const pendingReply = waitingForRunFeedback || (activeRun && !pendingApproval) ? <article aria-label={waitingForRunFeedback ? `${selectedModel?.label ?? selectedAgent?.name ?? "AI"} ${activeRunProgress}` : undefined} aria-live={waitingForRunFeedback ? "polite" : undefined} className="ai-message assistant ai-message-pending"><header><span>{activeRun ? activeRun.executionTarget === "model" ? activeRun.modelLabel ?? catalog.models.find((model) => model.modelId === activeRun.model)?.label ?? displayModelId(activeRun.model ?? "") : selectedAgent?.name ?? "AI" : selectedModel?.label ?? selectedAgent?.name ?? "AI"}</span>{activeRunStartedAt !== null ? <RunElapsedTime completedAt={activeRun?.completedAt ?? null} startedAt={activeRunStartedAt}/> : null}</header><RunExecutionProcess active progress={activeRunProgress} run={activeRun}/>{activeRun?.artifacts?.some((artifact) => artifact.artifactType === "image") ? <RunArtifacts onContext={openArtifactContextMenu} onOpen={(artifact) => void openConversationArtifact(activeRun.runId, artifact)} run={activeRun}/> : activeImageGenerationProgress ? <ImageGenerationPending phase={activeImageGenerationProgress}/> : null}{activeRun?.partialOutput ? <div aria-label="正在输出的回复" aria-live="polite" className="ai-streaming-output"><MarkdownContent canOpenLink={canOpenConversationLink} diagramIdPrefix="conversation-streaming" onContextLink={openConversationLinkContextMenu} onOpenLink={openConversationLink}>{assistantMessageBody(activeRun.partialOutput, activeRun.classificationNonce)}</MarkdownContent></div> : <PendingReplyIntro/>}</article> : null;

  const composerLabel = conversationStarted ? "发送给 AI" : "描述要处理的任务";
  const composerPlaceholder = conversationStarted ? "输入问题、补充约束或下一步要求…" : starterPlaceholder;
  // 新建对话提交后到对话建成之间锁住输入；对话进行中不锁，员工可以边等边写下一句。
  const composerLocked = historyTransfer.busy || (!conversationStarted && busy);
  const infoPanelContext = conversationInfoContext ?? pendingCreate?.context ?? null;
  const submitComposer = () => conversationStarted ? send() : startConversation();
  const handleComposerKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape" && slashInvocationQuery !== null) { event.preventDefault(); setDraft(""); return; }
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) {
      event.preventDefault();
      if (slashInvocationQuery !== null) { if (selectableSlashInvocationChoices[0]) selectSlashInvocation(selectableSlashInvocationChoices[0]); return; }
      void submitComposer();
    }
  };

  const removeMaterial = (localFileId: string) => {
    const context = conversation?.context ?? pendingCreate?.context;
    if (!context) return;
    const references = materialReferences(context).filter((item) => item.localFileId !== localFileId);
    const next = { ...context, referencedMaterials: references,
      ...(context.localFileId === localFileId ? { localFileId: null, relativePath: null, fileRevision: null } : {}) };
    if (conversation) void updateContext(next);
    else if (pendingCreate) setPendingCreate({ ...pendingCreate, context: next });
    if (starterFile?.localFileId === localFileId) setStarterFile(null);
  };


  return <section
    onDragOver={(event) => { if (event.dataTransfer.types.includes("Files")) event.preventDefault(); }}
    onDrop={(event) => { if (!event.dataTransfer.files.length) return; event.preventDefault(); if (busy) return;
      const files = Array.from(event.dataTransfer.files);
      if (files.some((file) => !isSupportedMedia(file) && !isDocumentAttachment(file))) { props.onMessage("支持 PDF、Word、Excel、PPT、文本、图片和短视频。", "error"); return; }
      void addMediaFiles(files);
    }}
    className={`ai-workbench${conversationRailOpen ? "" : " conversation-rail-collapsed"}${resizingConversationRail ? " resizing-conversation-rail" : ""}`}
    style={{ "--ai-conversation-rail-width": `${conversationRailWidth}px` } as CSSProperties}>
    <aside className="ai-conversation-rail" hidden={!conversationRailOpen} id="ai-conversation-rail">
      <header><div><h2>对话记录</h2><p>按项目整理</p></div><button aria-label="新建 AI 对话" disabled={actionBusy} onClick={() => newConversation()} title="新建对话" type="button"><PlusIcon/></button></header>
      <div className="ai-conversation-list">{conversationsByProject.map((group) => {
          const collapsed = collapsedProjects.has(group.projectKey);
          const expanded = expandedProjects.has(group.projectKey);
          const selected = pendingCreate?.context.projectId === group.projectKey || group.items.some((item) => item.conversationId === selectedConversationId && !pendingCreate);
          const visible = collapsed ? [] : expanded ? group.items : group.items.slice(0, conversationsPerProject);
          const hidden = group.items.length - visible.length;
          const projectMenuOpen = openProjectMenuKey === group.projectKey;
          const openProjectMenu = (event: React.MouseEvent) => {
            setOpenProjectMenuKey(group.projectKey);
            setContextMenu(openContextMenu(event, projectMenuItems(group)));
          };
          return <div className="ai-conversation-group" key={group.projectKey}>
            <div className={`ai-conversation-group-header${projectMenuOpen ? " menu-open" : ""}`}>
              <button
                aria-label={`${collapsed ? "展开" : "收起"}项目分类：${group.project}`}
                aria-current={selected ? "true" : undefined}
                aria-expanded={!collapsed}
                className="ai-conversation-group-toggle"
                onClick={() => toggleProject(group.projectKey)}
                onContextMenu={openProjectMenu}
                title={group.project}
                type="button"><ProjectFolderIcon expanded={!collapsed}/><span>{group.project}</span>{pinnedProjects.has(group.projectKey) ? <PinIcon/> : null}</button>
              <span className="ai-conversation-group-actions">
                {group.projectKey !== "unassigned" ? <button aria-label={`在 ${group.project} 中新建对话`} disabled={actionBusy} onClick={() => {
                  const project = props.projects.find((item) => item.projectId === group.projectKey);
                  if (project) confirmCreate(project, null, "fixed");
                }} title="在此项目中新建对话" type="button"><PlusIcon/></button> : null}
                <button aria-expanded={projectMenuOpen} aria-haspopup="menu" aria-label={`${group.project} 的更多操作`} onClick={openProjectMenu} title="更多操作" type="button"><MoreDotsIcon/></button>
              </span>
            </div>
            {visible.map((item) => {
              const itemMessages = messagesByConversation.get(item.conversationId) ?? [];
              const topic = conversationTopic(item, itemMessages);
              return <div
              className={`ai-conversation-item${item.conversationId === selectedConversationId && !pendingCreate ? " selected" : ""}`}
              key={item.conversationId}
              onContextMenu={(event) => {
                setOpenProjectMenuKey(null);
                setContextMenu(openContextMenu(event, [
                  { label: "打开会话", onSelect: () => openConversation(item.conversationId) },
                  { label: "重命名", disabled: runningConversationIds.has(item.conversationId), onSelect: () => beginConversationRename(item) },
                  { label: "复制会话标题", onSelect: () => void writeClipboard(topic).catch(() => props.onMessage("复制失败。", "error")) },
                  { separator: true },
                  { label: "归档会话", onSelect: () => void archiveConversation(item.conversationId) },
                ]));
              }}>
                <button aria-current={item.conversationId === selectedConversationId && !pendingCreate ? "true" : undefined} className="ai-conversation-open" onClick={() => openConversation(item.conversationId)} title={topic} type="button"><span className="ai-conversation-copy"><strong>{topic}</strong><small>{conversationSummary(item, itemMessages, Object.values(skillInstallations), snapshot.runs)}</small></span><span className="ai-conversation-meta"><time>{timelineMoment(item.updatedAt)}</time>{runningConversationIds.has(item.conversationId) ? <span className="ai-running-mark"/> : null}</span></button>
                <button aria-label={`归档会话：${topic}`} className="ai-conversation-archive" aria-busy={archivingIds.has(item.conversationId)} disabled={actionBusy || sendingDraftKeys.has(`conversation:${item.conversationId}`) || archivingIds.has(item.conversationId)} onClick={() => void archiveConversation(item.conversationId)} title={archivingIds.has(item.conversationId) ? "正在归档…" : "归档会话"} type="button">{archivingIds.has(item.conversationId) ? <span aria-label="正在归档" className="ai-archive-pending"/> : <ArchiveIcon/>}</button>
              </div>;
            })}
            {hidden > 0 && !collapsed
              ? <button className="ai-conversation-more" onClick={() => expandProject(group.projectKey)} type="button">展开显示</button>
              : null}
          </div>;
        })}
        {snapshotError ? <div className="ai-list-empty" role="alert"><strong>对话读取失败</strong><span>{snapshotError}</span><button type="button" onClick={() => void refresh()}>重试</button></div> : !snapshotLoaded ? <div className="ai-list-empty" role="status">正在读取对话…</div> : !conversations.length ? <div className="ai-list-empty"><ConversationIcon/><strong>还没有对话</strong></div> : null}
      </div>
      <div
        aria-label="调整对话记录宽度"
        aria-orientation="vertical"
        aria-valuemax={maxConversationRailWidth}
        aria-valuemin={minConversationRailWidth}
        aria-valuenow={conversationRailWidth}
        className="ai-conversation-resizer"
        onDoubleClick={() => applyConversationRailWidth(defaultConversationRailWidth, true)}
        onKeyDown={resizeConversationRailWithKeyboard}
        onPointerDown={startConversationRailResize}
        role="separator"
        tabIndex={0}
        title="拖动调整对话记录宽度；双击恢复默认宽度"/>
    </aside>

    <main className="ai-chat-stage" ref={chatStage}>
        <div className={`ai-conversation-shell${conversationInfoOpen ? " info-open" : ""}${conversationInfoOpen && conversationInfoDocked ? " info-docked" : " info-popover"}`}>
          <div className="ai-conversation-main">
        {failedOutgoing.map((item) => <div className="ai-recovered-send" data-failed="true" key={item.pendingId} role="alert"><div><strong>发送失败</strong><span>{aiErrorMessage(submissionsById.get(item.pendingId)?.reason ?? "")}</span><span>{item.content || item.attachments.map((attachment) => attachment.fileName).join("、")}</span></div><button type="button" onClick={() => void copyMessage(item.content)}>复制</button><button type="button" disabled={busy} onClick={() => restoreOutgoing(item)}>恢复草稿</button><button type="button" aria-label="移除发送失败记录" onClick={() => discardFailedOutgoing(item)}><CloseIcon/></button></div>)}
        {recoveredOutgoing.map((item) => <div className="ai-recovered-send" key={item.pendingId} role="status"><div><strong>上次发送未确认</strong><span>{item.content || item.attachments.map((attachment) => attachment.fileName).join("、")}</span></div><button type="button" onClick={() => void copyMessage(item.content)}>复制</button><button type="button" disabled={busy} onClick={() => restoreOutgoing(item)}>恢复草稿</button><button type="button" aria-label="移除恢复记录" onClick={() => setRecoveredOutgoing((current) => current.filter((entry) => entry.pendingId !== item.pendingId))}><CloseIcon/></button></div>)}
        <div className="ai-message-scroll-shell">
        <ConversationNavigator activeMessageId={activeNavigatorMessageId} messages={navigatorMessages} onJump={jumpToMessage}/>
        <div
          className={threadVisible ? "ai-message-list" : "ai-message-list ai-message-list-landing"}
          onKeyDown={handleMessageListKeyDown}
          onPointerDown={(event) => { if (event.target === event.currentTarget) markMessageListUserScroll(); }}
          onScroll={handleMessageListScroll}
          onTouchStart={markMessageListUserScroll}
          onWheel={markMessageListUserScroll}
          ref={messageList}>
          {threadVisible ? <>
          {handoff ? <aside aria-label="转交来源" className="ai-handoff-source">来源：{handoff.applicationName}</aside> : null}
          {visibleMessages.length > renderedMessageCount ? <button className="ai-history-more" type="button" onClick={() => { const root = messageList.current; if (root) historyScrollRestore.current = {height:root.scrollHeight,top:root.scrollTop}; markMessageListUserScroll(); setRenderedMessageCount((count) => count + 100); }}>加载更早消息</button> : null}
          {renderedMessages.map((message, renderedIndex) => {
            const messageIndex = visibleMessages.length - renderedMessages.length + renderedIndex;
            const messageRun = message.runId ? snapshot.runs.find((run) => run.runId === message.runId) : null;
            const isHandoffMessage = !message.runId && (message.messageId === handoffUserId || message.messageId === handoffAssistantId);
            const importedRun = isHandoffMessage ? handoff?.runSnapshot : null;
            const legacyPrefix = `来自「${handoff?.applicationName}」的页面 AI 对话\n\n`;
            const visibleContent = message.role === "assistant" ? assistantMessageBody(message.content, messageRun?.classificationNonce)
              : isHandoffMessage && !handoff?.userMessageId && message.content.startsWith(legacyPrefix) ? message.content.slice(legacyPrefix.length) : message.content;
            const decisionCard = message.role === "assistant" ? parseDecisionCard(visibleContent) : null;
            const installation = message.role === "assistant" && message.runId ? skillInstallations[message.runId] : undefined;
            const messageInstallation = installation?.messageId === message.messageId ? installation : undefined;
            const displayContent = messageInstallation ? skillMessageText(visibleContent) : visibleContent;
            const laterRunMessages = visibleMessages.slice(messageIndex + 1).filter((candidate) => candidate.runId === message.runId);
            const isFinalRunMessage = message.role === "assistant"
              ? !laterRunMessages.some((candidate) => candidate.role === "assistant")
              : message.messageId === messageRun?.inputMessageId;
            const responder = isHandoffMessage ? importedRun?.modelLabel ?? "原模型未记录" : messageRun?.executionTarget === "model"
              ? messageRun.modelLabel ?? catalog.models.find((model) => model.modelId === messageRun.model)?.label ?? (messageRun.model ? displayModelId(messageRun.model) : "模型")
              : snapshot.agents.find((agent) => agent.agentId === messageRun?.agentId)?.name ?? selectedAgent?.name ?? "AI";
            const editing = editingMessageId === message.messageId;
            const canEdit = message.role === "user"
              && message.messageId === latestEditableUserMessageId
              && !activeRun
              && !threadDraining
              && !conversationHasPendingFollowUps;
            const incompleteRun = message.role === "user"
              && messageRun
              && !activeRunStatuses.includes(messageRun.status)
              && messageRun.status !== "completed"
              && !messageRun.superseded
              && messageRun.partialOutput
              && laterRunMessages.length === 0
              ? messageRun
              : null;
            const historicApprovals = message.role === "user" && messageRun
              ? applicationApprovalsByRun.get(messageRun.runId) ?? []
              : [];
            const incompleteContent = incompleteRun ? assistantMessageBody(incompleteRun.partialOutput!, incompleteRun.classificationNonce) : "";
            return <Fragment key={message.messageId}><article aria-label={message.role === "user" ? "用户消息" : "AI 消息"} className={`ai-message ${message.role}${editing ? " editing" : ""}`} data-message-id={message.messageId}>
              <header><span>{message.role === "user" ? "你" : responder}</span>{isHandoffMessage
                ? importedRun ? <><time dateTime={new Date(message.createdAt).toISOString()} title={timelineMoment(message.createdAt)}>{clockMoment(message.createdAt)}</time>{message.role === "assistant" ? <RunElapsedTime completedAt={importedRun.completedAt} startedAt={importedRun.startedAt}/> : null}</> : <span className="ai-message-missing-time">原时间未记录</span>
                : message.role === "assistant" && messageRun && isFinalRunMessage
                ? <RunElapsedTime completedAt={messageRun.completedAt} startedAt={messageRun.startedAt}/>
                : <time>{clockMoment(message.createdAt)}</time>}</header>
              {!editing && message.referencedMaterials?.length ? <MaterialAttachments materials={message.referencedMaterials}/> : null}
              {!editing && message.attachments?.length ? <MessageAttachments attachments={message.attachments}/> : null}
              {message.role === "assistant" && messageRun && isFinalRunMessage ? <RunExecutionProcess run={messageRun}/> : null}
              {message.role === "assistant" && importedRun && handoff ? <RunExecutionProcess readOnly run={{ runId: handoff.runId, events: importedRun.events }}/> : null}
              {!editing && displayContent ? <div>{message.role === "assistant" ? <MarkdownContent canOpenLink={canOpenConversationLink} diagramIdPrefix="conversation" onContextLink={openConversationLinkContextMenu} onOpenLink={openConversationLink}>{displayContent}</MarkdownContent> : displayContent}</div> : null}
              {editing ? <form className="ai-message-editor" onSubmit={(event) => { event.preventDefault(); void regenerateMessage(message.messageId); }}>
                <textarea
                  aria-label="编辑最近一条消息"
                  autoFocus
                  disabled={busy}
                  onChange={(event) => setMessageEditDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") {
                      event.preventDefault();
                      setEditingMessageId(null);
                      setMessageEditDraft("");
                    } else if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) {
                      event.preventDefault();
                      void regenerateMessage(message.messageId);
                    }
                  }}
                  ref={messageEditBox}
                  rows={2}
                  value={messageEditDraft}/>
                <footer><button disabled={busy} onClick={() => { setEditingMessageId(null); setMessageEditDraft(""); }} type="button">取消</button><button className="primary" disabled={busy || Boolean(activeRun) || threadDraining || conversationHasPendingFollowUps || !messageEditDraft.trim() || !selectedRuntimeReady || starterPermissionSelectionInvalid} type="submit">{busy ? "正在生成…" : "重新生成"}</button></footer>
              </form> : null}
              {!editing && displayContent ? <nav aria-label="消息操作" className="ai-message-actions">
                <button aria-label="复制对话内容" onClick={() => void copyMessage(displayContent)} title="复制" type="button"><CopyIcon/></button>
                {message.role === "assistant" && messageRun?.status === "completed" && !messageRun.superseded && isFinalRunMessage && conversation && props.onReuseWork ? <button aria-label="复用这次工作" title="复用" type="button" onClick={(event) => {
                  const intent: WorkReuseIntent = { token: randomUuid(), title: conversation.title, conversationId: conversation.conversationId, projectId: conversation.context.projectId, agentId: messageRun.agentId, modelId: messageRun.executionTarget === "model" ? messageRun.model : null, input: messages.find((entry) => entry.messageId === messageRun.inputMessageId)?.content ?? "", output: displayContent };
                  setContextMenu(openContextMenu(event, [{ label: "创建技能", onSelect: () => props.onReuseWork?.("skill", intent) }, { label: "创建自动化", onSelect: () => props.onReuseWork?.("automation", intent) }]));
                }}><ReuseIcon/></button> : null}
                {canEdit ? <button aria-label="编辑最近一条消息并重新生成" disabled={busy} onClick={() => { setEditingMessageId(message.messageId); setMessageEditDraft(message.content); }} title="编辑并重新生成" type="button"><EditIcon/></button> : null}
              </nav> : null}
              {decisionCard && conversation?.context.projectId ? <footer className="ai-decision-action"><span>已识别六部分结构化决策卡</span><button disabled={Boolean(savingDecisionMessageId)} onClick={() => void saveDecisionCard(message.messageId, visibleContent)} type="button">{savingDecisionMessageId === message.messageId ? "正在保存…" : "保存为决策请求"}</button></footer> : null}
              {messageInstallation ? <SkillInstallationStatus installation={messageInstallation} onRetry={() => retrySkillInstallation(messageInstallation.runId)} onView={props.onOpenSkill} onTry={(id, name) => {
                selectConversationSkill(id);
                setDraft(`使用${name}，`);
                draftBox.current?.focus();
              }}/> : null}
              {skillStatusError && message.role === "assistant" && /```cilai-skill\b/i.test(visibleContent) ? <footer className="ai-skill-draft-action"><span>技能状态读取失败</span><details><summary>详情</summary><p>{skillStatusError}</p></details></footer> : null}
              {message.role === "assistant" && messageRun && isFinalRunMessage ? <RunArtifacts onContext={openArtifactContextMenu} onOpen={(artifact) => void openConversationArtifact(messageRun.runId, artifact)} run={messageRun}/> : null}
            </article>{historicApprovals.map((request) => <ConversationApprovalCard busy={false} key={request.permissionRequestId} request={request}/>) }{incompleteRun ? <article aria-label="未完成的 AI 消息" className="ai-message assistant ai-message-incomplete">
              <header><span>{incompleteRun.modelLabel ?? catalog.models.find((model) => model.modelId === incompleteRun.model)?.label ?? displayModelId(incompleteRun.model ?? "")} · 回复未完成</span><RunElapsedTime completedAt={incompleteRun.completedAt} startedAt={incompleteRun.startedAt}/></header>
              <RunExecutionProcess run={incompleteRun}/>
              <div><MarkdownContent canOpenLink={canOpenConversationLink} diagramIdPrefix="conversation-incomplete" onContextLink={openConversationLinkContextMenu} onOpenLink={openConversationLink}>{incompleteContent}</MarkdownContent></div>
              <footer className="ai-incomplete-run-note"><RunRecordIcon status={incompleteRun.status}/><span>{incompleteRunSummary(incompleteRun)}，已保留已生成内容</span></footer>
              {incompleteRun.errorReason ? <details><summary>详情</summary><p>{incompleteRun.errorReason}</p></details> : null}
              <nav aria-label="消息操作" className="ai-message-actions"><button aria-label="复制未完成的回复" onClick={() => void copyMessage(incompleteContent)} title="复制" type="button"><CopyIcon/></button></nav>
              {incompleteRun.artifacts.length ? <RunArtifacts onContext={openArtifactContextMenu} onOpen={(artifact) => void openConversationArtifact(incompleteRun.runId, artifact)} run={incompleteRun}/> : null}
            </article> : null}{messageIndex === pendingReplyAnchor ? pendingReply : null}</Fragment>;
          })}
          {pendingApproval ? <ConversationApprovalCard busy={busy} onDecision={(decision, applicationWriteArguments) => void resolveApproval(decision, applicationWriteArguments)} request={pendingApproval}/> : null}
          {pendingReplyAnchor < 0 ? pendingReply : null}
          <ConversationRun hasAssistantMessage={Boolean(latestConversationRun && messages.some((message) => message.role === "assistant" && message.runId === latestConversationRun.runId))} key={activeRun ? activeRun.runId : latestConversationRun?.runId ?? "no-run"} modelLabel={latestConversationRun?.modelLabel ?? catalog.models.find((model) => model.modelId === latestConversationRun?.model)?.label ?? null} run={waitingForRunFeedback || activeRun ? null : latestConversationRun}/>
          </> : snapshotLoaded ? <div className="ai-starter-content">
            <header className="ai-starter-heading"><h2>{pendingCreate?.title === "新建任务" ? "描述你要完成的事" : "Hi，今天有什么安排？"}</h2></header>
            <section className="ai-starter-businesses" aria-labelledby="ai-starter-business-title">
              <header><h3 id="ai-starter-business-title">选择一项业务开始</h3><p>在当前对话中处理常用工作</p></header>
              <div className="ai-starter-business-grid">{starterBusinesses.map((business) => <button
                aria-label={`开始${business.name}业务`}
                aria-pressed={starterBusinessId === business.id}
                className={starterBusinessId === business.id ? "selected" : ""}
                disabled={busy}
                key={business.id}
                onClick={() => selectStarterBusiness(business.id)}
                type="button">
                <span className="ai-starter-business-icon">{business.id === "search" ? <SearchBusinessIcon/> : business.id === "write" ? <WriteBusinessIcon/> : business.id === "fill" ? <FillBusinessIcon/> : <AdvanceBusinessIcon/>}</span>
                <span className="ai-starter-business-copy"><strong>{business.name}</strong><small>{business.description}</small></span>
                <ArrowRightIcon/>
              </button>)}</div>
              {!snapshot.agents.length ? <div className="ai-starter-no-agents"><strong>还没有可用的智能体</strong><span>先创建一个智能体，再回来执行上述业务。</span><button onClick={props.onOpenAgents} type="button">前往创建</button></div> : null}
            </section>
          </div> : null}
        </div>
        {!followingLatestOutput ? <button aria-label={unseenOutput ? "回到底部，有新输出" : "回到底部"} className="ai-return-to-latest" onClick={returnToLatestOutput} title="回到底部" type="button"><ArrowDownIcon/>{unseenOutput ? <i aria-hidden="true"/> : null}</button> : null}
        </div>
        {/* 排队区贴在输入框上方，与它同属输入区：这些是还没发出去的内容，不是对话记录。
            放在滚动区里的时候，往上翻几屏就看不见自己排了什么了。 */}
        {conversationQueuedFollowUps.length ? <QueuedFollowUps
          busy={busy}
          supplementUnavailableReason={supplementUnavailableReason}
          items={conversationQueuedFollowUps}
          onDelete={(queueId) => void deleteQueuedFollowUp(queueId)}
          onEdit={editQueuedFollowUp}
          onRetry={(queueId) => void retryQueuedFollowUp(queueId)}
          onSteer={(queueId) => void steerQueuedFollowUp(queueId)}/> : null}
        {/* 新建对话与进行中的对话共用这一个输入框：提交时按对话是否已开始决定是新建还是追加。
            此前起始页另有一个输入框，发出第一句话时整块被换掉，输入框、消息和工具按钮都会跳一下。 */}
        <ModelCatalogNotice catalog={catalog} variant="conversation"/>
        {catalog.loaded && catalog.status?.configured === false ? <div className="model-catalog-error model-catalog-error-conversation" role="status"><span>尚未连接模型，草稿和附件会保留。</span><button type="button" onClick={props.onOpenAgents}>连接模型</button></div> : null}
        <form className="ai-composer" onSubmit={(event) => { event.preventDefault(); void submitComposer(); }}>
          {showWindowsSandboxGate ? <WindowsSandboxGate loading={windowsSandboxLoading} onOpenSettings={props.onOpenAgents} settingUp={windowsSandboxSettingUp} status={windowsSandboxStatus}/> : null}
          {props.hostPlatform === "windows" && permissionWorkspace ? <WorkspacePermissions key={permissionWorkspace.workspaceId} workspaceId={permissionWorkspace.workspaceId} compact disabled={busy || Boolean(activeRun)}/> : null}
          {draftMaterials.length ? <MaterialAttachments materials={draftMaterials} onRemove={(material) => setDraftMaterials(draftMaterials.filter((item) => item !== material))}/> : null}
          {draftAttachments.length ? <DraftAttachments items={draftAttachments} onRemove={(attachmentId) => void removeDraftAttachment(attachmentId)}/> : null}
          {slashInvocationQuery !== null ? <SlashInvocationPalette choices={selectableSlashInvocationChoices} onSelect={selectSlashInvocation}/> : null}
          <div className={`ai-composer-input${applicationContext || selectedConversationSkill ? " has-invocation" : ""}`} onFocusCapture={() => {
            if (editingMessageId) {
              setEditingMessageId(null);
              setMessageEditDraft("");
            }
          }}>
            {applicationContext || selectedConversationSkill ? <span className="ai-invocation-input-prefix">
              {applicationContext ? <ApplicationContextChip context={applicationContext} disabled={busy || Boolean(activeRun)} onRemove={() => setApplicationContextSelections((current) => { const next = { ...current }; delete next[draftKey]; return next; })}/> : null}
              {selectedConversationSkill ? <SkillInputChip disabled={busy || Boolean(activeRun)} onRemove={removeSelectedConversationSkill} skill={selectedConversationSkill}/> : null}
            </span> : null}
            {applicationContext || selectedConversationSkill ? <InlineInvocationEditor ariaLabel={composerLabel} disabled={composerLocked} inputRef={draftBox} onBackspaceWhenEmpty={selectedConversationSkill ? removeSelectedConversationSkill : undefined} onChange={setDraft} onKeyDown={handleComposerKeyDown} onPaste={(event) => { const files = Array.from(event.clipboardData.files).filter((file) => isSupportedMedia(file) || isDocumentAttachment(file)); if (files.length) { event.preventDefault(); void addMediaFiles(files); } }} placeholder={composerPlaceholder} value={draft}/> : <ConversationInput aria-label={composerLabel} disabled={composerLocked} onChange={(event) => setDraft(event.target.value)} onKeyDown={handleComposerKeyDown} onPaste={(event) => { const files = Array.from(event.clipboardData.files).filter((file) => isSupportedMedia(file) || isDocumentAttachment(file)); if (files.length) { event.preventDefault(); void addMediaFiles(files); } }} placeholder={composerPlaceholder} ref={(node) => { draftBox.current = node; }} rows={1} value={draft}/>}
          </div>
          {/* 上下文与动作合成输入框底部的同一行：左边是这次带上的东西，右边是模式与发送。
              原先它们各占一行，输入框因此有两条几乎空着的横带。只读边界不在这一行呈现——
              它是一条由后端强制执行的恒定约束，不靠界面文字生效，写在这里只是每次发消息
              都要重读一遍的噪音。 */}
          <footer>
            <ComposerAddMenu
              canAttachMedia={canAttachMedia || Boolean(starterModelOverride)}
              canAttachVideo={canAttachVideo}
              mediaDescription={starterModelOverride ? "文档、图片和短视频" : mediaDescription}
              disabled={busy}
              hasFile={Boolean(draftMaterials.length)}
              hasWorkspaceFiles={Boolean(props.workspaces?.length) || props.files.some((file) => file.fileType !== "directory" && file.presenceState === "present")}
              onOpenFiles={() => setFilePickerOpen(true)}
              onOpenMedia={() => mediaPicker.current?.click()}
              workspaceFileDisabled={false}/>
            {conversationStarted && conversation ? contextChips(conversation.context).map((chip) => <span className="ai-context-chip" key={chip.label} title={`${chip.label}：${chip.value}`}>
              <span>{chip.label}：{chip.display}</span>
              <button aria-label={`移除${chip.label}上下文`} disabled={busy || Boolean(activeRun)} onClick={() => void updateContext(chip.clear(conversation.context))} type="button"><CloseIcon/></button>
            </span>) : null}
            <div className="ai-composer-actions">
              <StarterModelPicker displayModel={catalog.error ? catalogModel(catalog.displayModels, starterModelOverride, starterConnectionId) ?? undefined : undefined} onOpen={catalog.refreshOnOpen} historical={Boolean(conversation)} loading={!catalog.loaded && !catalog.error && (!snapshotLoaded || catalog.loading)} agents={snapshot.agents} disabled={busy || !snapshotLoaded} models={catalog.models} onSelectModel={selectStarterModel} selectedAgentId={starterAgentId} selectedModelId={starterModelOverride} selectedConnectionId={starterConnectionId} selectedReasoningEffort={starterReasoningEffort}/>
              <StarterPermissionPicker disabled={busy} hostPlatform={props.hostPlatform} modelMode={Boolean(starterModelOverride)} onChange={selectStarterPermissionMode} value={effectiveStarterPermissionMode}/>
              {conversationStarted ? <ComposerActionControl
                activeRun={activeRun}
                threadDraining={threadDraining || mustQueue}
                busy={busy}
                draftReady={Boolean(draft.trim())}
                hasAttachments={Boolean(draftAttachments.length || draftMaterials.length)}
                pendingApproval={Boolean(pendingApproval)}
                runtimeReady={selectedRuntimeReady && !starterReasoningSelectionInvalid && !starterPermissionSelectionInvalid}
                onQueue={() => void queueFollowUp()}
                onStop={() => void stopActiveRun()}/> : <button aria-label="开始对话" className="ai-send-button" disabled={(!conversation && !pendingCreate) || !starterAgentId || !starterRuntimeReady || starterReasoningSelectionInvalid || starterPermissionSelectionInvalid || (!draft.trim() && !draftAttachments.length && !draftMaterials.length) || busy} title={!snapshotLoaded ? "正在加载对话…" : !conversation && !pendingCreate ? "请先新建对话" : !starterAgentId ? "请选择模型" : starterPermissionSelectionInvalid ? "请选择审批模式" : starterReasoningSelectionInvalid ? "请重新选择思考深度" : !starterRuntimeReady ? starterModel && props.hostPlatform === "windows" ? "请先完成 Windows 安全沙箱配置" : "所选模型当前不可用" : "开始对话"} type="submit"><EnterIcon/></button>}
            </div>
          </footer>
          {!conversationStarted && starterProjectEditable && pendingCreate ? <footer className="ai-starter-project-row">
            <StarterProjectPicker canCreate={props.canCreateProject} disabled={busy} onChange={(projectId) => {
              if (projectId === null) {
                setPendingCreate({ ...pendingCreate, context: unclassifiedConversationContext() });
                return;
              }
              const project = props.projects.find((item) => item.projectId === projectId);
              if (project) confirmCreate(project, pendingCreate, "editable");
            }} onCreate={props.onCreateProject} project={selectedStarterProject} projects={props.projects}/>
          </footer> : null}
        </form>
          </div>
          <div className="ai-conversation-tools">
            <ConversationContextUsagePopover conversationId={conversation?.conversationId ?? null} runs={snapshot.runs}/>
            <button
              aria-controls="ai-conversation-info-panel"
              aria-expanded={conversationInfoOpen}
              aria-label={conversationInfoOpen ? "隐藏产物与来源" : "显示产物与来源"}
              className="ai-conversation-info-toggle"
              onClick={() => setConversationInfoOpen((current) => !current)}
              data-tip={conversationInfoOpen ? "隐藏产物与来源" : "产物与来源"}
              type="button"><ArtifactsSourcesIcon open={conversationInfoOpen}/></button>
            <ConversationRailToggle expanded={conversationRailOpen} onToggle={() => setConversationRailOpen((current) => !current)}/>
            {conversationStarted && props.aiBrowser
              ? <AiBrowserToggle onToggle={props.aiBrowser.onToggle} open={props.aiBrowser.open} state={props.aiBrowser.state}/>
              : null}
          </div>
          {conversationInfoOpen && infoPanelContext ? <ConversationInfoPanel
            context={infoPanelContext}
            files={props.files} workspace={props.workspace} onRemoveMaterial={removeMaterial}
            onAddContext={() => setFilePickerOpen(true)}
            onOpenArtifact={(runId, artifact) => void openConversationArtifact(runId, artifact)}
            onContextArtifact={openArtifactContextMenu}
            runs={conversationRuns}/> : null}
        </div>
    </main>

    {historyTransfer.dialog}
    {renamingConversation ? <ConfirmDialog
      busy={actionBusy} confirmDisabled={!conversationName.trim()} confirmLabel="保存" title="重命名会话"
      description={<label className="ai-conversation-name">名称<input aria-label="会话名称" autoFocus maxLength={80} value={conversationName} onChange={(event) => setConversationName(event.target.value)}/></label>}
      onCancel={() => setRenamingConversation(null)} onConfirm={() => { void (async () => {
        setBusy(true);
        try {
          await updateAiConversation({ conversationId: renamingConversation.conversationId, title: conversationName.trim(), mode: null, agentId: null, context: null, archived: null });
          await refresh(); setRenamingConversation(null);
        } catch (error) { props.onMessage(aiErrorMessage(error), "error"); }
        finally { setBusy(false); }
      })(); }}/> : null}
    {pendingArchive ? <ConfirmDialog
      busy={pendingArchive.ids.some((id) => archivingIds.has(id))}
      confirmLabel="归档"
      title="归档这个项目的全部会话？"
      onCancel={() => setPendingArchive(null)}
      onConfirm={() => void archiveProjectConversations(pendingArchive.ids)}/> : null}
    {confirmWriteSend ? <ConfirmDialog
      busy={busy}
      confirmLabel="确认并运行"
      description={<><p>本次运行的文件工具可以在当前工作区中创建或编辑文件。</p><p>运行仍不得访问网络或凭据，也不会自动改变项目、任务、阶段或交付物的正式状态。请确认本次任务确实需要修改文件。</p></>}
      title="允许修改工作区文件？"
      onCancel={() => setConfirmWriteSend(false)}
      onConfirm={() => { setConfirmWriteSend(false); void startConversation(true); }}/>: null}
    {filePickerOpen ? <StarterFilePicker workspaces={props.workspaces} currentWorkspaceId={props.workspace?.workspaceId ?? null} files={props.files} selectedFileId={draftMaterials.at(-1)?.localFileId ?? null} onCancel={() => setFilePickerOpen(false)} onPick={(files) => {
      setFilePickerOpen(false);
      const materials = new Map(draftMaterials.map((item) => [`${item.workspaceId ?? ""}:${item.localFileId}`, item]));
      for (const file of files) {
        const workspaceId = file.sourceWorkspaceId ?? props.workspace?.workspaceId ?? null;
        materials.set(`${workspaceId ?? ""}:${file.localFileId}`, { workspaceId, localFileId: file.localFileId, relativePath: file.relativePath, fileRevision: file.stateRevision });
      }
      setDraftMaterials([...materials.values()]);
    }}/> : null}
    <input accept={(starterModelOverride ? ".pdf,.docx,.xlsx,.pptx,.txt,.md,.csv,.json," : "") + (canAttachVideo ? "image/png,image/jpeg,image/webp,image/gif,video/mp4,video/quicktime,video/webm" : "image/png,image/jpeg,image/webp,image/gif")} aria-hidden="true" className="ai-media-input" multiple onChange={(event) => void addMediaFiles(Array.from(event.currentTarget.files ?? []))} ref={mediaPicker} tabIndex={-1} type="file"/>
    <ContextMenu state={contextMenu} onClose={() => { setContextMenu(null); setOpenProjectMenuKey(null); }}/>
    {projectPickerOpen && !props.projectCreationOpen ? <ConversationProjectPicker
      projects={pendingCreate?.projectIds ? props.projects.filter((project) => pendingCreate.projectIds?.includes(project.projectId)) : props.projects}
      busy={busy}
      canCreateProject={!pendingCreate?.projectIds && props.canCreateProject}
      onCreateProject={props.onCreateProject}
      onCancel={() => { setPendingCreate(null); setProjectPickerOpen(false); }}
      onPick={(project) => confirmCreate(project)}/> : null}
  </section>;
}

type MaterialFile = WorkspaceFileItem & { sourceWorkspaceId?: string };
function StarterFilePicker(props: { workspaces?: WorkspaceSummary[]; currentWorkspaceId?: string | null; files: WorkspaceFileItem[]; selectedFileId: string | null; onCancel: () => void; onPick: (files: MaterialFile[]) => void }) {
  const [selected, setSelected] = useState<Map<string, MaterialFile>>(new Map());
  const [workspaceId, setWorkspaceId] = useState(props.currentWorkspaceId ?? "");
  const [sourceFiles, setSourceFiles] = useState<MaterialFile[]>(() => props.files.map((file) => ({ ...file, sourceWorkspaceId: props.currentWorkspaceId ?? undefined })));
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    if (!workspaceId || workspaceId === props.currentWorkspaceId) { setLoading(false); setLoadError(null); setSourceFiles(props.files.map((file) => ({ ...file, sourceWorkspaceId: workspaceId || undefined }))); return; }
    let current = true; setLoading(true); setLoadError(null); setSourceFiles([]);
    void loadWorkspaceFileSnapshot(workspaceId, () => current).then((snapshot) => { if (current && snapshot) setSourceFiles(snapshot.files.map((file) => ({ ...file, sourceWorkspaceId: workspaceId }))); }).catch((error) => { if (current) setLoadError(String(error)); }).finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
  }, [workspaceId, props.currentWorkspaceId, props.files]);
  const [query, setQuery] = useState("");
  const [visibleCount, setVisibleCount] = useState(100);
  const modalRef = useModal<HTMLDivElement>({ onClose: props.onCancel, busy: false });
  const normalized = query.trim().toLocaleLowerCase("zh-CN");
  const files = useMemo(() => sourceFiles
    .filter((file) => file.fileType !== "directory" && file.presenceState === "present")
    .filter((file) => !normalized || file.relativePath.toLocaleLowerCase("zh-CN").includes(normalized)), [sourceFiles, normalized]);

  return <div aria-label="选择工作区文件" aria-modal="true" className="agent-picker-backdrop" ref={modalRef} role="dialog">
    <section className="starter-file-picker">
      <header><div><h3>添加材料</h3></div><button aria-label="关闭文件选择" onClick={props.onCancel} type="button"><CloseIcon/></button></header>
      {props.workspaces?.length ? <Select ariaLabel="材料来源" value={workspaceId} onChange={setWorkspaceId} options={props.workspaces.map((workspace) => ({value:workspace.workspaceId,label:`${workspace.displayName} · ${workspace.workspaceKind === "cloud_team" ? "云端" : "本地"}`}))}/> : null}
      {loading ? <p role="status">正在读取…</p> : null}{loadError ? <p role="alert">{loadError}</p> : null}
      <label className="starter-file-search" data-focus-frame="wrapper"><SearchBusinessIcon/><input aria-label="搜索工作区文件" autoFocus onChange={(event) => { setQuery(event.target.value); setVisibleCount(100); }} placeholder="按文件名或路径搜索" value={query}/></label>
      <div className="starter-file-list">{files.slice(0, visibleCount).map((file) => <button aria-current={file.localFileId === props.selectedFileId ? "true" : undefined} key={file.localFileId} aria-pressed={selected.has(`${file.sourceWorkspaceId ?? ""}:${file.localFileId}`)} onClick={() => setSelected((current) => { const next = new Map(current); const key = `${file.sourceWorkspaceId ?? ""}:${file.localFileId}`; if (next.has(key)) next.delete(key); else next.set(key, file); return next; })} title={file.relativePath} type="button"><FileIcon/><span><strong>{fileNameFromPath(file.relativePath)}</strong><small>{file.relativePath}</small></span>{selected.has(`${file.sourceWorkspaceId ?? ""}:${file.localFileId}`) ? <CheckIcon/> : null}</button>)}</div>
      {files.length > visibleCount ? <button className="starter-file-more" type="button" onClick={() => setVisibleCount((value) => value + 100)}>继续显示 · {visibleCount}/{files.length}</button> : null}
      <footer className="starter-file-actions"><span>{selected.size} 项</span><button disabled={!selected.size} onClick={() => props.onPick([...selected.values()].sort((a,b) => `${a.sourceWorkspaceId ?? ""}/${a.relativePath}`.localeCompare(`${b.sourceWorkspaceId ?? ""}/${b.relativePath}`, "zh-CN")))} type="button">添加</button></footer>
      {!files.length ? <p className="starter-file-empty">当前工作区没有匹配的可用文件。</p> : null}
    </section>
  </div>;
}

function MaterialAttachments({ materials, onRemove }: { materials: AiMaterialReference[]; onRemove?: (material: AiMaterialReference) => void }) {
  return <div aria-label={onRemove ? "待发送文件" : "消息文件"} className={onRemove ? "ai-draft-attachments" : "ai-message-attachments"}>{materials.map((material) => <div className="ai-draft-attachment ai-file-attachment" key={`${material.workspaceId ?? ""}:${material.localFileId}:${material.fileRevision}`} title={material.relativePath}>
    <span className="ai-document-attachment"><FileIcon/><span>{fileNameFromPath(material.relativePath)}</span></span>
    {onRemove ? <button aria-label={`移除文件 ${fileNameFromPath(material.relativePath)}`} className="ai-attachment-remove" onClick={() => onRemove(material)} type="button"><CloseIcon/></button> : null}
  </div>)}</div>;
}

function DraftAttachments({ items, onRemove, removable = true }: { items: DraftAttachment[]; onRemove: (attachmentId: string) => void; removable?: boolean }) {
  return <div aria-label="待发送附件" className="ai-draft-attachments">{items.map(({ attachment, previewUrl }) => <div className={`ai-draft-attachment${attachment.mediaType === "document" ? " ai-file-attachment" : ""}`} key={attachment.attachmentId}>
    <>{attachment.mediaType === "document" ? <span className="ai-document-attachment"><FileIcon/><span>{attachment.fileName}<small>{Math.ceil(attachment.sizeBytes / 1024)} KB</small></span></span> : previewUrl ? <MediaThumbnail attachment={attachment} previewUrl={previewUrl}/> : <StoredAttachmentThumbnail attachment={attachment}/>}</>
    {removable ? <button aria-label={`移除附件 ${attachment.fileName}`} className="ai-attachment-remove" onClick={() => onRemove(attachment.attachmentId)} type="button"><CloseIcon/></button> : null}
  </div>)}</div>;
}

export function MessageAttachments({ attachments }: { attachments: AiMessageAttachment[] }) {
  return <div aria-label="消息附件" className="ai-message-attachments">{attachments.map((attachment) => attachment.mediaType === "document" ? <span className="ai-document-attachment" key={attachment.attachmentId}><FileIcon/><span>{attachment.fileName}<small>已提供 · 读取记录见执行过程</small></span></span> : <StoredAttachmentThumbnail attachment={attachment} key={attachment.attachmentId}/>)}</div>;
}

function StoredAttachmentThumbnail({ attachment }: { attachment: AiMessageAttachment }) {
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    let objectUrl: string | null = null;
    void readAiAttachment(attachment.attachmentId).then((bytes) => {
      if (disposed) return;
      objectUrl = URL.createObjectURL(new Blob([bytes], { type: attachment.mimeType }));
      setPreviewUrl(objectUrl);
    }).catch(() => { if (!disposed) setPreviewUrl(""); });
    return () => {
      disposed = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [attachment.attachmentId, attachment.mimeType]);
  if (previewUrl === null) return <span aria-label={`正在加载 ${attachment.fileName}`} className="ai-attachment-thumbnail loading"/>;
  if (!previewUrl) return <span aria-label={`无法显示 ${attachment.fileName}`} className="ai-attachment-thumbnail unavailable"><MediaIcon/></span>;
  return <MediaThumbnail attachment={attachment} previewUrl={previewUrl}/>;
}

function MediaThumbnail({ attachment, previewUrl }: { attachment: AiMessageAttachment; previewUrl: string }) {
  const [open, setOpen] = useState(false);
  return <>
    <button aria-label={`查看${attachment.mediaType === "image" ? "图片" : "视频"} ${attachment.fileName}`} className="ai-attachment-thumbnail" onClick={() => setOpen(true)} title="点击放大查看" type="button">
      {attachment.mediaType === "image"
        ? <img alt={attachment.fileName} src={previewUrl}/>
        : <video aria-label={attachment.fileName} muted playsInline preload="metadata" src={previewUrl}/>}
    </button>
    {open ? <MediaLightbox attachment={attachment} onClose={() => setOpen(false)} previewUrl={previewUrl}/> : null}
  </>;
}

function MediaLightbox({ attachment, onClose, previewUrl }: { attachment: AiMessageAttachment; onClose: () => void; previewUrl: string }) {
  const modalRef = useModal<HTMLDivElement>({ onClose });
  return <div aria-label={`查看${attachment.mediaType === "image" ? "图片" : "视频"} ${attachment.fileName}`} aria-modal="true" className="ai-media-lightbox" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }} ref={modalRef} role="dialog">
    <section>
      <button aria-label="关闭媒体预览" autoFocus onClick={onClose} type="button"><CloseIcon/></button>
      {attachment.mediaType === "image"
        ? <img alt={attachment.fileName} src={previewUrl}/>
        : <video aria-label={attachment.fileName} autoPlay controls playsInline src={previewUrl}/>}
    </section>
  </div>;
}

function editableDraftText(node: HTMLElement) {
  return (node.innerText ?? node.textContent ?? "").replaceAll("\r", "");
}

function placeCaretAtEnd(node: HTMLElement) {
  const selection = window.getSelection();
  if (!selection) return;
  const range = document.createRange();
  range.selectNodeContents(node);
  range.collapse(false);
  selection.removeAllRanges();
  selection.addRange(range);
}

function InlineInvocationEditor(props: {
  ariaLabel: string;
  disabled: boolean;
  inputRef: { current: HTMLTextAreaElement | HTMLSpanElement | null };
  onBackspaceWhenEmpty?: () => void;
  onChange: (value: string) => void;
  onKeyDown: (event: React.KeyboardEvent<HTMLSpanElement>) => void;
  onPaste: (event: React.ClipboardEvent<HTMLSpanElement>) => void;
  placeholder: string;
  value: string;
}) {
  const editor = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const node = editor.current;
    if (!node || editableDraftText(node) === props.value) return;
    node.textContent = props.value;
    if (document.activeElement === node) placeCaretAtEnd(node);
  }, [props.value]);
  return <span
    aria-disabled={props.disabled || undefined}
    aria-label={props.ariaLabel}
    aria-multiline="true"
    className="ai-inline-draft"
    contentEditable={!props.disabled}
    data-placeholder={props.placeholder}
    onInput={(event) => props.onChange(editableDraftText(event.currentTarget))}
    onKeyDown={(event) => {
      if (event.key === "Backspace" && props.onBackspaceWhenEmpty && !editableDraftText(event.currentTarget)) {
        event.preventDefault();
        props.onBackspaceWhenEmpty();
        return;
      }
      props.onKeyDown(event);
    }}
    onPaste={props.onPaste}
    ref={(node) => { editor.current = node; props.inputRef.current = node; }}
    role="textbox"
    spellCheck="true"
    suppressContentEditableWarning
    tabIndex={props.disabled ? undefined : 0}/>;
}

function ApplicationContextChip(props: {
  context: ApplicationContextSelection;
  disabled: boolean;
  onRemove: () => void;
}) {
  return <span className="ai-invocation-input-chip" title={props.context.applicationId}>
    <ApplicationContextIcon/><span>旧应用引用</span>
    <button aria-label="移除旧应用引用" disabled={props.disabled} onClick={props.onRemove} type="button"><CloseIcon/></button>
  </span>;
}

function SkillInputChip(props: {
  skill: SkillConversationCatalogItem;
  disabled: boolean;
  onRemove: () => void;
}) {
  return <span className="ai-invocation-input-chip ai-skill-input-chip" title={`本轮使用技能：${props.skill.name}`}>
    <SkillIcon/><span>{props.skill.name}</span>
    <button aria-label={`移除技能 ${props.skill.name}`} disabled={props.disabled} onClick={props.onRemove} type="button"><CloseIcon/></button>
  </span>;
}

function SlashInvocationPalette(props: {
  choices: SlashInvocationChoice[];
  onSelect: (choice: SlashInvocationChoice) => void;
}) {
  return <div aria-label="选择技能" className="ai-slash-palette">
    <header><strong>选择技能</strong><small>仅本轮生效</small></header>
    {props.choices.length ? <section aria-label="技能"><h3>技能</h3>{props.choices.map((choice) => {
      const sourceLabel = choice.source === "system" ? "系统" : "个人";
      return <button aria-label={`${choice.command} ${sourceLabel} ${choice.description}`}
        key={choice.skillId} onClick={() => props.onSelect(choice)} type="button">
        <SkillIcon/>
        <span className="ai-slash-choice-copy"><strong>{choice.command}</strong><small>{choice.description}</small></span>
        <span aria-label={`${sourceLabel}技能`} className="ai-skill-source" data-source={choice.source}>{sourceLabel}</span>
      </button>;
    })}</section> : <p>没有匹配的技能。</p>}
  </div>;
}

function ApplicationContextIcon() {
  return <svg aria-hidden="true" viewBox="0 0 24 24"><rect fill="none" height="15" rx="2.5" stroke="currentColor" strokeWidth="1.7" width="18" x="3" y="4.5"/><path d="M3 9h18M7 7h.01M10 7h.01M8 13h3m-3 3h8" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.7"/></svg>;
}

function ComposerAddMenu(props: {
  canAttachMedia: boolean;
  canAttachVideo: boolean;
  mediaDescription: string;
  disabled: boolean;
  workspaceFileDisabled?: boolean;
  hasFile: boolean;
  hasWorkspaceFiles: boolean;
  onOpenFiles: () => void;
  onOpenMedia: () => void;
}) {

  return <RichMenu
    align="start"
    ariaLabel="添加上下文"
    className="ai-add-menu"
    maxHeight={420}
    width={380}
    trigger={({ open, toggle, triggerRef }) => <button
      aria-expanded={open}
      aria-haspopup="menu"
      aria-label="添加上下文"
      aria-pressed={props.hasFile}
      className={`ai-context-add${props.hasFile ? " selected" : ""}`}
      disabled={props.disabled}
      onClick={toggle}
      ref={triggerRef}
      title="添加附件或工作区文件"
      type="button"><PlusIcon/></button>}>
    <RichMenuSection label="添加">
      <RichMenuItem
        disabled={!props.canAttachMedia}
        description={props.mediaDescription}
        label="附件"
        leading={<MediaIcon/>}
        onSelect={props.onOpenMedia}/>
      <RichMenuItem
        disabled={props.workspaceFileDisabled || !props.hasWorkspaceFiles}
        description={props.workspaceFileDisabled ? "当前回复结束后可以更改工作区文件" : props.hasWorkspaceFiles ? "从当前工作区选择文件" : "当前工作区没有可用文件"}
        label="工作区文件"
        leading={<PaperclipIcon/>}
        onSelect={props.onOpenFiles}/>
    </RichMenuSection>
  </RichMenu>;
}

function StarterModelPicker(props: {
  displayModel?: AiRuntimeSnapshot["models"][number];
  onOpen: () => void;
  historical: boolean;
  loading: boolean;
  agents: AiRuntimeSnapshot["agents"];
  models: AiRuntimeSnapshot["models"];
  selectedAgentId: string | null;
  selectedModelId: string | null;
  selectedConnectionId: string | null;
  selectedReasoningEffort: AiReasoningEffort | null;
  disabled: boolean;
  onSelectModel: (modelId: string, agentId: string, reasoningEffort: AiReasoningEffort | null, connectionId?: string | null) => void;
}) {
  const selectedAgent = props.agents.find((agent) => agent.agentId === props.selectedAgentId) ?? null;
  const selectedModel = catalogModel(props.models, props.selectedModelId, props.selectedConnectionId);
  const selectedModelEfforts = selectedModel?.supportedReasoningEfforts ?? [];
  const selectedModelHasReasoning = selectedModelEfforts.length > 0;
  const selectedModelEffort = props.selectedReasoningEffort && selectedModelEfforts.includes(props.selectedReasoningEffort)
    ? props.selectedReasoningEffort
    : props.selectedReasoningEffort
      ? null
      : selectedModel?.defaultReasoningEffort ?? null;
  const [step, setStep] = useState<"model" | "reasoning">(selectedModelHasReasoning ? "reasoning" : "model");
  const [draftModelId, setDraftModelId] = useState<string | null>(selectedModelHasReasoning ? selectedModel ? catalogModelKey(selectedModel) : null : null);
  const [draftReasoningEffort, setDraftReasoningEffort] = useState<AiReasoningEffort | null>(selectedModelEffort);
  const draftModel = props.models.find((model) => catalogModelKey(model) === draftModelId) ?? null;
  const selectedModelUnavailable = Boolean(props.selectedModelId && !selectedModel);
  const effectiveAgent = selectedAgent ?? props.agents[0] ?? null;
  const selectionLabel = conversationModelLabel(undefined, "", false);
  const labelModel = selectedModel ?? props.displayModel ?? null;
  const selectedEffortInvalid = Boolean(labelModel && props.selectedReasoningEffort && !(labelModel.supportedReasoningEfforts ?? []).includes(props.selectedReasoningEffort));
  const displayedEffort = selectedEffortInvalid
    ? null
    : props.selectedReasoningEffort ?? labelModel?.defaultReasoningEffort ?? null;
  const selectedEffortLabel = selectedEffortInvalid ? "重新选择深度" : displayedEffort ? reasoningLabel(displayedEffort, labelModel) : null;
  const selectedLabel = props.loading ? "正在加载模型…" : labelModel
    ? `${catalogModelLabel(labelModel, props.models)}${selectedEffortLabel ? ` ${selectedEffortLabel}` : ""}`
    : (selectedModelUnavailable && props.historical ? "原模型已不可用" : !props.models.length ? "暂无可用模型" : null) ?? selectionLabel;
  const modelHostAgent = effectiveAgent;
  const resetDraft = useCallback(() => {
    setStep(selectedModelHasReasoning ? "reasoning" : "model");
    setDraftModelId(selectedModelHasReasoning ? selectedModel ? catalogModelKey(selectedModel) : null : null);
    setDraftReasoningEffort(selectedModelEffort);
  }, [selectedModel?.connectionId, selectedModel?.modelId, selectedModelEffort, selectedModelHasReasoning]);
  const selectModel = (model: AiRuntimeSnapshot["models"][number]) => {
    if (!modelHostAgent) return;
    const supported = model.supportedReasoningEfforts ?? [];
    if (!supported.length) {
      props.onSelectModel(model.modelId, modelHostAgent.agentId, null, model.connectionId);
      return;
    }
    const current = model.modelId === props.selectedModelId && (model.connectionId ?? null) === props.selectedConnectionId && props.selectedReasoningEffort && supported.includes(props.selectedReasoningEffort)
      ? props.selectedReasoningEffort
      : model.defaultReasoningEffort && supported.includes(model.defaultReasoningEffort)
        ? model.defaultReasoningEffort
        : null;
    setDraftModelId(catalogModelKey(model));
    setDraftReasoningEffort(current);
    if (current) props.onSelectModel(model.modelId, modelHostAgent.agentId, current, model.connectionId);
    setStep("reasoning");
  };
  const completeReasoningSelection = (effort: AiReasoningEffort) => {
    if (!draftModel || !modelHostAgent) return;
    setDraftReasoningEffort(effort);
    props.onSelectModel(draftModel.modelId, modelHostAgent.agentId, effort, draftModel.connectionId);
  };
  const draftReasoningTitle = draftModel?.reasoningControl === "toggle" ? "思考模式" : "思考深度";
  const draftReasoningEfforts = draftModel?.supportedReasoningEfforts ?? [];
  const draftReasoningIndex = draftReasoningEffort ? draftReasoningEfforts.indexOf(draftReasoningEffort) : -1;
  const sliderValue = Math.max(0, draftReasoningIndex);

  return <div className="ai-starter-model-picker">
    <RichMenu
      align="end"
      ariaLabel="模型"
      className="ai-model-picker-menu"
      maxHeight={430}
      onOpenChange={(open) => { if (!open) resetDraft(); }}
      /* 两步共用这一个浮层：模型列表要放下名称、描述和配额标签，思考深度只有一行文字
         和一条滑轨，按同样的宽度铺开会显得空。宽度是行内样式，样式表里改不动它。 */
      width={step === "model" ? 340 : 266}
      trigger={({ open, toggle, triggerRef }) => <button aria-expanded={open} aria-haspopup="menu" aria-label={`${selectionLabel}：${selectedLabel}`} className="ch-rich-menu-trigger ai-starter-model-trigger" disabled={props.disabled} onClick={() => { if (!open) props.onOpen(); toggle(); }} ref={triggerRef} type="button">
        <span style={{ width: adaptiveControlWidth(selectedLabel, 8, 30) }} title={selectedModelUnavailable ? props.selectedModelId ?? selectedLabel : selectedLabel}>{selectedLabel}</span><DownIcon/>
      </button>}>
      {() => step === "model" ? <>
        {props.models.length ? <RichMenuSection label="模型">{props.models.map((model) => {
          const checked = model.modelId === props.selectedModelId && (model.connectionId ?? null) === props.selectedConnectionId;
          const hasReasoning = Boolean(model.supportedReasoningEfforts?.length);
          return <RichMenuItem closeOnSelect={!hasReasoning} description={`上下文 ${formatContextWindow(model.contextWindowTokens)}`} disabled={props.loading || !modelHostAgent} key={catalogModelKey(model)} label={catalogModelLabel(model, props.models)} onSelect={() => selectModel(model)} selected={checked}/>;
        })}</RichMenuSection> : <RichMenuSection label="模型"><RichMenuEmpty><strong>暂无可用模型</strong>在设置中添加连接和模型。</RichMenuEmpty></RichMenuSection>}
      </> : draftModel ? <section aria-label={`选择${draftReasoningTitle}`} className="ai-reasoning-picker" role="group">
        <header>
          <span className="ai-reasoning-picker-symbol"><ReasoningIdeaIcon/></span>
          <button aria-label={`切换模型：${draftModel.label}`} className="ai-reasoning-model-switch" onClick={() => setStep("model")} type="button">
            <strong><span>{draftReasoningEffort ? reasoningLabel(draftReasoningEffort, draftModel) : "选择深度"}</span><ChevronIcon direction="right"/></strong>
            <small>{draftModel.label}</small>
          </button>
          <button aria-label={draftModel.reasoningControl === "toggle" ? "恢复模型默认模式" : "恢复模型默认深度"} disabled={!draftModel.defaultReasoningEffort} onClick={() => {
            if (draftModel.defaultReasoningEffort) completeReasoningSelection(draftModel.defaultReasoningEffort);
          }} type="button"><ResetReasoningIcon/></button>
        </header>
        <div className="ai-reasoning-effort-track" style={{
          "--reasoning-slider-size": `${reasoningSliderSize}px`,
          "--selected-position": draftReasoningIndex < 0 ? `${reasoningSliderSize / 2}px` : reasoningSliderPosition(draftReasoningIndex, draftReasoningEfforts.length),
        } as CSSProperties}>
          <span aria-hidden="true" className="ai-reasoning-effort-fill"/>
          <span aria-hidden="true" className="ai-reasoning-effort-thumb"/>
          <span aria-hidden="true" className="ai-reasoning-effort-ticks">
            {draftReasoningEfforts.map((effort, index) => (
              <i
                data-selected={draftReasoningEffort === effort}
                key={effort}
                style={{ left: reasoningSliderPosition(index, draftReasoningEfforts.length) }}/>
            ))}
          </span>
          <input
            aria-label={draftReasoningTitle}
            aria-valuetext={draftReasoningEffort ? reasoningLabel(draftReasoningEffort, draftModel) : "未选择"}
            disabled={!draftReasoningEfforts.length}
            max={Math.max(0, draftReasoningEfforts.length - 1)}
            min={0}
            onChange={(event) => {
              const effort = draftReasoningEfforts[Number(event.currentTarget.value)];
              if (effort) completeReasoningSelection(effort);
            }}
            onKeyDown={(event) => {
              if (draftReasoningIndex >= 0 || (event.key !== "Enter" && event.key !== " ")) return;
              const effort = draftReasoningEfforts[Number(event.currentTarget.value)];
              if (effort) completeReasoningSelection(effort);
            }}
            onMouseLeave={(event) => event.currentTarget.parentElement?.removeAttribute("data-thumb-hover")}
            onMouseMove={(event) => syncReasoningThumbHover(event.currentTarget, event.clientX, event.clientY)}
            onPointerUp={(event) => {
              if (draftReasoningIndex >= 0) return;
              const effort = draftReasoningEfforts[Number(event.currentTarget.value)];
              if (effort) completeReasoningSelection(effort);
            }}
            step={1}
            title={draftReasoningEffort ? reasoningLabel(draftReasoningEffort, draftModel) : "选择深度"}
            type="range"
            value={sliderValue}/>
        </div>
      </section> : null}
    </RichMenu>
  </div>;
}

function ResetReasoningIcon() {
  return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M4 4v6h6M4.8 9A8 8 0 1 1 6 17.3"/></svg>;
}

/** 思考深度的标识。灯泡比闪电贴题：这里调的是想多深，不是跑多快。 */
function ReasoningIdeaIcon() {
  return <svg aria-hidden="true" viewBox="0 0 24 24">
    <path d="M12 3a6 6 0 0 0-3.8 10.6c.7.6 1.1 1.4 1.3 2.4h5c.2-1 .6-1.8 1.3-2.4A6 6 0 0 0 12 3Z"/>
    <path d="M9.5 19h5"/>
    <path d="M10.5 21.5h3"/>
  </svg>;
}

function ComposerActionControl(props: {
  threadDraining?: boolean;
  activeRun: AiAgentRun | null;
  busy: boolean;
  draftReady: boolean;
  hasAttachments: boolean;
  pendingApproval: boolean;
  runtimeReady: boolean;
  onQueue: () => void;
  onStop: () => void;
}) {
  if (props.activeRun && !props.draftReady && !props.hasAttachments) return <button aria-label="停止回复" className="ai-send-button ai-stop-button" disabled={props.busy} onClick={props.onStop} title="停止回复" type="button"><StopIcon/></button>;
  if (props.pendingApproval) return <button aria-label="请先处理审批" className="ai-send-button" disabled title="请先处理上方审批" type="button"><EnterIcon/></button>;
  const queued = Boolean(props.activeRun || props.threadDraining);
  return <button aria-label={queued ? "排队发送" : "发送"} className="ai-send-button"
    disabled={(!props.draftReady && !props.hasAttachments) || props.busy || !props.runtimeReady}
    onClick={queued ? props.onQueue : undefined}
    title={!props.runtimeReady ? "请先选择可用的执行方式" : queued ? "加入队列，当前回复完成后发送" : "发送"}
    type={queued ? "button" : "submit"}><EnterIcon/></button>;
}

function ConversationContextUsagePopover(props: {
  conversationId: string | null;
  runs: AiRuntimeSnapshot["runs"];
}) {
  const usage = conversationContextUsage(props.conversationId, props.runs);
  return <RichMenu
    align="end"
    ariaLabel="当前对话上下文占用"
    width={320}
    trigger={({ open, toggle, triggerRef }) => <button
      aria-expanded={open}
      aria-haspopup="menu"
      aria-label={`查看${usage.isPreviousRun ? "上次请求" : "当前对话"}上下文占用：${usage.percent === null ? "未知" : `${usage.percent.toFixed(1)}%`}`}
      className="ai-context-usage-trigger"
      onClick={toggle}
      ref={triggerRef}
      data-tip={`上下文占用 ${usage.percent === null ? "未知" : `${usage.percent.toFixed(1)}%`}`}
      type="button"><ContextUsageIcon percent={usage.percent ?? 0}/></button>}>
    <ConversationContextUsage usage={usage}/>
  </RichMenu>;
}

function conversationContextUsage(
  conversationId: string | null,
  runs: AiRuntimeSnapshot["runs"],
) {
  const conversationRuns = runs.filter((run) => run.conversationId === conversationId && run.model && !run.superseded);
  const latestRun = conversationRuns
    .reduce<AiAgentRun | null>((latest, run) => !latest || run.startedAt > latest.startedAt ? run : latest, null);
  const measuredRun = conversationRuns
    .filter((run) => run.completedAt !== null && !activeRunStatuses.includes(run.status))
    .reduce<AiAgentRun | null>((latest, run) => !latest || run.startedAt > latest.startedAt ? run : latest, null);
  const isPreviousRun = Boolean(measuredRun && latestRun && measuredRun.runId !== latestRun.runId);
  const tokens = usageCount(measuredRun?.usage.context?.tokens);
  const capacityValue = usageCount(measuredRun ? measuredRun.usage.context?.windowTokens : latestRun?.usage.context?.windowTokens);
  const capacity = capacityValue !== null && capacityValue > 0 ? capacityValue : null;
  return {
    tokens,
    capacity,
    percent: tokens !== null && capacity !== null ? Math.min(100, tokens / capacity * 100) : null,
    run: measuredRun,
    isPreviousRun,
  };
}

function ConversationContextUsage({ usage }: { usage: ReturnType<typeof conversationContextUsage> }) {
  const [detailsExpanded, setDetailsExpanded] = useState(false);
  const heading = usage.isPreviousRun ? "上次请求上下文" : "最近请求上下文";
  const tokens = usage.tokens;
  const capacity = usage.capacity;
  const composition = usage.run?.usage.context?.composition;
  const categories = [
    { key: "conversation", label: "对话内容", tokens: usageCount(composition?.conversationTokens) },
    { key: "system", label: "系统提示词", tokens: usageCount(composition?.systemPromptTokens) },
    { key: "tools", label: "工具上下文", tokens: usageCount(composition?.toolTokens) },
    { key: "skills", label: "Skill", tokens: usageCount(composition?.skillTokens) },
    { key: "mcp", label: "MCP", tokens: usageCount(composition?.mcpTokens) },
    { key: "internal", label: "Codex 内部", tokens: usageCount(composition?.internalTokens) },
  ];
  return <section aria-label="当前对话上下文占用" className="ai-model-usage">
    <header><strong>{heading}</strong><span>{composition ? "分类估算" : "未提供分类"}</span></header>
    <button
      aria-controls="ai-context-usage-details"
      aria-expanded={detailsExpanded}
      aria-label={`${detailsExpanded ? "收起" : "展开"}上下文详细统计`}
      className="ai-model-usage-toggle"
      onClick={() => setDetailsExpanded((expanded) => !expanded)}
      title={`${detailsExpanded ? "收起" : "展开"}详细统计`}
      type="button">
      <span className="ai-model-usage-summary"><span>已占用</span><strong>{tokens === null ? "未提供" : formatCount(tokens)} / {formatContextWindow(capacity)} · {usage.percent === null ? "未知" : `${usage.percent.toFixed(1)}%`}</strong><DownIcon/></span>
      <span className="ai-model-usage-bar"><span className="ai-model-usage-bar-used" data-composed={composition ? "true" : undefined} style={{ width: `${usage.percent ?? 0}%` }}>
        {categories.map((category) => tokens !== null && tokens > 0 && category.tokens !== null && category.tokens > 0
          ? <i aria-label={`${category.label} ${formatCount(category.tokens)} Token`} data-kind={category.key} key={category.key} style={{ width: `${category.tokens / tokens * 100}%` }}/>
          : null)}
      </span></span>
    </button>
    {detailsExpanded ? <div className="ai-model-usage-details" id="ai-context-usage-details">
      <dl className="ai-model-usage-categories">{categories.map((category) => <div key={category.key}><dt><i data-kind={category.key}/>{category.label}</dt><dd>{category.tokens === null ? "未提供" : formatCount(category.tokens)} · {category.tokens !== null && capacity !== null ? `${(category.tokens / capacity * 100).toFixed(1)}%` : "未知"}</dd></div>)}</dl>
    </div> : null}
  </section>;
}

function usageCount(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}

function formatContextWindow(tokens: number | null) {
  if (tokens === null) return "未知";
  const megabytes = tokens / (1024 * 1024);
  if (megabytes >= 1) return `${Number.isInteger(megabytes) ? megabytes : megabytes.toFixed(1)}M`;
  return `${Math.round(tokens / 1024)}K`;
}

function StarterPermissionPicker(props: { value: StarterPermissionMode; modelMode: boolean; disabled: boolean; hostPlatform?: string | null; onChange: (value: StarterPermissionMode) => void }) {
  const options = props.modelMode ? modelApprovalOptionsForHost(props.hostPlatform) : agentPermissionOptions;
  const selected = options.find((option) => option.value === props.value);
  const shortcutActions = Object.fromEntries(options.map((option) => [option.shortcut, () => props.onChange(option.value)]));
  return <div className="ai-starter-permission-picker">
    <RichMenu
      align="end"
      ariaLabel={props.modelMode ? "审批模式" : "工作区权限模式"}
      shortcutActions={shortcutActions}
      width={320}
      trigger={({ open, toggle, triggerRef }) => <button aria-expanded={open} aria-haspopup="menu" aria-label={`选择${props.modelMode ? "审批模式" : "工作区权限"}：${selected?.label ?? "请选择"}`} className="ch-rich-menu-trigger" disabled={props.disabled} onClick={toggle} ref={triggerRef} type="button"><ShieldIcon/><span>{selected?.shortLabel ?? "请选择"}</span><DownIcon/></button>}>
      {options.map((option) => <RichMenuItem key={option.value} label={option.label} description={option.description} onSelect={() => props.onChange(option.value)} selected={option.value === props.value} shortcut={option.shortcut}/>) }
    </RichMenu>
  </div>;
}

function ConversationApprovalCard(props: {
  request: AiPermissionRequest;
  busy: boolean;
  onDecision?: (decision: "approved" | "approved_session" | "rejected", applicationWriteArguments?: Record<string, unknown>) => void;
}) {
  const applicationWrite = props.request.applicationWrite;
  if (applicationWrite) {
    const view = applicationWrite.confirmationView;
    const declaredFields = Array.isArray(view?.fields) ? view.fields : [];
    const declaredNames = new Set(declaredFields.map((field) => field.name));
    const undeclaredFields = Object.keys(applicationWrite.arguments)
      .filter((name) => !declaredNames.has(name))
      // 未在确认视图中声明的字段一律按技术字段呈现，且不可编辑：应用没有声明它，
      // 就说明它不在这次提案的可改范围内。字段形状与已声明的那批保持一致，
      // 否则合并后的联合类型里会缺一支，后续按字段取值要逐处判空。
      .map((name) => ({ name, label: name, visibility: "technical" as const, valueLabels: undefined, editable: false }));
    const fields = [...declaredFields, ...undeclaredFields]
      .filter((field) => Object.hasOwn(applicationWrite.arguments, field.name))
      .map((field) => ({
        ...field,
        value: applicationApprovalValue(applicationWrite.arguments[field.name], field.valueLabels),
      }));
    const businessFields = fields.filter((field) => field.visibility !== "technical");
    const technicalFields = fields.filter((field) => field.visibility === "technical");
    const executionStatus = applicationWrite.executionStatus ?? applicationApprovalStatus(props.request.status);
    const state = applicationApprovalState(applicationWrite, executionStatus);
    const pending = props.request.status === "pending" && executionStatus === "awaiting_confirmation";
    const editableProposal = view?.reviewMode === "editableProposal";
    const editableFields = businessFields.filter((field) => field.editable && ["string", "number", "boolean"].includes(typeof applicationWrite.arguments[field.name]));
    const receiptId = typeof applicationWrite.receipt?.invocationId === "string" ? applicationWrite.receipt.invocationId : null;
    return <article aria-label={`${view?.title ?? applicationWrite.toolTitle}操作卡`} aria-live="polite" className="ai-approval-card ai-application-action-card" data-state={executionStatus}>
      <header><span className="ai-approval-icon"><ApprovalIcon/></span><div><span className="ai-approval-eyebrow">{applicationWrite.applicationName}</span><strong>{view?.title ?? applicationWrite.toolTitle}</strong><p>{view?.impact ?? applicationWrite.description}</p></div></header>
      <div className="ai-approval-state" data-state={executionStatus}><i aria-hidden="true"/><span>{state.label}</span><small>{applicationWrite.riskLevel === "highRisk" ? "高风险操作" : "写入操作"}</small></div>
      <ConversationFields className="ai-approval-fields" fields={businessFields.map(field => ({ key: field.name, label: field.label, importance: field.visibility, value: pending && editableProposal && field.editable && ["string", "number", "boolean"].includes(typeof applicationWrite.arguments[field.name]) ? <ApplicationProposalInput field={field} value={applicationWrite.arguments[field.name]} /> : field.value }))}/>
      {technicalFields.length || receiptId || applicationWrite.failureReason || applicationWrite.receipt ? <details className="ai-approval-technical">
        <summary>技术信息</summary>
        <dl>{technicalFields.map((field) => <div key={field.name}><dt>{field.label}</dt><dd>{field.value}</dd></div>)}<div><dt>确认号</dt><dd>{applicationWrite.confirmationId}</dd></div>{receiptId ? <div><dt>回执号</dt><dd>{receiptId}</dd></div> : null}</dl>
        {applicationWrite.failureReason ? <p>{applicationWrite.failureReason}</p> : null}
        {applicationWrite.receipt ? <code aria-label="业务回执">{JSON.stringify(applicationWrite.receipt, null, 2)}</code> : null}
      </details> : null}
      {pending ? <footer><span>{editableProposal ? "请审核最终业务提案，确认后执行一次提交" : "确认后立即执行，参数不会再变更"}</span><div><button disabled={props.busy} onClick={() => props.onDecision?.("rejected")} type="button">取消</button><button className="primary" disabled={props.busy} onClick={() => props.onDecision?.("approved", editableProposal ? Object.fromEntries(editableFields.map((field) => [field.name, proposalInputValue(field.name, applicationWrite.arguments[field.name])])) : undefined)} type="button">{props.busy ? "提交中" : view?.submitLabel ?? "确认提交"}</button></div></footer> : <footer className="ai-approval-result"><span>{state.description}</span></footer>}
    </article>;
  }
  const title = props.request.protocolMethod === "item/commandExecution/requestApproval"
      ? "允许执行这条命令？"
      : props.request.protocolMethod === "item/fileChange/requestApproval"
        ? "允许修改文件？"
        : "允许扩大本次权限？";
  const available = props.request.availableDecisions ?? [];
  const allowSession = !available.length || available.includes("acceptForSession");
  return <article aria-live="polite" className="ai-approval-card">
    <header><span className="ai-approval-icon"><ApprovalIcon/></span><div><strong>{title}</strong><p>{props.request.reason}</p></div></header>
    {props.request.command ? <code>{props.request.command}</code> : null}
    {props.request.resource ? <span className="ai-approval-resource">作用位置：{props.request.resource}</span> : null}
    <footer>
      <button disabled={props.busy} onClick={() => props.onDecision?.("rejected")} type="button">拒绝</button>
      {allowSession ? <button disabled={props.busy} onClick={() => props.onDecision?.("approved_session")} type="button">本次会话允许</button> : null}
      <button className="primary" disabled={props.busy} onClick={() => props.onDecision?.("approved")} type="button">允许一次</button>
    </footer>
  </article>;
}

/**
 * 枚举字段用应用内的选择器，不用系统原生下拉——后者的选项列表由操作系统绘制，样式表
 * 够不到，在深色主题下会突兀地弹出一块系统外观。
 *
 * 取值仍沿用本表单既有的方式：提交时按标识读 DOM。选择器本身是受控的，因此把当前值
 * 同步到一个隐藏输入上，读值那侧不必为这一个字段分出一条特例。
 */
function ApplicationProposalSelect(props: { field: { name: string; label: string; valueLabels: Record<string, string> }; value: unknown; inputId: string }) {
  const [current, setCurrent] = useState(String(props.value ?? ""));
  const options = Object.entries(props.field.valueLabels).map(([value, label]) => ({ value, label }));
  return <>
    <Select ariaLabel={props.field.label} onChange={setCurrent} options={options} value={current}/>
    <input id={props.inputId} name={props.inputId} readOnly type="hidden" value={current}/>
  </>;
}

function ApplicationProposalInput(props: { field: { name: string; label: string; valueLabels?: Record<string, string> }; value: unknown }) {
  const inputId = `application-proposal-${props.field.name}`;
  if (props.field.valueLabels) {
    return <ApplicationProposalSelect field={{ ...props.field, valueLabels: props.field.valueLabels }} inputId={inputId} value={props.value}/>;
  }
  if (typeof props.value === "boolean") return <input aria-label={props.field.label} defaultChecked={props.value} id={inputId} name={inputId} type="checkbox"/>;
  return <input aria-label={props.field.label} defaultValue={String(props.value ?? "")} id={inputId} name={inputId} type={typeof props.value === "number" ? "number" : "text"}/>;
}

function proposalInputValue(name: string, original: unknown): unknown {
  const element = document.getElementById(`application-proposal-${name}`);
  if (element instanceof HTMLInputElement) {
    if (element.type === "checkbox") return element.checked;
    if (typeof original === "number") return Number(element.value);
    return element.value;
  }
  return original;
}

function applicationApprovalValue(value: unknown, labels?: Record<string, string>): string {
  const key = typeof value === "string" || typeof value === "number" || typeof value === "boolean" ? String(value) : null;
  if (key && labels?.[key]) return labels[key];
  if (value === null || value === undefined || value === "") return "未填写";
  if (typeof value === "boolean") return value ? "是" : "否";
  if (Array.isArray(value)) return value.map((item) => applicationApprovalValue(item)).join("、");
  if (typeof value === "object") {
    const object = value as Record<string, unknown>;
    if (object.sourceType === "collaboration") return "已授权协作空间文件";
    if (object.sourceType === "linked_system") return "已授权客户系统对象";
    return "结构化业务数据";
  }
  return String(value);
}

function applicationApprovalStatus(status: AiPermissionRequest["status"]) {
  if (status === "pending") return "awaiting_confirmation" as const;
  if (status === "rejected") return "cancelled" as const;
  if (status === "expired") return "expired" as const;
  return "submitting" as const;
}

function applicationWriteReceipt(receipt: Record<string, unknown> | null | undefined) {
  const result = receipt?.result;
  if (!result || typeof result !== "object" || Array.isArray(result)) return null;
  const structured = (result as Record<string, unknown>).structuredContent;
  return structured && typeof structured === "object" && !Array.isArray(structured)
    ? structured as Record<string, unknown>
    : result as Record<string, unknown>;
}

function applicationApprovalState(
  applicationWrite: NonNullable<AiPermissionRequest["applicationWrite"]>,
  status: NonNullable<NonNullable<AiPermissionRequest["applicationWrite"]>["executionStatus"]>,
) {
  if (status === "awaiting_confirmation") return { label: "待确认", description: "等待你的决定" };
  if (status === "submitting") return { label: "正在提交", description: "业务服务正在处理" };
  if (status === "failed") return { label: "未完成", description: "业务服务已返回失败结果" };
  if (status === "outcome_unknown") return { label: "结果待核实", description: "未取得可确认的业务回执" };
  if (status === "expired") return { label: "已过期", description: "本次确认已失效，未执行操作" };
  if (status === "cancelled") return { label: "已取消", description: "已取消，未执行操作" };
  const receipt = applicationWriteReceipt(applicationWrite.receipt);
  const description = typeof receipt?.receiptSummary === "string" ? receipt.receiptSummary : "业务服务已返回成功回执";
  if (receipt?.status === "registered") return { label: "已登记", description: "档案候选已建立，待内容处理与审核" };
  if (receipt?.status === "running") return { label: "审查已发起", description };
  if (receipt?.status === "pending_confirmation") return { label: "待业务确认", description };
  if (receipt?.effectStatus === "pending") return { label: "已保存，待评估", description };
  if (receipt?.reportState === "draft") return { label: "草稿已生成", description };
  if (typeof receipt?.reviewDecision === "string") return { label: "判断已保存", description };
  if (applicationWrite.toolName === "submit_review") return { label: "已提交审核", description };
  if (applicationWrite.toolName === "decide_review") return { label: "审核决定已保存", description };
  if (applicationWrite.toolName === "create_task") return { label: "整改任务已创建", description };
  if (applicationWrite.toolName === "withdraw_document") return { label: "已撤销", description };
  if (applicationWrite.toolName === "start_rto_analysis") return { label: "分析已生成", description };
  if (applicationWrite.toolName === "set_report_review_decision") return { label: "判断已保存", description };
  if (applicationWrite.toolName === "generate_rto_report") return { label: "草稿已生成", description };
  return { label: "已提交", description };
}

function StarterProjectPicker(props: { project: AiProjectOption | null; projects: AiProjectOption[]; disabled: boolean; canCreate: boolean; onChange: (value: string | null) => void; onCreate: () => void }) {
  const selectedLabel = props.project?.displayName ?? "未分类";
  return <div className="ai-starter-project-picker">
    <RichMenu
      align="start"
      ariaLabel="对话项目"
      maxHeight={300}
      width={320}
      trigger={({ open, toggle, triggerRef }) => <button aria-expanded={open} aria-haspopup="menu" aria-label={`选择对话项目：${selectedLabel}`} className="ch-rich-menu-trigger ai-starter-project-trigger" disabled={props.disabled} onClick={toggle} ref={triggerRef} title={props.project ? `${props.project.displayName} · ${projectWorkspaceLabel(props.project)}` : "选择或新增项目"} type="button">
        <ProjectGroupIcon/><span style={{ width: adaptiveControlWidth(selectedLabel, 8, 18) }}>{selectedLabel}</span><DownIcon/>
      </button>}>
      <RichMenuItem label="未分类" description="不归入具体项目" leading={<ProjectGroupIcon/>} onSelect={() => props.onChange(null)} selected={props.project === null}/>
      {props.projects.map((project) => <RichMenuItem disabled={!projectWorkspaceAvailable(project)} key={project.projectId} label={project.displayName} description={projectWorkspaceAvailable(project) ? projectWorkspaceLabel(project) : "暂无工作空间"} leading={<ProjectGroupIcon/>} onSelect={() => props.onChange(project.projectId)} selected={project.projectId === props.project?.projectId} title={projectWorkspaceLabel(project)}/>) }
      <RichMenuSeparator/>
      <RichMenuItem action disabled={!props.canCreate} label="新增项目" description="选择本地或云端工作空间，创建项目" leading={<PlusIcon/>} onSelect={props.onCreate}/>
    </RichMenu>
  </div>;
}

export function ConversationProjectPicker(props: { projects: AiProjectOption[]; busy: boolean; canCreateProject: boolean; onCreateProject: () => void; onCancel: () => void; onPick: (project: AiProjectOption) => void }) {
  const modalRef = useModal<HTMLDivElement>({ onClose: props.onCancel, busy: props.busy });

  return <div
    aria-label="选择新对话的项目"
    ref={modalRef}
    aria-modal="true"
    className="agent-picker-backdrop"
    onMouseDown={(event) => { if (event.target === event.currentTarget && !props.busy) props.onCancel(); }}
    role="dialog">
    <section className="agent-picker conversation-project-picker">
      <header><h3>选择项目</h3><p>选定项目后，在对话框中选择智能体或模型。</p></header>
      <div className="agent-picker-list">{props.projects.map((project) => <button
        disabled={props.busy || !projectWorkspaceAvailable(project)}
        key={project.projectId}
        type="button"
        onClick={() => props.onPick(project)}><ProjectGroupIcon/><span><strong title={project.displayName}>{project.displayName}</strong><small title={projectWorkspaceLabel(project)}>{projectWorkspaceAvailable(project) ? projectWorkspaceLabel(project) : "请先添加工作空间"}</small></span></button>)}</div>
      {!props.projects.length ? <p>还没有项目，创建项目后即可开始对话。</p> : null}
      <footer><button disabled={props.busy || !props.canCreateProject} onClick={props.onCreateProject} type="button"><PlusIcon/>新建项目</button><button disabled={props.busy} onClick={props.onCancel} type="button">取消</button></footer>
    </section>
  </div>;
}

type ConversationInfoSection = "progress" | "outputs" | "context";

function ConversationInfoPanel({ context, runs, files, workspace, onRemoveMaterial, onAddContext, onOpenArtifact, onContextArtifact }: { context: AiContext; runs: AiAgentRun[]; files: WorkspaceFileItem[]; workspace: WorkspaceSummary | null; onRemoveMaterial: (id: string) => void; onAddContext: () => void; onOpenArtifact: (runId: string, artifact: ConversationArtifact) => void; onContextArtifact: ArtifactContextHandler }) {
  const [expanded, setExpanded] = useState<Record<ConversationInfoSection, boolean>>({ progress: true, outputs: true, context: true });
  const artifacts = conversationFileArtifacts(runs);
  const latestRun = runs[runs.length - 1] ?? null;
  const hasTodoList = Boolean(latestRun?.events.some((event) => event.eventType === "turn.plan.updated"));
  const progress = runProgress(latestRun);
  const referencedMaterials = conversationReferencedMaterials(context);
  const normalizePath = (path: string) => path.replaceAll("\\", "/");
  const readPaths = new Set(runs.flatMap((run) => run.events.flatMap((event) => event.readPaths ?? [])).map(normalizePath));
  const hasReadRecord = (path: string) => readPaths.has(normalizePath(path)) || readPaths.has(normalizePath(`${context.workspaceRoot}/${path}`));
  const toggleSection = (section: ConversationInfoSection) => setExpanded((current) => ({ ...current, [section]: !current[section] }));

  return <aside aria-label={hasTodoList ? "对话进度、产物与上下文" : "对话产物与上下文"} className="ai-conversation-info-panel" id="ai-conversation-info-panel">
    {hasTodoList ? <section className="ai-conversation-info-section">
      <ConversationInfoSectionHeader controls="ai-conversation-progress" expanded={expanded.progress} label="进度" onToggle={() => toggleSection("progress")}/>
      {expanded.progress ? <div className="ai-conversation-progress" id="ai-conversation-progress">
        <div aria-label={`当前运行进度：${progress.summary}`} className="ai-progress-track" role="img">
          {progress.steps.map((step, index) => <span key={step.label} title={step.label}>
            {index > 0 ? <i aria-hidden="true" data-state={step.state}/> : null}
            <b data-state={step.state}>{step.state === "complete" ? <CheckIcon/> : step.state === "error" ? <CloseIcon/> : null}</b>
          </span>)}
        </div>
        <p>{progress.description}</p>
      </div> : null}
    </section> : null}
    <section className="ai-conversation-info-section">
      <ConversationInfoSectionHeader controls="ai-conversation-outputs" expanded={expanded.outputs} label="产物" onToggle={() => toggleSection("outputs")}/>
      {expanded.outputs ? <div className="ai-conversation-section-body" id="ai-conversation-outputs">
        {artifacts.length
          ? <div className="ai-conversation-info-list">{artifacts.map(({ runId, artifact }) => <button aria-label={`打开产物 ${fileNameFromPath(artifact.path!)}`} className="ai-conversation-info-item" data-image={artifact.artifactType === "image" || undefined} key={artifact.artifactId} onClick={() => onOpenArtifact(runId, artifact)} onContextMenu={(event) => onContextArtifact(event, runId, artifact)} title={artifact.path!} type="button">
            <ArtifactVisual artifact={artifact} runId={runId}/>
            <div><strong>{fileNameFromPath(artifact.path!)}</strong><small>{artifact.path}</small></div>
          </button>)}</div>
          : <div className="ai-conversation-output-empty"><OutputIcon/><div><strong>暂无产物</strong><p>大模型生成的文件会显示在这里。</p></div></div>}
      </div> : null}
    </section>
    <section className="ai-conversation-info-section">
      <header className="ai-conversation-info-context-header">
        <ConversationInfoSectionHeader controls="ai-conversation-context" expanded={expanded.context} label="上下文" onToggle={() => toggleSection("context")}/>
        <button aria-label="添加引用材料" onClick={onAddContext} title="添加引用材料" type="button"><ContextAddIcon/></button>
      </header>
      {expanded.context ? <div className="ai-conversation-section-body" id="ai-conversation-context">
        {referencedMaterials.length
          ? <div className="ai-conversation-material-list">{referencedMaterials.map((material) => <article className="ai-conversation-material" key={material.key} title={material.path}>
            <span className="ai-conversation-material-icon">{material.kind === "image" ? <ImageMaterialIcon/> : <FileIcon/>}</span>
            <span><strong>{material.name}</strong><small>{material.path}</small><small>{material.workspaceId && material.workspaceId === workspace?.workspaceId ? workspace.workspaceKind === "cloud_team" ? "云端" : "本地" : context.teamSpaceId ? "云端" : "对话材料"} · {files.some((file) => file.localFileId === material.key && file.stateRevision !== material.revision) ? "文件已更新" : hasReadRecord(material.path) ? "有读取记录" : "已引用"} · 版本 {material.revision}</small></span>
            <button aria-label={`移除材料 ${material.name}`} onClick={() => onRemoveMaterial(material.key)} title="移除" type="button"><CloseIcon/></button>
          </article>)}</div>
          : <p className="ai-conversation-info-empty">本对话尚未引用文件或图片</p>}
      </div> : null}
    </section>
  </aside>;
}

function ConversationInfoSectionHeader({ controls, expanded, label, onToggle }: { controls: string; expanded: boolean; label: string; onToggle: () => void }) {
  return <button aria-controls={controls} aria-expanded={expanded} className="ai-conversation-info-section-toggle" onClick={onToggle} type="button"><span aria-level={2} className="ai-conversation-info-section-title" role="heading">{label}</span><DownIcon/></button>;
}

function runProgress(run: AiAgentRun | null) {
  if (!run) return {
    summary: "尚未运行",
    description: "发送任务后，这里会显示执行进度。",
    steps: ["current", "pending", "pending"].map((state, index) => ({ label: ["等待任务", "执行中", "已完成"][index], state })),
  } as const;
  const failed = ["failed", "interrupted", "terminated"].includes(run.status);
  const completed = run.status === "completed";
  return {
    summary: runStatusLabel(run.status),
    description: runStatusLabel(run.status),
    steps: [
      { label: "任务已接收", state: "complete" },
      { label: "执行中", state: run.status === "queued" ? "current" : "complete" },
      { label: failed ? runStatusLabel(run.status) : "已完成", state: failed ? "error" : completed ? "complete" : "pending" },
    ],
  } as const;
}

function conversationReferencedMaterials(context: AiContext) {
  return materialReferences(context).map((material) => ({
    key: material.localFileId || material.relativePath,
    kind: isImagePath(material.relativePath) ? "image" : "file",
    name: fileNameFromPath(material.relativePath),
    path: material.relativePath,
    revision: material.fileRevision,
    workspaceId: material.workspaceId ?? context.workspaceId,
  }));
}

function isImagePath(path: string) {
  return /\.(?:avif|bmp|gif|heic|heif|jpe?g|png|svg|tiff?|webp)$/iu.test(path);
}

function materialReferences(context: AiContext) {
  const references = [...(context.referencedMaterials ?? [])];
  if (context.localFileId && context.relativePath && context.fileRevision !== null && !references.some((item) => item.localFileId === context.localFileId)) {
    references.push({ localFileId: context.localFileId, relativePath: context.relativePath, fileRevision: context.fileRevision, workspaceId: context.workspaceId });
  }
  const unique = new Map<string, (typeof references)[number]>();
  for (const reference of references) {
    const key = `${reference.workspaceId ?? "isolated"}:${reference.localFileId}`;
    if (!unique.has(key)) unique.set(key, reference);
  }
  return [...unique.values()];
}

function QueuedFollowUps({ busy, supplementUnavailableReason, items, onDelete, onEdit, onRetry, onSteer }: {
  busy: boolean;
  supplementUnavailableReason: string | null;
  items: QueuedFollowUp[];
  onDelete: (queueId: string) => void;
  onEdit: (queueId: string) => void;
  onRetry: (queueId: string) => void;
  onSteer: (queueId: string) => void;
}) {
  /* 更多菜单挂在文档根部，展开时看不出它属于哪一行。记下开着的是哪一条，把那一行标出来。 */
  const [menuOpen, setMenuOpen] = useState<string | null>(null);
  return <section aria-label="待发送的后续指令" aria-live="polite" className="ai-queued-followups">
    {items.map((item, index) => {
      /*
       * 序号与默认状态不再占版面。每条都写着"排队 N""等待当前回复完成"，条数越多重复得越厉害，
       * 而先后顺序本来就由行的排列表达。偏离默认的状态才真的带信息，那几种照常写出来。
       * 无障碍标签里仍带序号——那里没有"看一眼就知道是第几条"这回事。
       */
      const position = index + 1;
      const pending = item.status === "queued" || item.status === "failed";
      const status = item.status === "sending" || item.status === "steering" ? "正在发送"
        : item.status === "deleting" ? "正在删除"
        : item.status === "failed" ? (item.error ?? "发送失败")
        : null;
      const canAdjust = item.executionTarget === "model" && !item.attachments.length && !item.referencedMaterials?.length && !item.applicationContext;
      return <article data-menu-open={menuOpen === item.queueId || undefined} data-status={item.status} key={item.queueId}>
        <QueueIcon/>
        <div>
          {item.referencedMaterials?.length ? <MaterialAttachments materials={item.referencedMaterials}/> : null}
          {item.attachments.length ? <DraftAttachments items={item.attachments} onRemove={() => undefined} removable={false}/>: null}
          {/* 收成一行，超出省略；完整内容交给原生浮层，不为此把行撑高 */}
          {item.content ? <p title={item.content}>{item.content}</p> : null}
        </div>
        {status ? <span>{status}</span> : null}
        {/*
          * 露在行上的是"这一条马上要怎么处理"：插进当前回复、失败了重来一次、不要了。
          * 编辑要把内容搬回输入框，是另起一次编辑，收进更多菜单，不与前几个并排抢位置。
          */}
        <footer>
          {canAdjust ? <button aria-label={`补充回复：排队 ${position}`} className="ai-queued-adjust" disabled={busy || !pending || supplementUnavailableReason !== null} onClick={() => onSteer(item.queueId)} title={status && !pending ? status : supplementUnavailableReason ?? "立即发送补充"} type="button"><QueueAdjustIcon/>补充回复</button> : null}
          {item.status === "failed" ? <button aria-label={`重试排队消息 ${position}`} disabled={busy} onClick={() => onRetry(item.queueId)} title="重试发送" type="button"><RefreshIcon/></button> : null}
          {pending ? <button aria-label={`删除排队消息 ${position}`} disabled={busy} onClick={() => onDelete(item.queueId)} title="删除排队消息" type="button"><TrashIcon/></button> : null}
          {pending ? <RichMenu
            align="end"
            ariaLabel={`排队消息 ${position} 的更多操作`}
            onOpenChange={(open) => setMenuOpen(open ? item.queueId : null)}
            width={220}
            trigger={({ open, toggle, triggerRef }) => <button aria-expanded={open} aria-haspopup="menu" aria-label={`排队消息 ${position} 的更多操作`} disabled={busy} onClick={toggle} ref={triggerRef} title="更多操作" type="button"><MoreIcon/></button>}>
            <RichMenuItem description="移回输入框修改，改完重新排队" label="编辑消息" leading={<EditIcon/>} onSelect={() => onEdit(item.queueId)}/>
          </RichMenu> : null}
        </footer>
      </article>;
    })}
  </section>;
}

const openExecutionRuns = new Set<string>();

type EventDetailState = { loading: boolean; value?: AiRunEventDetail; error?: string };

function executionEventStatus(event: AiRunEvent) {
  if (event.eventType.endsWith("started")) return "started";
  if (event.eventType.includes("failed")) return "failed";
  return event.status ?? "completed";
}

function executionEventSummary(event: AiRunEvent) {
  return event.summary
    .replaceAll("模型 Harness", "模型")
    .replace("模型开始处理本轮请求", "正在理解你的请求")
    .replace("模型开始分析", "正在分析上下文与约束")
    .replace("模型完成分析", "已分析上下文与约束")
    .replace("模型开始生成回复", "正在生成回复")
    .replace("模型完成生成回复", "已生成回复")
    .replace("开始执行工作区命令", "执行工作区操作")
    .replace("完成执行工作区命令", "已执行工作区操作")
    .replace("开始处理候选文件变更", "处理文件变更")
    .replace("完成处理候选文件变更", "已编辑文件")
    .replace("开始调用已授权工具", "调用已授权工具")
    .replace("完成调用已授权工具", "已调用工具");
}

type ExecutionEntry = { event: AiRunEvent; mode: "narrative" | "activity"; summary: string };

type ExecutionEventKind = NonNullable<AiRunEvent["kind"]>;

function executionEventKind(event: AiRunEvent): ExecutionEventKind {
  if (event.kind && event.kind !== "status") return event.kind;
  if (/工作区命令|工作区操作/.test(event.summary)) return "command";
  if (/候选文件变更|文件变更|编辑文件/.test(event.summary)) return "file_change";
  if (/授权工具|调用工具/.test(event.summary)) return "mcp";
  if (/分析/.test(event.summary)) return "reasoning";
  if (/生成回复/.test(event.summary)) return "message";
  if (/计划/.test(event.summary)) return "plan";
  return "status";
}

function executionEntry(event: AiRunEvent, active: boolean, phase: "reply" | "classification"): ExecutionEntry | null {
  if (phase !== "reply") return null;
  if (isBackgroundToolDiscoveryEvent(event)) return null;
  const status = executionEventStatus(event);
  const kind = executionEventKind(event);
  const summary = executionEventSummary(event).trim();
  if (!summary) return null;
  if (["command", "mcp", "file_change", "plan", "context"].includes(kind)) return { event, mode: "activity", summary };
  if (kind === "reasoning") {
    if (["正在分析上下文与约束", "已分析上下文与约束", "分析完成"].includes(summary)) {
      return active && status === "started" ? { event, mode: "activity", summary } : null;
    }
    return { event, mode: "narrative", summary };
  }
  if (kind === "message") {
    if (["正在生成回复", "已生成回复", "回复生成完成"].includes(summary)) return null;
    return { event, mode: "narrative", summary };
  }
  if (status === "failed") return { event, mode: "activity", summary };
  return null;
}

function executionEntries(events: AiRunEvent[], active: boolean) {
  const entries: ExecutionEntry[] = [];
  const pendingLegacyActivities = new Map<ExecutionEventKind, number>();
  let legacyPhase: "reply" | "classification" = "reply";
  for (const event of events) {
    if (!event.phase && event.eventType === "harness.classification_started") legacyPhase = "classification";
    const entry = executionEntry(event, active, event.phase ?? legacyPhase);
    if (!entry) continue;
    const kind = executionEventKind(event);
    const status = executionEventStatus(event);
    const legacyActivity = !event.itemId && ["command", "mcp", "file_change", "plan"].includes(kind);
    if (!legacyActivity) {
      entries.push(entry);
      continue;
    }
    if (status === "started") {
      pendingLegacyActivities.set(kind, entries.length);
      entries.push(entry);
      continue;
    }
    const pendingIndex = pendingLegacyActivities.get(kind);
    if (pendingIndex === undefined) {
      entries.push(entry);
    } else {
      entries[pendingIndex] = entry;
      pendingLegacyActivities.delete(kind);
    }
  }
  return entries;
}

function ExecutionActivityIcon({ event }: { event: AiRunEvent }) {
  if (executionEventStatus(event) === "started") return <span aria-hidden="true" className="ai-running-mark"/>;
  const kind = executionEventKind(event);
  if (kind === "file_change") return <FileChangeActivityIcon/>;
  if (kind === "mcp") return <ToolActivityIcon/>;
  if (kind === "plan") return <PlanActivityIcon/>;
  if (kind === "context") return <ContextActivityIcon/>;
  if (kind === "reasoning") return <ReasoningActivityIcon/>;
  return <CommandActivityIcon/>;
}

function RunEventDetail({ detail }: { detail: AiRunEventDetail }) {
  const copyText = [
    `工具：${detail.toolName}`,
    `操作：${detail.operation}`,
    detail.target ? `对象：${detail.target}` : null,
    detail.parameters ? `参数：\n${detail.parameters}` : null,
    detail.output ? `输出：\n${detail.output}` : null,
    detail.exitCode !== null ? `退出码：${detail.exitCode}` : null,
    detail.error ? `错误：\n${detail.error}` : null,
    detail.truncated ? "详情已截断" : null,
  ].filter(Boolean).join("\n");
  return <div className="ai-execution-detail">
    <header><strong>步骤详情</strong><button aria-label="复制已脱敏的步骤详情" onClick={() => void writeClipboard(copyText)} title="复制" type="button"><CopyIcon/></button></header>
    <dl>
      <div><dt>工具</dt><dd>{detail.toolName}</dd></div>
      <div><dt>操作</dt><dd>{detail.operation}</dd></div>
      {detail.target ? <div><dt>对象</dt><dd>{detail.target}</dd></div> : null}
      {detail.exitCode !== null ? <div><dt>退出码</dt><dd>{detail.exitCode}</dd></div> : null}
    </dl>
    {detail.parameters ? <section><strong>参数</strong><pre>{detail.parameters}</pre></section> : null}
    {detail.output ? <section><strong>输出摘要</strong><pre>{detail.output}</pre></section> : null}
    {detail.error ? <section className="error"><strong>错误</strong><pre>{detail.error}</pre></section> : null}
    {detail.truncated ? <small>内容较长，已按安全上限截断。</small> : null}
  </div>;
}

function PendingReplyIntro() {
  return <div aria-label="回复准备中" className="ai-pending-reply-intro" role="status"><span aria-hidden="true" className="ai-pending-reply-dots"><i/><i/><i/></span></div>;
}

function RunExecutionProcess({ active = false, progress, readOnly = false, run }: { active?: boolean; progress?: string; readOnly?: boolean; run: Pick<AiAgentRun, "runId" | "events"> | null }) {
  const [open, setOpen] = useState(() => Boolean(run && openExecutionRuns.has(run.runId)));
  const [detailOpen, setDetailOpen] = useState<Record<string, boolean>>({});
  const [details, setDetails] = useState<Record<string, EventDetailState>>({});
  const entries = executionEntries(run?.events ?? [], active);
  const toggleOpen = () => {
    const next = !open;
    setOpen(next);
    if (run) {
      if (next) openExecutionRuns.add(run.runId); else openExecutionRuns.delete(run.runId);
    }
  };
  const toggleDetail = async (event: AiRunEvent) => {
    if (!run || !event.itemId || !event.detailAvailable) return;
    const key = `${event.phase ?? "reply"}:${event.itemId}`;
    const next = !detailOpen[key];
    setDetailOpen((current) => ({ ...current, [key]: next }));
    if (!next || details[key]) return;
    setDetails((current) => ({ ...current, [key]: { loading: true } }));
    try {
      const value = await loadAiRunEventDetail(run.runId, event.itemId);
      setDetails((current) => ({ ...current, [key]: { loading: false, value } }));
    } catch (error) {
      setDetails((current) => ({ ...current, [key]: { loading: false, error: aiErrorMessage(error) } }));
    }
  };
  const renderEntry = ({ event, mode, summary }: ExecutionEntry) => {
    const status = executionEventStatus(event);
    const key = `${event.phase ?? "reply"}:${event.itemId ?? event.sequence}`;
    const expandable = Boolean(!readOnly && event.detailAvailable && event.itemId);
    const detail = details[key];
    if (mode === "narrative") return <div className="ai-execution-narrative" key={key}><MarkdownContent diagramIdPrefix={`execution-${run?.runId ?? "pending"}`}>{summary}</MarkdownContent></div>;
    return <div className="ai-execution-activity" data-status={status} key={key}>
      <span className="ai-execution-activity-icon"><ExecutionActivityIcon event={event}/></span>
      {expandable ? <button aria-expanded={Boolean(detailOpen[key])} className="ai-execution-step-button" onClick={() => void toggleDetail(event)} type="button"><span>{summary}</span><ChevronIcon direction={detailOpen[key] ? "down" : "right"}/></button> : <div className="ai-execution-step-label"><span>{summary}</span></div>}
      {expandable && detailOpen[key] ? detail?.loading ? <p className="ai-execution-detail-state">正在读取安全详情…</p> : detail?.error ? <p className="ai-execution-detail-state error">{detail.error}</p> : detail?.value ? <RunEventDetail detail={detail.value}/> : null : null}
    </div>;
  };
  const toggleLabel = active ? progress ?? "正在处理…" : "执行过程";
  if (readOnly && !entries.length) return null;
  return <section className="ai-execution-process" data-active={active || undefined}>
    <button aria-expanded={open} aria-label={`${open ? "收起" : "展开"}执行过程：${toggleLabel}`} aria-live={active ? "polite" : undefined} className="ai-execution-toggle" onClick={toggleOpen} type="button">
      <strong>{toggleLabel}</strong>
      <ChevronIcon direction={open ? "down" : "right"}/>
    </button>
    {open ? <div className="ai-execution-body">
      {entries.length ? entries.map(renderEntry) : <p className="ai-execution-empty">{active ? "正在处理…" : "本次运行没有可展示的执行记录"}</p>}
    </div> : null}
  </section>;
}

function RunArtifacts({ onOpen, onContext, run }: { onOpen: (artifact: ConversationArtifact) => void; onContext: ArtifactContextHandler; run: AiAgentRun }) {
  const artifacts = conversationFileArtifacts([{ runId: run.runId, artifacts: run.artifacts }]).reverse();
  if (!artifacts.length) return null;
  const images = artifacts.filter(({ artifact }) => artifact.artifactType === "image").map(({ artifact }) => artifact);
  const documents = artifacts.filter(({ artifact }) => artifact.artifactType !== "image").map(({ artifact }) => artifact);
  return <>
    {images.length ? <ImageArtifactGallery artifacts={images} onContext={onContext} runId={run.runId}/> : null}
    {documents.length ? <section aria-label="本次运行文件" className="ai-run-artifacts">
      <header><FileChangeActivityIcon/><strong>已生成 {documents.length} 个文件</strong></header>
      <div>{documents.map((artifact) => <button aria-label={`打开文件 ${fileNameFromPath(artifact.path!)}`} key={artifact.artifactId} onClick={() => onOpen(artifact)} onContextMenu={(event) => onContext(event, run.runId, artifact)} title={artifact.path!} type="button"><ArtifactVisual artifact={artifact} runId={run.runId}/><span><strong>{fileNameFromPath(artifact.path!)}</strong><small>{artifact.path}</small></span><ChevronIcon direction="right"/></button>)}</div>
    </section> : null}
  </>;
}

export function imageGenerationProgress(run: AiAgentRun): "generating" | "saving" | null {
  if (run.artifacts?.some((artifact) => artifact.artifactType === "image")) return null;
  const event = [...(run.events ?? [])].reverse().find((item) => item.phase !== "classification"
    && item.kind === "mcp"
    && /\bgenerate_image\b/i.test(item.summary));
  if (!event || executionEventStatus(event) === "failed") return null;
  return executionEventStatus(event) === "started" ? "generating" : "saving";
}

function ImageGenerationPending({ phase }: { phase: "generating" | "saving" }) {
  const label = phase === "generating" ? "正在生成图片" : "正在保存图片";
  return <section aria-label="图片生成中" aria-live="polite" className="ai-run-image-pending" role="status">
    <header><ImageMaterialIcon/><strong>{label}</strong></header>
    <div><ImageMaterialIcon/><span>{label}</span></div>
  </section>;
}

function ImageArtifactGallery({ artifacts, runId, onContext }: { artifacts: ConversationArtifact[]; runId: string; onContext: ArtifactContextHandler }) {
  const [activeIndex, setActiveIndex] = useState(0);
  const [lightboxOpen, setLightboxOpen] = useState(false);
  const previews = useArtifactImagePreviews(artifacts, runId);
  const active = artifacts[Math.min(activeIndex, artifacts.length - 1)];
  const preview = previews[active.artifactId];
  const previewUrl = preview || null;
  const previewUnavailable = preview === "";
  return <section aria-label="本次运行图片" className="ai-run-image-gallery" onContextMenu={(event) => onContext(event, runId, active)}>
    <header><ImageMaterialIcon/><strong>已生成 {artifacts.length} 张图片</strong><span>{fileNameFromPath(active.path!)}</span></header>
    <div className="ai-run-image-gallery-body">
      <button aria-label={`查看图片 ${fileNameFromPath(active.path!)}`} className="ai-run-image-main" disabled={!previewUrl} onClick={() => setLightboxOpen(true)} title="点击全屏查看" type="button">
        {previewUrl ? <img alt={fileNameFromPath(active.path!)} src={previewUrl}/> : previewUnavailable ? <span className="ai-run-image-unavailable"><ImageMaterialIcon/>无法显示图片</span> : <span aria-label="正在加载图片" className="ai-run-image-loading"><ImageMaterialIcon/></span>}
        {previewUrl ? <span className="ai-run-image-expand"><ExpandIcon/>全屏</span> : null}
      </button>
      {artifacts.length > 1 ? <div aria-label="图片列表" className="ai-run-image-thumbnails" role="list">
        {artifacts.map((artifact, index) => {
          const source = previews[artifact.artifactId];
          return <button aria-current={index === activeIndex ? "true" : undefined} aria-label={`选择图片 ${index + 1}：${fileNameFromPath(artifact.path!)}`} key={artifact.artifactId} onContextMenu={(event) => onContext(event, runId, artifact)} onClick={() => setActiveIndex(index)} role="listitem" title={fileNameFromPath(artifact.path!)} type="button">
            {source ? <img alt="" src={source}/> : <ImageMaterialIcon/>}
          </button>;
        })}
      </div> : null}
    </div>
    {lightboxOpen && previewUrl ? <ImageArtifactLightbox activeIndex={activeIndex} artifacts={artifacts} onChange={setActiveIndex} onClose={() => setLightboxOpen(false)} previews={previews}/> : null}
  </section>;
}

function useArtifactImagePreviews(artifacts: ConversationArtifact[], runId: string) {
  const [previews, setPreviews] = useState<Record<string, string>>({});
  const artifactKey = artifacts.map((artifact) => `${artifact.artifactId}:${artifact.path ?? ""}`).join("|");
  useEffect(() => {
    let disposed = false;
    const objectUrls: string[] = [];
    setPreviews({});
    for (const artifact of artifacts) {
      void invoke<ArrayBuffer | Uint8Array | number[]>("ai_artifact_read_image", {
        runId,
        artifactId: artifact.artifactId,
      }).then((payload) => {
        if (disposed) return;
        const source = payload instanceof ArrayBuffer
          ? new Uint8Array(payload)
          : payload instanceof Uint8Array
            ? payload
            : Uint8Array.from(payload);
        const bytes = new Uint8Array(source.byteLength);
        bytes.set(source);
        const objectUrl = URL.createObjectURL(new Blob([bytes], { type: imageArtifactMimeType(artifact.path) }));
        objectUrls.push(objectUrl);
        setPreviews((current) => ({ ...current, [artifact.artifactId]: objectUrl }));
      }).catch(() => {
        if (!disposed) setPreviews((current) => ({ ...current, [artifact.artifactId]: "" }));
      });
    }
    return () => {
      disposed = true;
      objectUrls.forEach((url) => URL.revokeObjectURL(url));
    };
  }, [artifactKey, runId]);
  return previews;
}

function WindowsSandboxGate(props: { loading: boolean; settingUp: boolean; status: AiWindowsSandboxStatus | null; onOpenSettings: () => void }) {
  const label = props.settingUp ? "正在自动修复 Windows 安全沙箱，请在系统弹窗中批准"
    : props.loading ? "正在检查 Windows 安全沙箱"
    : props.status?.status === "notConfigured" ? "Windows 安全沙箱未初始化"
      : props.status?.status === "updateRequired" ? "Windows 安全沙箱需要更新"
        : "Windows 安全沙箱不可用";
  return <button aria-label={`${label}，打开设置`} className="ai-windows-sandbox-gate" onClick={props.onOpenSettings} type="button">
    <span><SandboxGateIcon/>{label}</span><strong>打开设置</strong>
  </button>;
}

function classificationFailureDetail(reason: string) {
  const code = reason.match(/\bAI-CLASSIFY-00[12]\b/)?.[0]
    ?? (/格式|解析|schema|structured|invalid\s+json/i.test(reason) ? "AI-CLASSIFY-002" : "AI-CLASSIFY-001");
  const requestId = reason.match(/(?:请求编号|request(?:[ _-]?id)?)\s*[:：=]\s*([a-z0-9-]{8,})/i)?.[1];
  const summary = code === "AI-CLASSIFY-002"
    ? "工作归类结果格式不符合要求，本次回复已保留，任务状态未更新。"
    : "工作归类暂时失败，本次回复已保留，任务状态未更新。";
  return { summary, code, requestId, text: `${summary}\n错误代码：${code}${requestId ? `\n请求编号：${requestId}` : ""}` };
}

/** 正常运行的执行记录跟随对应回复；这里只保留无法附着到回复的异常事实。 */
function ConversationRun({ hasAssistantMessage, modelLabel, run }: { hasAssistantMessage: boolean; modelLabel: string | null; run: AiAgentRun | null }) {
  const [open, setOpen] = useState(false);
  if (!run) return null;
  const classificationFailed = run.classificationStatus === "failed";
  const healthy = run.status === "completed" && !run.errorReason && !classificationFailed;
  if (healthy) return null;
  const statusLabel = classificationFailed && run.status === "completed" ? "回复已生成" : runStatusLabel(run.status);
  const summary = run.errorReason ? runFailureSummary(run.errorReason) : classificationFailed ? "工作归类失败，任务状态未更新" : run.status === "completed" ? "回复已保存到对话" : runStatusDescription(run.status);
  const tone = run.status === "completed" ? classificationFailed ? "warning" : "success" : "danger";
  return <section aria-label={run.status === "completed" ? "最近一次运行提示" : "最近一次运行异常"} className="ai-conversation-runs">
    {!hasAssistantMessage ? <RunExecutionProcess run={run}/> : null}
    <button aria-haspopup="dialog" aria-label={`查看运行详情：${statusLabel}，${summary}`} className="ai-run-record-trigger" data-tone={tone} onClick={() => setOpen(true)} type="button">
      <span className="ai-run-record-state"><RunRecordIcon status={run.status}/><strong>{statusLabel}</strong></span>
      <span className="ai-run-record-summary">{summary}</span>
      <span aria-hidden="true" className="ai-run-record-action">详情<ChevronIcon direction="right"/></span>
    </button>
    {open ? <RunDetailDialog key={run.runId} modelLabel={modelLabel} onClose={() => setOpen(false)} run={run} statusLabel={statusLabel} summary={summary} tone={tone}/> : null}
  </section>;
}

function RunDetailDialog({ modelLabel, onClose, run, statusLabel, summary, tone }: {
  modelLabel: string | null; onClose: () => void; run: AiAgentRun; statusLabel: string; summary: string; tone: string;
}) {
  const modalRef = useModal<HTMLDivElement>({ onClose });
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const classification = !run.errorReason && run.classificationError ? classificationFailureDetail(run.classificationError) : null;
  const detailError = run.errorReason ?? classification?.text;
  const copyDetails = async () => {
    if (!detailError) return;
    try {
      await writeClipboard(detailError);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  };
  return createPortal(<div aria-label="运行详情" aria-modal="true" className="confirm-backdrop ai-run-detail-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) { event.preventDefault(); onClose(); } }} ref={modalRef} role="dialog">
    <section className="confirm-dialog ai-run-detail-dialog" data-tone={tone}>
      <header className="ai-run-detail-header"><h3>运行详情</h3><button aria-label="关闭" autoFocus className="ai-run-detail-close" onClick={onClose} type="button"><CloseIcon/></button></header>
      <div className="ai-run-detail-body">
        <div className="ai-run-detail-outcome"><span className="ai-run-record-state"><RunRecordIcon status={run.status}/><strong>{statusLabel}</strong></span><p>{summary}</p></div>
        <dl className="ai-run-detail-meta">
          <div><dt>执行方式</dt><dd>{run.executionTarget === "model" ? "直接模型" : "本地 Agent"}</dd></div>
          <div><dt>模型</dt><dd>{modelLabel ?? "当前模型"}</dd></div>
          <div><dt>权限</dt><dd>{run.sandboxMode === "danger-full-access" ? "完全访问" : run.sandboxMode === "workspace-write" ? "可修改工作区" : "只读"}</dd></div>
        </dl>
        {detailError ? <section className="ai-run-diagnostics">
          <h4>{classification ? "归类详情" : "技术详情"}</h4>
          {classification ? <><p>{classification.summary}</p><dl className="ai-run-diagnostic-ids">
            <div><dt>错误代码</dt><dd><code>{classification.code}</code></dd></div>
            {classification.requestId ? <div><dt>请求编号</dt><dd><code>{classification.requestId}</code></dd></div> : null}
          </dl></> : <pre>{detailError}</pre>}
        </section> : <p className="ai-run-detail-empty">本次运行结果已保存在对话中。</p>}
      </div>
      <footer className="ai-run-detail-footer">
        <span aria-live="polite" className="ai-run-copy-feedback" data-error={copyState === "failed" || undefined}>{copyState === "copied" ? "已复制" : copyState === "failed" ? "复制失败，请重试" : ""}</span>
        {detailError ? <button onClick={() => void copyDetails()} type="button"><CopyIcon/>复制详情</button> : null}
      </footer>
    </section>
  </div>, document.body);
}

function incompleteRunSummary(run: AiAgentRun) {
  const summary = run.errorReason ? runFailureSummary(run.errorReason) : runStatusDescription(run.status);
  return summary === "运行未能完成" && ["interrupted", "terminated"].includes(run.status)
    ? runStatusDescription(run.status)
    : summary;
}

export function runFailureSummary(reason: string) {
  if (reason.includes("工作台登录已失效")) return "请重新登录";
  if (reason.includes("当前企业会话已变更")) return "企业会话已变更";
  if (reason.includes("AI-SVC-001")) return "模型服务暂时不可用";
  if (reason.includes("AI-SVC-002")) return "模型服务繁忙";
  if (reason.includes("AI-QUOTA-001")) return "模型额度不足";
  if (reason.includes("AI-QUOTA-002")) return "企业额度账尚未就绪";
  if (reason.includes("AI-LIMIT-001")) return "企业模型并发已达上限";
  if (reason.includes("AI-LIMIT-002")) return "请求过于频繁";
  if (reason.includes("AI-AUTH-001")) return "本次运行凭据已失效";
  if (reason.includes("AI-AUTH-002")) return "无权使用此模型";
  if (reason.includes("AI-AUTH-003")) return "无权执行此操作";
  if (reason.includes("AI-REQ-002")) return "所选模型不支持图片";
  if (reason.includes("AI-REQ-003")) return "输入内容超过模型限制";
  if (reason.includes("AI-REQ-001")) return "请求内容无法处理";
  if (reason.includes("AI-EXEC-001")) return "本次回复未完整完成";
  if (reason.includes("AI-EXEC-002")) return "上一次请求仍在处理中";
  if (reason.includes("AI-EXEC-003")) return "模型响应超时";
  if (reason.includes("AI-MODEL-001")) return "所选模型当前不可用";
  if (reason.includes("AI-SYS-001")) return "模型运行未能完成";
  const threadFailure = ["上一轮仍在收尾", "上一轮执行状态待核验", "消息接收状态待核实", "对话接续未完成", "对话恢复失败", "历史上下文尚未完整恢复", "单条历史内容过大"]
    .find((title) => reason.startsWith(`${title}：`));
  if (threadFailure) return threadFailure;
  if (reason.includes("Windows 沙箱尚未初始化")) return "请先初始化 Windows 安全沙箱";
  if (reason.includes("Windows 沙箱需要更新")) return "请先更新 Windows 安全沙箱";
  if (reason.includes("Windows 沙箱运行组件不完整")) return "Windows 安全沙箱组件不完整";
  if (reason.includes("无法确认 Windows 沙箱状态")) return "请检查 Windows 安全沙箱";
  if (reason.startsWith("AI 回复未通过结构化协议校验")) return "回复格式校验失败";
  if (reason.startsWith("模型额度不足：")) return reason;
  if (reason.includes("额度不足")) return "模型额度不足";
  if (/\bMODEL_REQUEST_REJECTED\b/.test(reason)) return "模型服务拒绝了请求";
  // 网关以 HTTP 状态拒绝的请求不是员工可处理的权限确认，文案里的“权限”二字不能作为判断依据
  const gatewayRejection = /unexpected status \d{3}/.test(reason);
  if (!gatewayRejection && (reason.includes("permission") || reason.includes("权限"))) return "运行需要权限处理";
  if (reason.includes("credential") || reason.includes("凭据")) return "模型凭据不可用";
  if (reason.includes("timed_out") || reason.includes("超时")) return "运行等待超时";
  return "运行未能完成";
}

function runStatusDescription(status: string) {
  if (status === "interrupted") return "运行被意外中断";
  if (status === "terminated") return "运行已由用户终止";
  if (status === "waiting_permission") return "需要确认后才能继续";
  if (status === "permission_approved") return "权限已确认，可以安全重试";
  return "查看本次运行详情";
}

function RunRecordIcon({ status }: { status: string }) {
  if (status === "completed") return <svg aria-hidden="true" viewBox="0 0 20 20"><circle cx="10" cy="10" r="7"/><path d="m6.8 10 2 2 4.4-4.4"/></svg>;
  if (["failed", "interrupted", "terminated"].includes(status)) return <svg aria-hidden="true" viewBox="0 0 20 20"><circle cx="10" cy="10" r="7"/><path d="M10 6.5v4M10 13.5h.01"/></svg>;
  return <svg aria-hidden="true" viewBox="0 0 20 20"><circle cx="10" cy="10" r="7"/><path d="M10 6v4l2.5 1.5"/></svg>;
}

function SandboxGateIcon() {
  return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="m10 2.5 6 2.6v4.2c0 4-2.4 6.9-6 8.2-3.6-1.3-6-4.2-6-8.2V5.1z"/><path d="M10 6.2v4.1M10 13.2h.01"/></svg>;
}

function runStatusLabel(status: string) { return conversationRunLabel(status); }


// 会话标题在创建时就写入了完整相对路径（如「讨论 00项目总览/项目目录结构与产品分类.md」）。
// 标题是已持久化的数据，这里只在渲染时压缩，不回写、不改动存储内容；
// 完整路径仍由上下文条和上下文面板的“文件”行完整呈现。
export function fileNameFromPath(path: string): string {
  const segments = path.replaceAll("\\", "/").split("/").filter((segment) => segment.length > 0);
  return segments.length ? segments[segments.length - 1] : path;
}

export function workspaceAbsolutePath(workspaceRoot: string, relativePath: string): string {
  const windows = workspaceRoot.includes("\\");
  const separator = windows ? "\\" : "/";
  const root = workspaceRoot.replace(/[\\/]+$/, "");
  const relative = windows ? relativePath.replaceAll("/", "\\") : relativePath.replaceAll("\\", "/");
  return `${root}${separator}${relative}`;
}

export function markdownWorkspaceFile(href: string, workspaceRoot: string, files: WorkspaceFileItem[]): WorkspaceFileItem | null {
  const path = workspaceRelativePath(href, workspaceRoot);
  return path ? files.find((file) => workspacePathsEqual(file.relativePath, path, workspaceRoot)) ?? null : null;
}

type ContextChip = { label: string; value: string; display: string; clear: (current: AiContext) => AiContext };

/** 会话实际带上的上下文，逐项可移除。没带上的不占位。 */
function contextChips(context: AiContext): ContextChip[] {
  type Candidate = { label: string; value: string | null; short?: (value: string) => string; clear: (current: AiContext) => AiContext };
  const candidates: Candidate[] = [
    { label: "阶段", value: context.phaseName, clear: (current) => ({ ...current, phaseId: null, phaseName: null }) },
    { label: "交付物", value: context.deliverableName, clear: (current) => ({ ...current, deliverableId: null, deliverableName: null }) },
  ];
  return candidates
    .filter((chip): chip is Candidate & { value: string } => Boolean(chip.value))
    .map(({ label, value, short, clear }) => ({ label, value, display: elideMiddle(short ? short(value) : value), clear }));
}

/** 标签上只留一眼能认出的长度。超出的从中间省略而不是掐尾——文件名的扩展名和结尾的
    编号都在尾部，掐尾会让一批同目录文件在标签上长得一模一样。完整值始终在悬停提示里。 */
export function elideMiddle(text: string, max = 10): string {
  if (text.length <= max) return text;
  const head = Math.ceil((max - 1) / 2);
  return `${text.slice(0, head)}…${text.slice(text.length - (max - 1 - head))}`;
}

export function conversationDisplayTitle(title: string): string {
  return title.split(/(\s+)/).map((part) => (part.includes("/") ? fileNameFromPath(part) : part)).join("");
}

export function conversationTitleFromFirstMessage(content: string, attachmentName?: string): string {
  const firstLine = content.split(/\r?\n/).map((line) => line.trim()).find(Boolean) ?? "";
  const title = firstLine.replace(/[#>*_`~\[\]]/g, "").replace(/\s+/g, " ").trim();
  return compactConversationText(title || attachmentName?.trim() || "附件对话", 32);
}

const genericConversationTitles = new Set(["新对话", "项目工作对话", "工作区对话"]);

/** 会话身份优先来自用户真正讨论的主题。历史数据不回写，文件仍只是可移除的上下文。 */
export function conversationTopic(conversation: AiConversation, messages: AiRuntimeSnapshot["messages"]): string {
  const firstPrompt = messages.find((message) => message.role === "user")?.content;
  const titleTarget = conversation.title.startsWith("讨论 ") ? conversation.title.slice(3).trim() : "";
  const fileNamed = Boolean(titleTarget) && (Boolean(conversation.context.relativePath) || titleTarget.includes("/") || /\.[\p{L}\p{N}]{1,10}$/u.test(titleTarget));
  if ((genericConversationTitles.has(conversation.title) || fileNamed) && firstPrompt) return compactConversationText(firstPrompt, 28);
  if (fileNamed) return "文件讨论";
  return compactConversationText(conversationDisplayTitle(conversation.title), 32);
}

export function conversationSummary(conversation: AiConversation, messages: AiRuntimeSnapshot["messages"], installations: Array<{ messageId: string | null; displayContent: string | null }> = [], runs: AiRuntimeSnapshot["runs"] = []): string {
  const latest = messages.at(-1);
  if (latest) {
    const installation = latest.role === "assistant" ? installations?.find(item => item.messageId === latest.messageId) : undefined;
    return compactConversationText(installation ? skillMessageText(assistantMessageBody(latest.content, runs.find(run => run.runId === latest.runId)?.classificationNonce)) : (latest.role === "assistant" ? assistantMessageBody(latest.content, runs.find(run => run.runId === latest.runId)?.classificationNonce) : latest.content), 34);
  }
  return conversation.context.relativePath ? `尚未发送消息 · 引用 ${fileNameFromPath(conversation.context.relativePath)}` : "尚未发送消息";
}

function compactConversationText(content: string, max: number): string {
  const text = markdownText(content).replace(/\s+/g, " ").trim();
  return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}



function parseDecisionCard(content: string): { question: string; options: string; impact: string; evidence: string; recommendation: string; consequence: string } | null {
  const headingMap: Array<[keyof ReturnTypeShape, string]> = [
    ["question", "决策问题"],
    ["options", "可选方案"],
    ["impact", "影响范围"],
    ["evidence", "证据"],
    ["recommendation", "建议与理由"],
    ["consequence", "不决定的后果"],
  ];
  const result: ReturnTypeShape = { question: "", options: "", impact: "", evidence: "", recommendation: "", consequence: "" };
  for (let index = 0; index < headingMap.length; index += 1) {
    const [key, heading] = headingMap[index];
    const start = content.search(new RegExp(`^#{1,3}\\s+${heading}\\s*$`, "m"));
    if (start < 0) return null;
    const bodyStart = content.indexOf("\n", start);
    if (bodyStart < 0) return null;
    const nextHeading = headingMap.slice(index + 1).map(([, value]) => content.search(new RegExp(`^#{1,3}\\s+${value}\\s*$`, "m"))).filter((value) => value > bodyStart).sort((left, right) => left - right)[0] ?? content.length;
    result[key] = content.slice(bodyStart + 1, nextHeading).trim();
    if (!result[key]) return null;
  }
  return result;
}
type ReturnTypeShape = { question: string; options: string; impact: string; evidence: string; recommendation: string; consequence: string };

function Icon({ children }: { children: ReactNode }) { return <svg aria-hidden="true" viewBox="0 0 24 24">{children}</svg>; }
function AiSparkIcon() { return <Icon><path d="m12 2 1.8 5.2L19 9l-5.2 1.8L12 16l-1.8-5.2L5 9l5.2-1.8L12 2Z"/><path d="m19 16 .8 2.2L22 19l-2.2.8L19 22l-.8-2.2L16 19l2.2-.8L19 16Z"/></Icon>; }
function PlusIcon() { return <Icon><path d="M12 5v14M5 12h14"/></Icon>; }
function PinIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M8 3h4l-.6 4.2 2.6 2.3H5.9l2.7-2.3z"/><path d="M10 9.5V16"/></svg>; }
function MoreDotsIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M5 10h.01M10 10h.01M15 10h.01"/></svg>; }
function ProjectFolderIcon({ expanded }: { expanded: boolean }) {
  return <svg aria-hidden="true" data-state={expanded ? "expanded" : "collapsed"} viewBox="0 0 20 20">
    {expanded ? <><path d="M2.5 14.8V5.5h5l2 2h8v2"/><path d="M3.8 9.5h13.7l-2 6H2.5l1.3-6Z"/></> : <path d="M2.5 5.5h5l2 2h8v8h-15v-10Z"/>}
  </svg>;
}
function ProjectGroupIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><rect x="2.5" y="2.5" width="6" height="6" rx="1"/><rect x="11.5" y="2.5" width="6" height="6" rx="1"/><rect x="2.5" y="11.5" width="6" height="6" rx="1"/><rect x="11.5" y="11.5" width="6" height="6" rx="1"/></svg>; }
function ConversationIcon() { return <Icon><path d="M5 5.5h14v10H9l-4 3v-13Z"/></Icon>; }
function ContextUsageIcon({ percent }: { percent: number }) {
  const circumference = Math.PI * 17;
  return <svg aria-hidden="true" viewBox="0 0 24 24"><circle className="ai-context-usage-track" cx="12" cy="12" r="8.5"/><circle className="ai-context-usage-value" cx="12" cy="12" r="8.5" strokeDasharray={`${circumference * percent / 100} ${circumference}`}/></svg>;
}
function ShieldIcon() { return <Icon><path d="M12 3 19 6v5c0 4.5-2.8 8-7 10-4.2-2-7-5.5-7-10V6l7-3Z"/><path d="m9 12 2 2 4-4"/></Icon>; }
function ApprovalIcon() { return <Icon><path d="M8.5 11V6.5a1.5 1.5 0 0 1 3 0V10 5.5a1.5 1.5 0 0 1 3 0V10 7a1.5 1.5 0 0 1 3 0v6.5A6.5 6.5 0 0 1 11 20h-.5A6.5 6.5 0 0 1 4 13.5V11a1.5 1.5 0 0 1 3 0v2"/></Icon>; }
function RunsIcon() { return <Icon><path d="M5 12h3l2-5 4 10 2-5h3"/><path d="M4 4h16v16H4z"/></Icon>; }
function EnterIcon() { return <svg aria-hidden="true" className="ai-enter-icon" viewBox="0 0 24 24"><path d="M19 5v5a5 5 0 0 1-5 5H5"/><path d="m9 11-4 4 4 4"/></svg>; }
/* 递减的三条线：这是一列待办中的一条。此前用的是时钟，读起来是"在等时间"，而不是"排在队里" */
function QueueIcon() { return <Icon><path d="M4.5 7h15"/><path d="M4.5 12h10"/><path d="M4.5 17h6"/></Icon>; }
/* 折线箭头：把这一条拐进正在进行的那一轮里 */
function QueueAdjustIcon() { return <Icon><path d="M6.5 5v6.5a3 3 0 0 0 3 3h8"/><path d="m14.5 11 3.5 3.5-3.5 3.5"/></Icon>; }
function StopIcon() { return <Icon><rect x="7" y="7" width="10" height="10" rx="1.5"/></Icon>; }
function CopyIcon() { return <Icon><rect x="8" y="8" width="11" height="11" rx="2"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/></Icon>; }
function EditIcon() { return <Icon><path d="M5 19h4L19 9l-4-4L5 15v4Z"/><path d="m13.5 6.5 4 4"/></Icon>; }
function RefreshIcon() { return <Icon><path d="M20 6v5h-5"/><path d="M18 16a7 7 0 1 1 1.5-7.6L20 11"/></Icon>; }
function CloseIcon() { return <Icon><path d="m7 7 10 10M17 7 7 17"/></Icon>; }
function ExpandIcon() { return <Icon><path d="M9 4H4v5M15 4h5v5M9 20H4v-5M15 20h5v-5"/></Icon>; }
function ChevronIcon({ direction }: { direction: "down" | "left" | "right" }) {
  const path = direction === "down" ? "m7 10 5 5 5-5" : direction === "right" ? "m10 7 5 5-5 5" : "m14 7-5 5 5 5";
  return <Icon><path d={path}/></Icon>;
}
function ConversationRailToggle({ expanded, onToggle }: { expanded: boolean; onToggle: () => void }) {
  const label = expanded ? "收起对话记录" : "展开对话记录";
  return <button aria-controls="ai-conversation-rail" aria-expanded={expanded} aria-label={label} className="ai-conversation-rail-toggle" data-tip={label} onClick={onToggle} type="button"><PanelToggleIcon expanded={expanded}/></button>;
}
function ArrowDownIcon() { return <Icon><path d="M12 5v13M7 13l5 5 5-5"/></Icon>; }
function DownIcon() { return <Icon><path d="m8 10 4 4 4-4"/></Icon>; }
function ReasoningActivityIcon() { return <Icon><path d="M8 16.5h8M9.5 20h5"/><path d="M8.2 13.5A6 6 0 1 1 15.8 13.5c-1 .8-1.5 1.5-1.6 2.2H9.8c-.1-.7-.6-1.4-1.6-2.2Z"/></Icon>; }
function CommandActivityIcon() { return <Icon><rect x="3.5" y="5" width="17" height="14" rx="2.5"/><path d="m7 10 2.5 2L7 14M12 15h5"/></Icon>; }
function ToolActivityIcon() { return <Icon><path d="M8.5 4.5 12 3l3.5 1.5L17 8l3.5 1.5V14L17 15.5 15.5 19 12 20.5 8.5 19 7 15.5 3.5 14V9.5L7 8l1.5-3.5Z"/><circle cx="12" cy="12" r="2.5"/></Icon>; }
function FileChangeActivityIcon() { return <Icon><path d="M6 3h8l4 4v14H6V3Z"/><path d="M14 3v5h5M9 16l5-5 2 2-5 5H9v-2Z"/></Icon>; }
function PlanActivityIcon() { return <Icon><path d="m5 7 1.5 1.5L9 6M11 7h8M5 12l1.5 1.5L9 11M11 12h8M5 17l1.5 1.5L9 16M11 17h8"/></Icon>; }
function ContextActivityIcon() { return <Icon><path d="M4 5h16M4 19h16M8 9l4 3 4-3M8 15l4-3 4 3"/></Icon>; }
function SearchBusinessIcon() { return <Icon><circle cx="10.5" cy="10.5" r="5.5"/><path d="m15 15 5 5"/></Icon>; }
function WriteBusinessIcon() { return <Icon><path d="M5 19h4L19 9l-4-4L5 15v4Z"/><path d="m13.5 6.5 4 4M5 19l3-3"/></Icon>; }
function FillBusinessIcon() { return <Icon><rect x="5" y="3" width="14" height="18" rx="2"/><path d="M9 3.5h6V7H9zM9 11h6M9 15h6"/></Icon>; }
function AdvanceBusinessIcon() { return <Icon><path d="M4 18V6M4 18h16"/><path d="m7 14 4-4 3 2 5-6"/><path d="M16 6h3v3"/></Icon>; }
function ArrowRightIcon() { return <Icon><path d="M6 12h12M14 8l4 4-4 4"/></Icon>; }
function FileIcon() { return <Icon><path d="M6 3h8l4 4v14H6V3Z"/><path d="M14 3v5h5M9 12h6M9 16h6"/></Icon>; }
function ImageMaterialIcon() { return <Icon><rect x="4" y="4" width="16" height="16" rx="2.5"/><circle cx="9" cy="9" r="1.5"/><path d="m6.5 17 4-4 2.5 2.5 2-2 2.5 3.5"/></Icon>; }
function OutputIcon() { return <Icon><rect x="3" y="4" width="18" height="16" rx="3"/><path d="M7 16v-4M11 16V9M15 16v-6M19 16V7"/></Icon>; }
function ContextAddIcon() { return <Icon><path d="M3.5 6.5h6l2 2h9v11h-17v-13Z"/><path d="M15.5 11.5v5M13 14h5"/></Icon>; }
function CheckIcon() { return <Icon><path d="m6 12 4 4 8-8"/></Icon>; }
function PaperclipIcon() { return <Icon><path d="m9 12.5 5.8-5.8a3 3 0 0 1 4.2 4.2l-7.3 7.3a5 5 0 0 1-7.1-7.1l7-7"/><path d="m7.5 14.5 7.4-7.4"/></Icon>; }
function MediaIcon() { return <Icon><rect x="3.5" y="5" width="17" height="14" rx="2.5"/><circle cx="9" cy="10" r="1.5"/><path d="m5.5 17 4-4 3 3 2.5-2.5 3.5 3.5"/><path d="m16 7.5 3 2-3 2v-4Z"/></Icon>; }
function SkillIcon() { return <Icon><path d="M9 3h6l1 3 3 1v6l-3 1-1 3H9l-1-3-3-1V7l3-1 1-3Z"/><circle cx="12" cy="10" r="2.5"/></Icon>; }

function ReuseIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M4 7a6 6 0 0 1 10-2l2 2M16 3v4h-4M16 13a6 6 0 0 1-10 2l-2-2M4 17v-4h4"/></svg>; }

function ArtifactsSourcesIcon({ open }: { open: boolean }) {
  return <svg aria-hidden="true" data-open={open} viewBox="0 0 20 20">
    {open ? <path className="ai-info-panel-fill" d="M5 2.5h6.5L16 7v9.5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1Z"/> : null}
    <path d="M11.5 2.5H5a1 1 0 0 0-1 1v13a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V7Zm0 0V7H16M7 10h6M7 13h4"/>
  </svg>;
}
