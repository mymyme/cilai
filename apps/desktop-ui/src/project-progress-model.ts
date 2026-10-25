import {
type AiContext
} from "./ai-runtime";
import type {
BusinessTaskItem,
PhasePlanItem,
ProjectPhaseItem,
ProjectSummary,
WorkspaceSummary
} from "./generated/protocol";


export const phaseCommentRequired = (phase: ProjectPhaseItem, target: string) =>
  ["completed", "paused", "terminated"].includes(target) ||
  (phase.status === "review" && target === "in_progress");

export const taskCommentRequired = (target: string) =>
  ["completed", "rework"].includes(target);

export const attentionCommentRequired = (target: string, decisionAction?: string) =>
  target === "resolved" || Boolean(decisionAction);

export const phaseStatusLabel: Record<string, string> = {
  planned: "待启动",
  in_progress: "进行中",
  review: "阶段评审",
  completed: "已完成",
  paused: "已暂停",
  terminated: "已终止",
};

export const taskStatusLabel: Record<string, string> = {
  candidate: "候选",
  todo: "待办",
  in_progress: "进行中",
  decision: "待决策",
  review: "待验收",
  rework: "返工",
  completed: "已完成",
  terminated: "已终止",
};

export const projectTaskGroups = [
  { key: "todo", label: "待处理", statuses: ["candidate", "todo", "rework"] },
  { key: "decision", label: "待决策", statuses: ["decision"] },
  { key: "review", label: "待验收", statuses: ["review"] },
  { key: "in_progress", label: "进行中", statuses: ["in_progress"] },
  { key: "completed", label: "已完成", statuses: ["completed"] },
] as const;

export const taskPriorityLabel: Record<string, string> = { critical: "紧急", high: "高", medium: "中", low: "低" };

export const nextTaskStatus: Record<string, string | undefined> = {
  candidate: "todo",
  todo: "in_progress",
  in_progress: "review",
  review: "completed",
  rework: "in_progress",
};

export function emptyPhase(deliverableId = ""): PhaseDraft {
  return {
    name: "",
    goal: "",
    acceptanceCriteria: "",
    requiredDeliverableId: deliverableId,
    predecessorIndexes: [],
  };
}

export interface PhaseDraft {
  name: string;
  goal: string;
  acceptanceCriteria: string;
  requiredDeliverableId: string;
  predecessorIndexes: number[];
}

export function replaceTask(
  plans: PhasePlanItem[],
  task: BusinessTaskItem,
): PhasePlanItem[] {
  return plans.map((plan) => ({
    ...plan,
    phases: plan.phases.map((phase) =>
      phase.phaseId === task.phaseId
        ? {
            ...phase,
            tasks: phase.tasks.map((item) =>
              item.businessTaskId === task.businessTaskId ? task : item,
            ),
          }
        : phase,
    ),
  }));
}

export function progressAiContext(
  workspace: WorkspaceSummary,
  project: ProjectSummary,
  phase: ProjectPhaseItem | null,
  task: BusinessTaskItem | null,
): AiContext {
  return {
    workspaceId: workspace.workspaceId,
    workspaceName: workspace.displayName,
    workspaceRoot: workspace.workspaceKind === "local" ? workspace.localRoot : "",
    teamSpaceId: workspace.teamSpaceId,
    projectId: project.projectId,
    projectName: project.displayName,
    phaseId: phase?.phaseId ?? null,
    phaseName: phase?.name ?? null,
    taskId: task?.businessTaskId ?? null,
    taskName: task?.title ?? null,
    deliverableId: phase?.requiredDeliverableId ?? null,
    deliverableName: phase?.requiredDeliverableTitle ?? null,
    localFileId: null,
    relativePath: null,
    fileRevision: null,
  };
}
