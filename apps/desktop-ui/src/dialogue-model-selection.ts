import type { AiAgentRun, AiModelInfo, AiReasoningEffort } from "./ai-runtime";

const storageKey = "cilai.workbench.ai-last-execution-selection";
export type ModelSelection = { modelId: string | null; reasoningEffort: AiReasoningEffort | null };
export type ExecutionSelection = ModelSelection & { agentId: string | null; selectedAt?: number };

const reasoningLabels: Record<AiReasoningEffort, string> = {
  none: "关闭", minimal: "极低", low: "较低", medium: "标准", high: "较高", xhigh: "更高", max: "最高",
};
function key(profileId?: string | null, conversationId?: string) {
  const scoped = profileId ? `${storageKey}:${profileId}` : storageKey;
  return conversationId ? `${scoped}:conversation:${conversationId}` : scoped;
}

export function readExecutionSelection(profileId?: string | null, conversationId?: string): ExecutionSelection | null {
  const raw = window.localStorage.getItem(key(profileId, conversationId));
  if (raw === null) return null;
  const value = JSON.parse(raw) as ExecutionSelection | null;
  const effort = value?.reasoningEffort ?? null;
  if (!value || (value.agentId !== null && typeof value.agentId !== "string") || (value.modelId !== null && typeof value.modelId !== "string") || (effort !== null && !Object.hasOwn(reasoningLabels, effort))) {
    throw new Error("保存的模型选择格式无效，请重新选择。");
  }
  return { ...value, reasoningEffort: effort };
}

export function saveExecutionSelection(selection: ExecutionSelection, profileId?: string | null, conversationId?: string) {
  window.localStorage.setItem(key(profileId), JSON.stringify(selection));
  if (conversationId) window.localStorage.setItem(key(profileId, conversationId), JSON.stringify({ ...selection, selectedAt: Date.now() }));
}

export function modelSelectionFromRun(run: Pick<AiAgentRun, "model" | "reasoningEffort" | "modelSelection">): ModelSelection {
  return {
    modelId: run.model,
    reasoningEffort: run.modelSelection?.reasoningEffort == null ? null : run.reasoningEffort ?? null,
  };
}

export function reasoningLabel(effort: AiReasoningEffort, model: AiModelInfo | null) {
  return model?.reasoningControl === "toggle"
    ? (model.reasoningEnabledEfforts?.includes(effort) ? "开启" : "关闭")
    : reasoningLabels[effort];
}
