import {
type AiAgentRun,
type AiContext,
type AiConversation,
type AiRunEvent,
type AiRuntimeSnapshot
} from "./ai-runtime";
import type { WorkspaceFileItem } from "./generated/protocol";
import { markdownText } from "./markdown-text";
import { skillMessageText } from "./skill-message";
import { workspacePathsEqual,workspaceRelativePath } from "./workspace-path";


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

export function isBackgroundToolDiscoveryEvent(event: AiRunEvent) {
  return ["application_discovery.", "enterprise_tool_snapshot.", "tool_search."].some((prefix) =>
    event.eventType.startsWith(prefix),
  );
}

export function neutralRunProgress() {
  return "正在处理…";
}

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

export const genericConversationTitles = new Set(["新对话", "项目工作对话", "工作区对话"]);

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

export function compactConversationText(content: string, max: number): string {
  const text = markdownText(content).replace(/\s+/g, " ").trim();
  return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}

export function parseDecisionCard(content: string): { question: string; options: string; impact: string; evidence: string; recommendation: string; consequence: string } | null {
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

export type ReturnTypeShape = { question: string; options: string; impact: string; evidence: string; recommendation: string; consequence: string };

export type ContextChip = { label: string; value: string; display: string; clear: (current: AiContext) => AiContext };

export function contextChips(context: AiContext): ContextChip[] {
  type Candidate = { label: string; value: string | null; short?: (value: string) => string; clear: (current: AiContext) => AiContext };
  const candidates: Candidate[] = [
    { label: "阶段", value: context.phaseName, clear: (current) => ({ ...current, phaseId: null, phaseName: null }) },
    { label: "交付物", value: context.deliverableName, clear: (current) => ({ ...current, deliverableId: null, deliverableName: null }) },
  ];
  return candidates
    .filter((chip): chip is Candidate & { value: string } => Boolean(chip.value))
    .map(({ label, value, short, clear }) => ({ label, value, display: elideMiddle(short ? short(value) : value), clear }));
}
