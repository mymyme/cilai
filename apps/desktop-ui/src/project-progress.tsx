import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { randomUuid } from "./uuid";
import type {
  AttentionCaseItem,
  AttentionCaseListResult,
  AttentionCaseTransitionResult,
  BusinessTaskItem,
  BusinessTaskMoveResult,
  BusinessTaskTransitionResult,
  DecisionRequestCreateResult,
  DeliverableItem,
  DeliverableListResult,
  PhasePlanActivateResult,
  PhasePlanCreateResult,
  PhasePlanItem,
  ProjectListResult,
  ProjectGetResult,
  ProjectPhaseItem,
  ProjectPhaseTransitionResult,
  ProjectBlockerCreateResult,
  ProjectProgressGetResult,
  ProjectSummary,
  WorkspaceFileItem,
  WorkspaceSummary,
  MyWorkItem,
} from "./generated/protocol";
import { rpc, sha256 } from "./app-server-rpc";
import {
  loadAiSnapshot,
  type AiAgentRun,
  type AiContext,
  type AiConversation,
  type AiConversationMode,
  type AiConversationType,
} from "./ai-runtime";
import { ConfirmDialog } from "./confirm-dialog";
import { DeliverableManager } from "./project-deliverables";
import { Select } from "./select";
import { SearchField } from "./search-field";
import { TaskArtifactList, type TaskArtifactPreview } from "./task-artifacts";
import { ImageArtifactLightbox } from "./artifact-preview";
import { conversationFileArtifacts } from "./ai-artifacts";
import type { MessageSink } from "./app-message";
import { LoadFailure } from "./load-failure";
import { TaskDetailDrawer } from "./my-work";

interface ProjectProgressPageProps {
  hostPlatform: string | null;
  operatorName?: string;
  workspace: WorkspaceSummary | null;
  files: WorkspaceFileItem[];
  eventSequence: number;
  initialProjectId: string | null;
  initialPhaseId: string | null;
  onMessage: MessageSink;
  onOpenAi: (request: {
    title: string;
    conversationType: AiConversationType;
    mode?: AiConversationMode;
    context: AiContext;
  }) => void;
  onOpenConversation: (conversationId: string) => void;
  onOpenRun: (runId: string) => void;
  onOpenFile: (localFileId: string, workspaceId: string) => void;
  onExitProject: () => void;
  onPhaseSelected: (phaseId: string | null, phaseName: string | null) => void;
}

interface PhaseDraft {
  name: string;
  goal: string;
  acceptanceCriteria: string;
  requiredDeliverableId: string;
  predecessorIndexes: number[];
}

/**
 * A written conclusion belongs to one transition, so these predicates decide which transitions have
 * to carry one. They are shared by the guard inside each action and by the caller that decides
 * whether to open the confirmation dialog, so the two can never disagree.
 */
const phaseCommentRequired = (phase: ProjectPhaseItem, target: string) =>
  ["completed", "paused", "terminated"].includes(target) ||
  (phase.status === "review" && target === "in_progress");
const taskCommentRequired = (target: string) =>
  ["completed", "rework"].includes(target);
const attentionCommentRequired = (target: string, decisionAction?: string) =>
  target === "resolved" || Boolean(decisionAction);

const phaseStatusLabel: Record<string, string> = {
  planned: "待启动",
  in_progress: "进行中",
  review: "阶段评审",
  completed: "已完成",
  paused: "已暂停",
  terminated: "已终止",
};

const taskStatusLabel: Record<string, string> = {
  candidate: "候选",
  todo: "待办",
  in_progress: "进行中",
  decision: "待决策",
  review: "待验收",
  rework: "返工",
  completed: "已完成",
  terminated: "已终止",
};

const projectTaskGroups = [
  { key: "todo", label: "待处理", statuses: ["candidate", "todo", "rework"] },
  { key: "decision", label: "待决策", statuses: ["decision"] },
  { key: "review", label: "待验收", statuses: ["review"] },
  { key: "in_progress", label: "进行中", statuses: ["in_progress"] },
  { key: "completed", label: "已完成", statuses: ["completed"] },
] as const;

const taskPriorityLabel: Record<string, string> = { critical: "紧急", high: "高", medium: "中", low: "低" };

const nextTaskStatus: Record<string, string | undefined> = {
  candidate: "todo",
  todo: "in_progress",
  in_progress: "review",
  review: "completed",
  rework: "in_progress",
};

function emptyPhase(deliverableId = ""): PhaseDraft {
  return {
    name: "",
    goal: "",
    acceptanceCriteria: "",
    requiredDeliverableId: deliverableId,
    predecessorIndexes: [],
  };
}

export function ProjectProgressPage(props: ProjectProgressPageProps) {
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [deliverables, setDeliverables] = useState<DeliverableItem[]>([]);
  const [plans, setPlans] = useState<PhasePlanItem[]>([]);
  const [projectTasks, setProjectTasks] = useState<BusinessTaskItem[]>([]);
  const [activeTaskDetail, setActiveTaskDetail] = useState<MyWorkItem | null>(null);
  const [taskStatusFilter, setTaskStatusFilter] = useState("all");
  const [artifactPreview, setArtifactPreview] = useState<TaskArtifactPreview | null>(null);
  const [projectView, setProjectView] = useState<
    "overview" | "phases" | "tasks"
  >(props.initialPhaseId ? "phases" : "overview");
  const [projectConversations, setProjectConversations] = useState<
    AiConversation[]
  >([]);
  const [projectRuns, setProjectRuns] = useState<AiAgentRun[]>([]);
  const [selectedPhaseId, setSelectedPhaseId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  // 读不出来和这里是空的必须分开：此前失败会落进"尚无阶段计划"，那是一句假话。
  const [loadError, setLoadError] = useState<unknown>(null);
  const [reloadToken, setReloadToken] = useState(0);
  const [busy, setBusy] = useState(false);
  const [showDeliverables, setShowDeliverables] = useState(false);
  const [showPlanForm, setShowPlanForm] = useState(false);
  const [planSummary, setPlanSummary] = useState("");
  const [phaseDrafts, setPhaseDrafts] = useState<PhaseDraft[]>([emptyPhase()]);
  const [operator, setOperator] = useState(props.operatorName ?? "");
  useEffect(() => setOperator(props.operatorName ?? ""), [props.operatorName]);
  const [pendingTransition, setPendingTransition] = useState<{
    title: string;
    description: string;
    confirmLabel: string;
    commentLabel: string;
    commentPlaceholder: string;
    danger?: boolean;
    run: (comment: string) => Promise<void>;
  } | null>(null);
  const [phaseMoreOpen, setPhaseMoreOpen] = useState(false);
  const phaseMoreRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!phaseMoreOpen) return;
    const closeOnOutside = (event: PointerEvent) => {
      if (!phaseMoreRef.current?.contains(event.target as Node))
        setPhaseMoreOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setPhaseMoreOpen(false);
    };
    window.addEventListener("pointerdown", closeOnOutside);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", closeOnOutside);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [phaseMoreOpen]);
  const [attentionCases, setAttentionCases] = useState<AttentionCaseItem[]>([]);
  const [attentionMode, setAttentionMode] = useState<
    "blocker" | "decision" | null
  >(null);
  const [attentionTitle, setAttentionTitle] = useState("");
  const [attentionDetail, setAttentionDetail] = useState("");
  const [attentionOwner, setAttentionOwner] = useState("");
  const [attentionPriority, setAttentionPriority] = useState("high");
  const [decisionOptions, setDecisionOptions] = useState("");
  const [decisionImpact, setDecisionImpact] = useState("");
  const [decisionEvidence, setDecisionEvidence] = useState("");
  const [decisionRecommendation, setDecisionRecommendation] = useState("");
  const [decisionConsequence, setDecisionConsequence] = useState("");

  const selectedProject =
    projects.find((project) => project.projectId === projectId) ?? null;
  const activePlan = plans.find((plan) => plan.status === "active") ?? null;
  const draftPlan = plans.find((plan) => plan.status === "draft") ?? null;
  const visiblePlan = activePlan ?? draftPlan ?? null;
  const selectedPhase =
    visiblePlan?.phases.find((phase) => phase.phaseId === selectedPhaseId) ??
    visiblePlan?.phases.find((phase) => phase.isMain) ??
    visiblePlan?.phases.find(
      (phase) => phase.status !== "completed" && phase.status !== "terminated",
    ) ??
    visiblePlan?.phases[0] ??
    null;

  useEffect(() => {
    if (loading) return;
    props.onPhaseSelected(
      selectedPhase?.phaseId ?? null,
      selectedPhase?.name ?? null,
    );
  }, [
    loading,
    props.onPhaseSelected,
    selectedPhase?.phaseId,
    selectedPhase?.name,
  ]);

  // Tracks the latest initialPhaseId without making loadProgress depend on it: onPhaseSelected
  // above writes every in-page phase click back into initialPhaseId, so making loadProgress react
  // to it would re-trigger the whole load cascade on every phase click.
  const initialPhaseIdRef = useRef(props.initialPhaseId);
  initialPhaseIdRef.current = props.initialPhaseId;

  const loadProgress = useCallback(async (nextProjectId: string) => {
    const [deliverableResult, progressResult, attentionResult, aiSnapshot] =
      await Promise.all([
        rpc<DeliverableListResult>("deliverable/list", {
          projectId: nextProjectId,
        }),
        rpc<ProjectProgressGetResult>("projectProgress/get", {
          projectId: nextProjectId,
        }),
        rpc<AttentionCaseListResult>("attentionCase/list", {
          projectId: nextProjectId,
        }),
        loadAiSnapshot(),
      ]);
    setDeliverables(deliverableResult.deliverables);
    setPlans(progressResult.plans);
    setProjectTasks(progressResult.tasks);
    setAttentionCases(attentionResult.cases);
    const conversations = aiSnapshot.conversations.filter(
      (conversation) =>
        !conversation.archived &&
        conversation.context.projectId === nextProjectId,
    );
    const conversationIds = new Set(
      conversations.map((conversation) => conversation.conversationId),
    );
    setProjectConversations(conversations);
    setProjectRuns(
      aiSnapshot.runs.filter((run) => conversationIds.has(run.conversationId)),
    );
    const currentPlan =
      progressResult.plans.find((plan) => plan.status === "active") ??
      progressResult.plans.find((plan) => plan.status === "draft") ??
      null;
    const requestedPhaseId = initialPhaseIdRef.current;
    setSelectedPhaseId((current) =>
      requestedPhaseId &&
      currentPlan?.phases.some((phase) => phase.phaseId === requestedPhaseId)
        ? requestedPhaseId
        : current &&
            currentPlan?.phases.some((phase) => phase.phaseId === current)
          ? current
          : (currentPlan?.phases.find((phase) => phase.isMain)?.phaseId ??
            currentPlan?.phases.find((phase) => phase.status !== "completed")
              ?.phaseId ??
            currentPlan?.phases[0]?.phaseId ??
            null),
    );
  }, []);

  const loadProjects = useCallback(
    async (workspaceId: string) => {
      const result = await rpc<ProjectListResult>("project/list", {
        workspaceId,
      });
      setProjects(result.projects);
      const requestedProjectMissing =
        Boolean(props.initialProjectId) &&
        !result.projects.some(
          (project) => project.projectId === props.initialProjectId,
        );
      if (requestedProjectMissing)
        props.onMessage(
          "指定的项目在当前工作区中未找到，可能已失效或已解除关联。",
          "error",
        );
      const nextProjectId = props.initialProjectId
        ? requestedProjectMissing
          ? null
          : props.initialProjectId
        : (result.projects[0]?.projectId ?? null);
      setProjectId(nextProjectId);
      if (nextProjectId) await loadProgress(nextProjectId);
      else {
        setDeliverables([]);
        setPlans([]);
        setProjectTasks([]);
        setAttentionCases([]);
        setProjectConversations([]);
        setProjectRuns([]);
      }
    },
    [loadProgress, props.initialProjectId, props.onMessage],
  );

  useEffect(() => {
    if (!props.workspace && !props.initialProjectId) {
      setProjects([]);
      setProjectId(null);
      setPlans([]);
      setProjectTasks([]);
      setAttentionCases([]);
      setProjectConversations([]);
      setProjectRuns([]);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    const load = props.initialProjectId
      ? rpc<ProjectGetResult>("project/get", {
          projectId: props.initialProjectId,
        }).then(async (result) => {
          if (cancelled) return;
          setProjects([result.project]);
          setProjectId(result.project.projectId);
          await loadProgress(result.project.projectId);
        })
      : loadProjects(props.workspace!.workspaceId);
    void load
      .catch((error) => {
        if (!cancelled) {
          setLoadError(error);
          props.onMessage(String(error), "error");
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [
    loadProgress,
    loadProjects,
    props.eventSequence,
    props.initialProjectId,
    props.onMessage,
    props.workspace,
    reloadToken,
  ]);

  const refresh = async () => {
    if (projectId) await loadProgress(projectId);
  };

  const updatePhaseDraft = (index: number, patch: Partial<PhaseDraft>) => {
    setPhaseDrafts((current) =>
      current.map((phase, phaseIndex) =>
        phaseIndex === index ? { ...phase, ...patch } : phase,
      ),
    );
  };

  const addPhaseDraft = () => {
    setPhaseDrafts((current) => [
      ...current,
      {
        ...emptyPhase(),
        predecessorIndexes: current.length ? [current.length - 1] : [],
      },
    ]);
  };

  const removePhaseDraft = (index: number) => {
    setPhaseDrafts((current) =>
      current
        .filter((_, phaseIndex) => phaseIndex !== index)
        .map((phase) => ({
          ...phase,
          predecessorIndexes: phase.predecessorIndexes
            .filter((predecessor) => predecessor !== index)
            .map((predecessor) =>
              predecessor > index ? predecessor - 1 : predecessor,
            ),
        })),
    );
  };

  const createPlan = async () => {
    if (
      !projectId ||
      busy ||
      !planSummary.trim() ||
      phaseDrafts.some(
        (phase) =>
          !phase.name.trim() ||
          !phase.goal.trim() ||
          !phase.acceptanceCriteria.trim(),
      )
    )
      return;
    setBusy(true);
    try {
      const clientOperationId = `desktop-phase-plan-${randomUuid()}`;
      const fingerprintSource = JSON.stringify({
        projectId,
        planSummary: planSummary.trim(),
        phaseDrafts,
      });
      const requestFingerprint = await sha256(fingerprintSource);
      const result = await rpc<PhasePlanCreateResult>("phasePlan/create", {
        projectId,
        summary: planSummary.trim(),
        phases: phaseDrafts.map((phase) => ({
          name: phase.name.trim(),
          goal: phase.goal.trim(),
          acceptanceCriteria: phase.acceptanceCriteria.trim(),
          requiredDeliverableId: phase.requiredDeliverableId || null,
          predecessorIndexes: phase.predecessorIndexes,
        })),
        clientOperationId,
        requestFingerprint,
      });
      setPlans((current) => [result.plan, ...current]);
      setSelectedPhaseId(result.plan.phases[0]?.phaseId ?? null);
      setShowPlanForm(false);
      setPlanSummary("");
      setPhaseDrafts([emptyPhase()]);
      props.onMessage("阶段计划草案已创建。激活前仍不会改变当前项目推进状态。");
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const activatePlan = async (plan: PhasePlanItem) => {
    if (busy) return;
    setBusy(true);
    try {
      const clientOperationId = `desktop-phase-plan-activate-${randomUuid()}`;
      const requestFingerprint = await sha256(
        `${plan.planId}:${plan.stateRevision}`,
      );
      const result = await rpc<PhasePlanActivateResult>("phasePlan/activate", {
        planId: plan.planId,
        expectedRevision: plan.stateRevision,
        clientOperationId,
        requestFingerprint,
      });
      setPlans((current) =>
        current.map((item) =>
          item.planId === result.plan.planId ? result.plan : item,
        ),
      );
      setSelectedPhaseId(result.plan.phases[0]?.phaseId ?? null);
      props.onMessage("阶段计划已激活，可以启动首个阶段。");
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const transitionPhase = async (
    phase: ProjectPhaseItem,
    targetStatus: string,
    comment: string,
  ) => {
    if (busy || !operator.trim()) return;
    if (phaseCommentRequired(phase, targetStatus) && !comment) return;
    setBusy(true);
    try {
      const clientOperationId = `desktop-phase-transition-${randomUuid()}`;
      const requestFingerprint = await sha256(
        [
          phase.phaseId,
          targetStatus,
          String(phase.stateRevision),
          operator.trim(),
          comment,
        ].join("\n"),
      );
      await rpc<ProjectPhaseTransitionResult>("projectPhase/transition", {
        phaseId: phase?.phaseId ?? null,
        targetStatus,
        expectedRevision: phase.stateRevision,
        actorLabel: operator.trim(),
        comment,
        clientOperationId,
        requestFingerprint,
      });
      await refresh();
      props.onMessage(
        targetStatus === "completed"
          ? "阶段验收完成，后继阶段现在可以启动。"
          : "阶段状态已更新。",
      );
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const startTask = (phase: ProjectPhaseItem | null) => {
    if (!props.workspace || !selectedProject) return;
    props.onOpenAi({
      title: "新建任务",
      conversationType: "project",
      mode: "discuss",
      context: progressAiContext(props.workspace, selectedProject, phase, null),
    });
  };

  const transitionTask = async (
    task: BusinessTaskItem,
    targetStatus: string,
    comment: string,
  ) => {
    if (busy || !operator.trim()) return;
    if (taskCommentRequired(targetStatus) && !comment) return;
    setBusy(true);
    try {
      const clientOperationId = `desktop-business-task-transition-${randomUuid()}`;
      const requestFingerprint = await sha256(
        [
          task.businessTaskId,
          targetStatus,
          String(task.stateRevision),
          operator.trim(),
          comment,
        ].join("\n"),
      );
      const result = await rpc<BusinessTaskTransitionResult>(
        "businessTask/transition",
        {
          businessTaskId: task.businessTaskId,
          targetStatus,
          expectedRevision: task.stateRevision,
          actorLabel: operator.trim(),
          comment,
          clientOperationId,
          requestFingerprint,
        },
      );
      setPlans((current) => replaceTask(current, result.task));
      setProjectTasks((current) =>
        current.map((item) =>
          item.businessTaskId === result.task.businessTaskId
            ? result.task
            : item,
        ),
      );
      props.onMessage(
        targetStatus === "rework"
          ? "任务已进入新的返工尝试，原验收反馈继续保留。"
          : "任务状态已更新。",
      );
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const moveTask = async (
    task: BusinessTaskItem,
    input: {
      targetPhaseId: string;
      dependencyImpact: string;
      deliverableImpact: string;
      acceptanceImpact: string;
    },
  ) => {
    if (busy || !operator.trim()) return;
    setBusy(true);
    try {
      const clientOperationId = `desktop-business-task-move-${randomUuid()}`;
      const requestFingerprint = await sha256(
        [
          task.businessTaskId,
          input.targetPhaseId,
          String(task.stateRevision),
          operator.trim(),
          input.dependencyImpact.trim(),
          input.deliverableImpact.trim(),
          input.acceptanceImpact.trim(),
        ].join("\n"),
      );
      await rpc<BusinessTaskMoveResult>("businessTask/move", {
        businessTaskId: task.businessTaskId,
        targetPhaseId: input.targetPhaseId,
        expectedRevision: task.stateRevision,
        actorLabel: operator.trim(),
        dependencyImpact: input.dependencyImpact.trim(),
        deliverableImpact: input.deliverableImpact.trim(),
        acceptanceImpact: input.acceptanceImpact.trim(),
        clientOperationId,
        requestFingerprint,
      });
      await refresh();
      props.onMessage("任务已移动到目标阶段，影响说明已写入审计记录。");
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const resetAttentionForm = () => {
    setAttentionMode(null);
    setAttentionTitle("");
    setAttentionDetail("");
    setAttentionOwner("");
    setAttentionPriority("high");
    setDecisionOptions("");
    setDecisionImpact("");
    setDecisionEvidence("");
    setDecisionRecommendation("");
    setDecisionConsequence("");
  };

  const createAttentionCase = async () => {
    if (
      !projectId ||
      !selectedPhase ||
      !attentionMode ||
      busy ||
      !operator.trim() ||
      !attentionTitle.trim() ||
      !attentionDetail.trim() ||
      !attentionOwner.trim() ||
      (attentionMode === "decision" &&
        [
          decisionOptions,
          decisionImpact,
          decisionEvidence,
          decisionRecommendation,
          decisionConsequence,
        ].some((value) => !value.trim()))
    )
      return;
    setBusy(true);
    try {
      const clientOperationId = `desktop-attention-create-${randomUuid()}`;
      const requestFingerprint = await sha256(
        [
          attentionMode,
          projectId,
          selectedPhase.phaseId,
          attentionTitle.trim(),
          attentionDetail.trim(),
          attentionOwner.trim(),
          operator.trim(),
          attentionPriority,
          decisionOptions.trim(),
          decisionImpact.trim(),
          decisionEvidence.trim(),
          decisionRecommendation.trim(),
          decisionConsequence.trim(),
        ].join("\n"),
      );
      if (attentionMode === "blocker") {
        const result = await rpc<ProjectBlockerCreateResult>(
          "projectBlocker/create",
          {
            projectId,
            phaseId: selectedPhase.phaseId,
            taskId: null,
            title: attentionTitle.trim(),
            reason: attentionDetail.trim(),
            ownerLabel: attentionOwner.trim(),
            reporterLabel: operator.trim(),
            priority: attentionPriority,
            dueAt: null,
            clientOperationId,
            requestFingerprint,
          },
        );
        setAttentionCases((current) => [result.blocker, ...current]);
      } else {
        const result = await rpc<DecisionRequestCreateResult>(
          "decisionRequest/create",
          {
            projectId,
            phaseId: selectedPhase.phaseId,
            deliverableId: selectedPhase.requiredDeliverableId,
            title: attentionTitle.trim(),
            question: attentionDetail.trim(),
            options: decisionOptions.trim(),
            impact: decisionImpact.trim(),
            evidence: decisionEvidence.trim(),
            recommendation: decisionRecommendation.trim(),
            consequence: decisionConsequence.trim(),
            decisionOwnerLabel: attentionOwner.trim(),
            requesterLabel: operator.trim(),
            priority: attentionPriority,
            dueAt: null,
            clientOperationId,
            requestFingerprint,
          },
        );
        setAttentionCases((current) => [result.decisionRequest, ...current]);
      }
      resetAttentionForm();
      props.onMessage(
        attentionMode === "blocker"
          ? "项目阻塞已记录，并同步进入个人工作台。"
          : "决策请求已发起，并同步进入个人工作台。",
      );
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const updateOperator = (value: string) => {
    setOperator(value);
  };

  const requestPhaseTransition = (phase: ProjectPhaseItem, target: string) => {
    if (!phaseCommentRequired(phase, target)) {
      void transitionPhase(phase, target, "");
      return;
    }
    const terminating = target === "terminated";
    setPendingTransition({
      title:
        target === "completed"
          ? "验收并完成阶段"
          : terminating
            ? "终止阶段"
            : "暂停阶段",
      description: `${phase.name}：${target === "completed" ? "完成后后继阶段方可启动，本次结论会写入变更历史。" : terminating ? "终止后该阶段不再推进，未完成的任务保持原状。" : "暂停后阶段可再次启动。"}`,
      confirmLabel:
        target === "completed"
          ? "确认完成"
          : terminating
            ? "确认终止"
            : "确认暂停",
      commentLabel: target === "completed" ? "验收结论" : "原因说明",
      commentPlaceholder:
        target === "completed"
          ? "说明依据哪些证据判定阶段达标"
          : "说明为什么在此刻改变阶段状态",
      danger: terminating,
      run: (comment) => transitionPhase(phase, target, comment),
    });
  };

  const requestTaskTransition = (task: BusinessTaskItem, target: string) => {
    if (!taskCommentRequired(target)) {
      void transitionTask(task, target, "");
      return;
    }
    const rework = target === "rework";
    setPendingTransition({
      title: rework ? "请求返工" : "确认任务完成",
      description: `${task.title}：${rework ? "任务将进入新的返工尝试，原验收反馈继续保留。" : "完成结论会写入变更历史并汇入个人工作台。"}`,
      confirmLabel: rework ? "确认返工" : "确认完成",
      commentLabel: rework ? "返工反馈" : "验收结论",
      commentPlaceholder: rework
        ? "说明哪些验收标准未达成"
        : "说明依据哪些证据判定任务达标",
      run: (comment) => transitionTask(task, target, comment),
    });
  };

  const requestAttentionTransition = (
    item: AttentionCaseItem,
    target: "open" | "acknowledged" | "resolved",
    decisionAction?: string,
    delegatedTo?: string,
  ) => {
    if (!attentionCommentRequired(target, decisionAction)) {
      void transitionAttentionCase(
        item,
        target,
        "",
        decisionAction,
        delegatedTo,
      );
      return;
    }
    setPendingTransition({
      title: decisionAction ? "记录决策结论" : "解决关注事项",
      description: `${item.title}：结论会写入变更历史，并同步到个人工作台。`,
      confirmLabel: "确认",
      commentLabel: decisionAction ? "决策理由" : "解决说明",
      commentPlaceholder: decisionAction
        ? "说明作出该决策的依据"
        : "说明该事项如何被解决",
      run: (comment) =>
        transitionAttentionCase(
          item,
          target,
          comment,
          decisionAction,
          delegatedTo,
        ),
    });
  };

  const transitionAttentionCase = async (
    attentionCase: AttentionCaseItem,
    targetStatus: "open" | "acknowledged" | "resolved",
    comment: string,
    decisionAction?: string,
    delegatedTo?: string,
  ) => {
    if (busy || !operator.trim()) return;
    if (attentionCommentRequired(targetStatus, decisionAction) && !comment)
      return;
    setBusy(true);
    try {
      const clientOperationId = `desktop-attention-transition-${randomUuid()}`;
      const resolutionSummary = attentionCommentRequired(
        targetStatus,
        decisionAction,
      )
        ? comment
        : "";
      const requestFingerprint = await sha256(
        [
          attentionCase.attentionCaseId,
          targetStatus,
          String(attentionCase.stateRevision),
          operator.trim(),
          resolutionSummary,
          decisionAction ?? "",
          delegatedTo ?? "",
        ].join("\n"),
      );
      const result = await rpc<AttentionCaseTransitionResult>(
        "attentionCase/transition",
        {
          attentionCaseId: attentionCase.attentionCaseId,
          targetStatus,
          expectedRevision: attentionCase.stateRevision,
          actorLabel: operator.trim(),
          resolutionSummary,
          decisionAction: decisionAction ?? null,
          delegatedTo: delegatedTo ?? null,
          clientOperationId,
          requestFingerprint,
        },
      );
      setAttentionCases((current) =>
        current.map((item) =>
          item.attentionCaseId === result.attentionCase.attentionCaseId
            ? result.attentionCase
            : item,
        ),
      );
      props.onMessage(
        targetStatus === "acknowledged"
          ? "智能体运行异常已知晓，已进入处置中。"
          : decisionAction === "delegated"
            ? "决策已转交，新的责任人已同步到个人工作台。"
            : "关注事项已解决，个人工作台投影已同步清理。 ",
      );
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const progress = useMemo(() => {
    if (!activePlan?.phases.length) return 0;
    const completed = activePlan.phases.filter(
      (phase) => phase.status === "completed",
    ).length;
    return Math.round((completed / activePlan.phases.length) * 100);
  }, [activePlan]);

  const projectArtifacts = useMemo(() => conversationFileArtifacts([...projectRuns].sort((a, b) => (a.completedAt ?? a.startedAt) - (b.completedAt ?? b.startedAt))), [projectRuns]);

  const sortedConversations = useMemo(
    () =>
      [...projectConversations].sort(
        (left, right) => right.updatedAt - left.updatedAt,
      ),
    [projectConversations],
  );

  const openFileForDeliverable = (deliverable: DeliverableItem) => {
    const acceptedVersion =
      deliverable.versions.find(
        (version) => version.versionId === deliverable.currentAcceptedVersionId,
      ) ?? deliverable.versions[0];
    const primaryBinding =
      acceptedVersion?.bindings.find(
        (binding) => binding.bindingRole === "primary",
      ) ?? acceptedVersion?.bindings[0];
    if (!primaryBinding) {
      props.onMessage("该交付物还没有绑定文件。", "error");
      return;
    }
    props.onOpenFile(primaryBinding.localFileId, primaryBinding.workspaceId);
  };

  // 返回按钮已经从独立的面包屑条挪进这一页，这三个分支同样绕不过去：
  // 少了它，工作区没选中或项目对不上时页面里就没有任何退出口。
  const backToProjects = (
    <button
      aria-label="返回项目列表"
      className="project-back"
      onClick={props.onExitProject}
      title="返回项目列表"
      type="button"
    >
      <BackArrowIcon />
    </button>
  );
  if (loading)
    return (
      <div className="center-state">正在读取项目推进事实…{backToProjects}</div>
    );
  if (loadError)
    return (
      <LoadFailure
        error={loadError}
        what="项目推进"
        onRetry={() => setReloadToken((value) => value + 1)}
      />
    );
  if (!selectedProject)
    return (
      <section className="project-unlinked-state">
        <RouteIcon />
        <p className="eyebrow">Project association</p>
        <h2>当前工作区尚未关联项目</h2>
        <p>
          项目需要由真实业务来源建立并关联到该工作区。文件仍可从一级“文件”入口浏览；这里不会根据目录名称推断项目，也不提供本地创建入口。
        </p>
        {backToProjects}
      </section>
    );

  const openTaskDetail = (task: BusinessTaskItem) => {
    const phase = plans.flatMap((plan) => plan.phases).find((item) => item.phaseId === task.phaseId);
    setActiveTaskDetail({
      workItemId: `business_task:${task.businessTaskId}`,
      sourceType: "business_task", sourceId: task.businessTaskId,
      projectId: task.projectId, projectName: selectedProject.displayName,
      primaryWorkspaceId: selectedProject.primaryWorkspaceId,
      phaseId: task.phaseId, phaseName: phase?.name ?? null,
      taskId: task.businessTaskId, deliverableId: null, deliverableTitle: null,
      attentionCaseId: null, title: task.title, actionType: "view",
      focusGroup: ["candidate", "rework"].includes(task.status) ? "todo" : task.status,
      responsibilityType: task.assigneeType, responsibilityLabel: task.assigneeLabel,
      priority: task.priority, dueAt: task.dueAt,
      sourceRevision: task.stateRevision, updatedAt: task.updatedAt,
    });
  };

  return (
    <>
      {!props.workspace ? (
        <div className="project-unlinked-notice">
          当前项目暂无文件夹；项目计划可以继续维护，文件、交付物版本和智能体操作需添加文件夹后使用。
        </div>
      ) : null}
      <section className="project-summary-header">
        {backToProjects}
        <h2>{selectedProject.displayName}</h2>
      </section>
      <nav aria-label="项目内容" className="project-view-tabs">
        <button
          aria-current={projectView === "overview" ? "page" : undefined}
          onClick={() => setProjectView("overview")}
          type="button"
        >
          概览
        </button>
        <button
          aria-current={projectView === "tasks" ? "page" : undefined}
          onClick={() => setProjectView("tasks")}
          type="button"
        >
          任务
        </button>
        <button
          aria-current={projectView === "phases" ? "page" : undefined}
          onClick={() => {
            setProjectView("phases");
            if (!visiblePlan) setShowPlanForm(true);
          }}
          type="button"
        >
          {visiblePlan ? "阶段" : "设置阶段"}
        </button>
      </nav>
      {projectView === "overview" ? (
        <ProjectOverview
          artifacts={projectArtifacts}
          attentionCases={attentionCases}
          conversations={sortedConversations}
          deliverables={deliverables}
          hasPlan={Boolean(visiblePlan)}
          tasks={projectTasks}
          onOpenConversation={props.onOpenConversation}
          onOpenTasks={(status = "all") => { setTaskStatusFilter(status); setProjectView("tasks"); }}
          onPreview={setArtifactPreview}
          onOpenTask={openTaskDetail}
          onSetPhases={() => {
            setProjectView("phases");
            setShowPlanForm(true);
          }}
        />
      ) : projectView === "tasks" ? (
        <section className="project-tasks-view">
          <header>
            <div>
              <h2>项目任务</h2>
            </div>
            <button
              disabled={!props.workspace}
              onClick={() => startTask(null)}
              type="button"
            >
              <PlusIcon />
              新建任务
            </button>
          </header>
          <ProjectTaskLedger
            status={taskStatusFilter}
            onStatus={setTaskStatusFilter}
            phases={visiblePlan?.phases ?? []}
            tasks={projectTasks}
            onOpenTask={openTaskDetail}
          />
        </section>
      ) : (
        <section className="project-progress-layout">
          <aside className="phase-rail">
            <header>
              <div>
                <h2>阶段计划</h2>
              </div>
              <button
                aria-label="新建阶段计划版本"
                disabled={Boolean(draftPlan)}
                onClick={() => setShowPlanForm(true)}
                title={draftPlan ? "已有待激活草案" : "新建阶段计划版本"}
                type="button"
              >
                <PlusIcon />
              </button>
            </header>
            {activePlan ? (
              <div className="plan-progress">
                <div>
                  <span>计划 V{activePlan.versionNo}</span>
                  <strong>{progress}%</strong>
                </div>
                <i>
                  <b style={{ width: `${progress}%` }} />
                </i>
                <p>{activePlan.summary}</p>
              </div>
            ) : null}
            {draftPlan ? (
              <div className="draft-plan-banner">
                <span>草案 V{draftPlan.versionNo}</span>
                <strong>{draftPlan.summary}</strong>
                <button
                  disabled={
                    busy ||
                    Boolean(
                      activePlan &&
                      activePlan.phases.some(
                        (phase) =>
                          !["completed", "terminated"].includes(phase.status),
                      ),
                    )
                  }
                  onClick={() => void activatePlan(draftPlan)}
                  type="button"
                >
                  激活计划
                </button>
              </div>
            ) : null}
            {visiblePlan ? (
              <ol className="phase-timeline">
                {visiblePlan.phases.map((phase, index) => (
                  <li data-status={phase.status} key={phase.phaseId}>
                    <button
                      className={
                        selectedPhase?.phaseId === phase.phaseId
                          ? "selected"
                          : ""
                      }
                      onClick={() => setSelectedPhaseId(phase.phaseId)}
                      type="button"
                    >
                      <span>
                        {phase.status === "completed" ? (
                          <CheckIcon />
                        ) : (
                          index + 1
                        )}
                      </span>
                      <div>
                        <small>
                          {phaseStatusLabel[phase.status] ?? phase.status}
                        </small>
                        <strong>{phase.name}</strong>
                        <p>{phase.goal}</p>
                      </div>
                    </button>
                  </li>
                ))}
              </ol>
            ) : (
              <div className="phase-rail-empty">
                <strong>尚无阶段计划</strong>
                <span>需要分段管理时再设置</span>
                <button onClick={() => setShowPlanForm(true)} type="button">
                  设置阶段
                </button>
              </div>
            )}
          </aside>

          <main className="phase-workspace">
            {showPlanForm ? (
              <PlanComposer
                busy={busy}
                deliverables={deliverables}
                drafts={phaseDrafts}
                summary={planSummary}
                onSummary={setPlanSummary}
                onDraft={updatePhaseDraft}
                onAdd={addPhaseDraft}
                onRemove={removePhaseDraft}
                onCancel={() => setShowPlanForm(false)}
                onSubmit={() => void createPlan()}
              />
            ) : selectedPhase ? (
              <>
                <header className="phase-header">
                  <div>
                    <span
                      className="phase-status"
                      data-status={selectedPhase.status}
                    >
                      {phaseStatusLabel[selectedPhase.status] ??
                        selectedPhase.status}
                    </span>
                    <h2>{selectedPhase.name}</h2>
                    <p>{selectedPhase.goal}</p>
                  </div>
                  <div className="phase-header-actions">
                    <button
                      disabled={!props.workspace}
                      onClick={() => {
                        if (props.workspace)
                          props.onOpenAi({
                            title: `${selectedProject.displayName} · ${selectedPhase.name} 编排`,
                            conversationType: "project",
                            mode: "discuss",
                            context: progressAiContext(
                              props.workspace,
                              selectedProject,
                              selectedPhase,
                              null,
                            ),
                          });
                      }}
                      title={
                        props.workspace
                          ? "使用智能体编排阶段"
                          : "添加文件夹后可使用智能体"
                      }
                      type="button"
                    >
                      <AiIcon />
                      AI 编排
                    </button>
                    {selectedPhase.status === "planned" && activePlan ? (
                      <button
                        className="primary"
                        disabled={busy || !operator.trim()}
                        onClick={() =>
                          requestPhaseTransition(selectedPhase, "in_progress")
                        }
                        type="button"
                      >
                        启动阶段
                      </button>
                    ) : null}
                    {selectedPhase.status === "in_progress" ? (
                      <>
                        <button
                          className="primary"
                          disabled={busy || !operator.trim()}
                          onClick={() =>
                            requestPhaseTransition(selectedPhase, "review")
                          }
                          type="button"
                        >
                          提交阶段评审
                        </button>
                      </>
                    ) : null}
                    {selectedPhase.status === "paused" ? (
                      <button
                        className="primary"
                        disabled={busy || !operator.trim()}
                        onClick={() =>
                          requestPhaseTransition(selectedPhase, "in_progress")
                        }
                        type="button"
                      >
                        恢复阶段
                      </button>
                    ) : null}
                    {selectedPhase.status === "review" ? (
                      <>
                        <button
                          disabled={busy || !operator.trim()}
                          onClick={() =>
                            requestPhaseTransition(selectedPhase, "in_progress")
                          }
                          type="button"
                        >
                          返工推进
                        </button>
                        <button
                          className="primary"
                          disabled={busy || !operator.trim()}
                          onClick={() =>
                            requestPhaseTransition(selectedPhase, "completed")
                          }
                          type="button"
                        >
                          确认阶段完成
                        </button>
                      </>
                    ) : null}
                    {!["completed", "terminated"].includes(
                      selectedPhase.status,
                    ) ? (
                      <div className="phase-more" ref={phaseMoreRef}>
                        <button
                          aria-expanded={phaseMoreOpen}
                          aria-label="更多阶段操作"
                          onClick={() => setPhaseMoreOpen((open) => !open)}
                          title="更多阶段操作"
                          type="button"
                        >
                          <MoreIcon />
                        </button>
                        {phaseMoreOpen ? (
                          <div className="phase-more-menu" role="menu">
                            {selectedPhase.status === "in_progress" ? (
                              <button
                                disabled={busy || !operator.trim()}
                                onClick={() => {
                                  setPhaseMoreOpen(false);
                                  requestPhaseTransition(
                                    selectedPhase,
                                    "paused",
                                  );
                                }}
                                role="menuitem"
                                type="button"
                              >
                                暂停阶段
                              </button>
                            ) : null}
                            <button
                              className="danger"
                              disabled={busy || !operator.trim()}
                              onClick={() => {
                                setPhaseMoreOpen(false);
                                requestPhaseTransition(
                                  selectedPhase,
                                  "terminated",
                                );
                              }}
                              role="menuitem"
                              type="button"
                            >
                              终止阶段
                            </button>
                          </div>
                        ) : null}
                      </div>
                    ) : null}
                  </div>
                </header>
                <section className="task-board-header">
                  <div>
                    <h3>阶段任务</h3>
                  </div>
                  {selectedPhase.status === "in_progress" ? (
                    <button
                      disabled={!props.workspace}
                      onClick={() => startTask(selectedPhase)}
                      type="button"
                    >
                      <PlusIcon />
                      新建任务
                    </button>
                  ) : null}
                </section>
                <div className="task-board">
                  {selectedPhase.tasks
                    .filter(
                      (task) =>
                        !["completed", "terminated"].includes(task.status),
                    )
                    .map((task) => (
                      <TaskCard
                        availablePhases={(visiblePlan?.phases ?? []).filter(
                          (phase) =>
                            phase.phaseId !== task.phaseId &&
                            ["planned", "in_progress", "paused"].includes(
                              phase.status,
                            ),
                        )}
                        busy={busy}
                        key={task.businessTaskId}
                        task={task}
                        operatorReady={Boolean(operator.trim())}
                        onMove={moveTask}
                        onTransition={requestTaskTransition}
                        onOpenAi={() => {
                          if (props.workspace)
                            props.onOpenAi({
                              title: `执行 ${task.title}`,
                              conversationType: "task",
                              mode: "discuss",
                              context: progressAiContext(
                                props.workspace,
                                selectedProject,
                                selectedPhase,
                                task,
                              ),
                            });
                          else
                            props.onMessage(
                              "请先在项目编辑中添加文件夹，再使用智能体执行任务。",
                              "error",
                            );
                        }}
                      />
                    ))}
                  {!selectedPhase.tasks.some(
                    (task) =>
                      !["completed", "terminated"].includes(task.status),
                  ) ? (
                    <div className="task-empty">
                      <TaskIcon />
                      <strong>当前没有任务</strong>
                      <span>
                        {selectedPhase.status === "in_progress"
                          ? "可以新建任务"
                          : "启动阶段后再创建任务"}
                      </span>
                    </div>
                  ) : null}
                </div>
                <AttentionPanel
                  attentionCases={attentionCases}
                  busy={busy}
                  detail={attentionDetail}
                  decisionConsequence={decisionConsequence}
                  decisionEvidence={decisionEvidence}
                  decisionImpact={decisionImpact}
                  decisionOptions={decisionOptions}
                  decisionRecommendation={decisionRecommendation}
                  mode={attentionMode}
                  operatorReady={Boolean(operator.trim())}
                  owner={attentionOwner}
                  phaseName={(phaseId) =>
                    visiblePlan?.phases.find(
                      (phase) => phase.phaseId === phaseId,
                    )?.name ?? "项目级"
                  }
                  priority={attentionPriority}
                  title={attentionTitle}
                  onCancel={resetAttentionForm}
                  onCreate={() => void createAttentionCase()}
                  onDetail={setAttentionDetail}
                  onDecisionConsequence={setDecisionConsequence}
                  onDecisionEvidence={setDecisionEvidence}
                  onDecisionImpact={setDecisionImpact}
                  onDecisionOptions={setDecisionOptions}
                  onDecisionRecommendation={setDecisionRecommendation}
                  onMode={setAttentionMode}
                  onOwner={setAttentionOwner}
                  onPriority={setAttentionPriority}
                  onTitle={setAttentionTitle}
                  onTransition={requestAttentionTransition}
                />
              </>
            ) : (
              <div className="center-state">创建或选择一个阶段计划。</div>
            )}
          </main>

          <aside className="phase-inspector">
            <h3>操作人</h3>
            <label>
              署名
              <input
                value={operator}
                readOnly={Boolean(props.operatorName)}
                onChange={(event) => updateOperator(event.target.value)}
                placeholder="你的名字"
              />
            </label>
            <p className="phase-operator-hint">
              操作将以当前署名记入历史。验收与决策仍需确认。
            </p>
            {selectedPhase ? (
              <>
                <hr />
                <dl>
                  <div>
                    <dt>阶段状态</dt>
                    <dd>
                      {phaseStatusLabel[selectedPhase.status] ??
                        selectedPhase.status}
                    </dd>
                  </div>
                  <div>
                    <dt>修订</dt>
                    <dd>{selectedPhase.stateRevision}</dd>
                  </div>
                  <div>
                    <dt>任务</dt>
                    <dd>{selectedPhase.tasks.length}</dd>
                  </div>
                  <div>
                    <dt>验收标准</dt>
                    <dd>{selectedPhase.acceptanceCriteria}</dd>
                  </div>
                </dl>
                <hr />
                <h3>阶段交付物</h3>
                <div className="phase-deliverable">
                  <DeliverableIcon />
                  <strong>
                    {selectedPhase.requiredDeliverableTitle ?? "未绑定"}
                  </strong>
                  <small>
                    {selectedPhase.requiredDeliverableId
                      ? selectedPhase.requiredDeliverableVersionId
                        ? "已有正式验收版本"
                        : "尚无正式验收版本"
                      : "当前阶段不要求正式交付物"}
                  </small>
                  <div className="phase-deliverable-actions">
                    <button
                      onClick={() => setShowDeliverables(true)}
                      type="button"
                    >
                      管理与验收
                    </button>
                    {selectedPhase.requiredDeliverableVersionId ? (
                      <button
                        onClick={() => {
                          const deliverable = deliverables.find(
                            (item) =>
                              item.deliverableId ===
                              selectedPhase.requiredDeliverableId,
                          );
                          if (deliverable) openFileForDeliverable(deliverable);
                        }}
                        type="button"
                      >
                        查看文件
                      </button>
                    ) : null}
                  </div>
                </div>
              </>
            ) : (
              <>
                <hr />
                <h3>项目交付物</h3>
                <button
                  className="manage-deliverables-button"
                  onClick={() => setShowDeliverables(true)}
                  type="button"
                >
                  管理 {deliverables.length} 项交付物
                </button>
              </>
            )}
          </aside>
        </section>
      )}
      {artifactPreview ? <ImageArtifactLightbox activeIndex={0} artifacts={[artifactPreview.artifact]} onChange={() => {}} previews={{ [artifactPreview.artifact.artifactId]: artifactPreview.url }} onClose={() => setArtifactPreview(null)}/> : null}
      {activeTaskDetail ? <TaskDetailDrawer
        item={activeTaskDetail}
        onClose={() => setActiveTaskDetail(null)}
        onMessage={props.onMessage}
        onOpenConversation={(id) => { setActiveTaskDetail(null); props.onOpenConversation(id); }}
        onOpenRun={(id) => { setActiveTaskDetail(null); props.onOpenRun(id); }}
      /> : null}
      {pendingTransition ? (
        <ConfirmDialog
          busy={busy}
          commentLabel={pendingTransition.commentLabel}
          commentPlaceholder={pendingTransition.commentPlaceholder}
          confirmLabel={pendingTransition.confirmLabel}
          danger={pendingTransition.danger}
          description={pendingTransition.description}
          title={pendingTransition.title}
          onCancel={() => setPendingTransition(null)}
          onConfirm={(comment) => {
            void pendingTransition
              .run(comment)
              .then(() => setPendingTransition(null));
          }}
        />
      ) : null}
      {showDeliverables ? (
        <DeliverableManager
          deliverables={deliverables}
          files={props.files}
          hostPlatform={props.hostPlatform}
          onChanged={refresh}
          onClose={() => setShowDeliverables(false)}
          onMessage={props.onMessage}
          project={selectedProject}
          workspace={props.workspace}
        />
      ) : null}
    </>
  );
}

function ProjectOverview(props: {
  artifacts: ReturnType<typeof conversationFileArtifacts>;
  onPreview: (preview: TaskArtifactPreview) => void;
  attentionCases: AttentionCaseItem[];
  conversations: AiConversation[];
  deliverables: DeliverableItem[];
  hasPlan: boolean;
  tasks: BusinessTaskItem[];
  onOpenConversation: (conversationId: string) => void;
  onOpenTasks: (status?: string) => void;
  onOpenTask: (task: BusinessTaskItem) => void;
  onSetPhases: () => void;
}) {
  const openCases = props.attentionCases
    .filter((item) => item.status !== "resolved" && item.caseType !== "agent_run_incident")
    .sort((left, right) => right.updatedAt - left.updatedAt);
  const recentTasks = [...props.tasks]
    .sort((left, right) => right.updatedAt - left.updatedAt)
    .slice(0, 5);
  return (
    <main className="project-overview">
      <section className="project-status-overview">
        <header>
          <div>
            <h2>任务状态</h2>
          </div>
          <button onClick={() => props.onOpenTasks()} type="button">
            查看全部
          </button>
        </header>
        <div className="project-status-grid">
          {projectTaskGroups.map((group) => {
            const count = props.tasks.filter((task) =>
              group.statuses.includes(task.status as never),
            ).length;
            return (
              <button
                data-status={group.key}
                key={group.key}
                onClick={() => props.onOpenTasks(group.key)}
                type="button"
              >
                <span>{group.label}</span>
                <strong>{count}</strong>
              </button>
            );
          })}
        </div>
      </section>

      {openCases.length ? (
        <section className="project-overview-attention">
          <header>
            <h2>需介入事项</h2>
            <span>{openCases.length}</span>
          </header>
          <div>
            {openCases.slice(0, 5).map((item) => (
              <article key={item.attentionCaseId}>
                <span>
                  {item.caseType === "decision_request"
                    ? "决策"
                    : item.caseType === "project_blocker"
                      ? "阻塞"
                      : "异常"}
                </span>
                <strong>{item.title}</strong>
                <small>{item.responsibilityLabel}</small>
              </article>
            ))}
          </div>
        </section>
      ) : null}

      <section className="project-overview-streams">
        <article>
          <header>
            <h2>最近任务</h2>
            <span>{props.tasks.length}</span>
          </header>
          {recentTasks.length ? (
            <div>
              {recentTasks.map((task) => (
                <button
                  key={task.businessTaskId}
                  onClick={() => props.onOpenTask(task)}
                  type="button"
                >
                  <span>{taskStatusLabel[task.status] ?? task.status}</span>
                  <strong>{task.title}</strong>
                  <small>{formatBriefTime(task.updatedAt)}</small>
                </button>
              ))}
            </div>
          ) : (
            <p>暂无任务</p>
          )}
        </article>
        <article>
          <header>
            <h2>最近对话</h2>
            <span>{props.conversations.length}</span>
          </header>
          {props.conversations.length ? (
            <div>
              {props.conversations.slice(0, 5).map((conversation) => (
                <button
                  key={conversation.conversationId}
                  onClick={() =>
                    props.onOpenConversation(conversation.conversationId)
                  }
                  type="button"
                >
                  <strong>{conversation.title}</strong>
                  <small>{formatBriefTime(conversation.updatedAt)}</small>
                </button>
              ))}
            </div>
          ) : (
            <p>暂无项目对话</p>
          )}
        </article>
        <article>
          <header>
            <h2>最近产物</h2>
            <span>{props.artifacts.length}</span>
          </header>
          {props.artifacts.length ? (
            <TaskArtifactList entries={props.artifacts} onPreview={props.onPreview}/>

          ) : (
            <p>暂无登记产物</p>
          )}
        </article>
      </section>

      {props.hasPlan || props.deliverables.length ? (
        <section className="project-overview-structure">
          <h2>管理结构</h2>
          <div>
            {props.hasPlan ? <span>已设置阶段</span> : null}
            {props.deliverables.length ? (
              <span>正式交付物 {props.deliverables.length}</span>
            ) : null}
          </div>
        </section>
      ) : (
        <button
          className="project-overview-phase-action"
          onClick={props.onSetPhases}
          type="button"
        >
          需要分段管理时设置阶段
        </button>
      )}
    </main>
  );
}

function ProjectTaskLedger(props: {
  phases: ProjectPhaseItem[];
  tasks: BusinessTaskItem[];
  status: string;
  onStatus: (status: string) => void;
  onOpenTask: (task: BusinessTaskItem) => void;
}) {
  const [query, setQuery] = useState("");
  const [owner, setOwner] = useState("all");
  const [phase, setPhase] = useState("all");
  const [priority, setPriority] = useState("all");
  const [board, setBoard] = useState(false);
  const phaseNames = new Map(props.phases.map((phase) => [phase.phaseId, phase.name]));
  const owners = [...new Set(props.tasks.map((task) => task.assigneeLabel))];
  const normalized = query.trim().toLocaleLowerCase();
  const filtered = props.tasks.filter((task) =>
    (!normalized || [task.title, task.objective, task.assigneeLabel].join(" ").toLocaleLowerCase().includes(normalized)) &&
    (owner === "all" || task.assigneeLabel === owner) &&
    (phase === "all" || (task.phaseId ?? "none") === phase) &&
    (priority === "all" || task.priority === priority));
  return <section className="project-task-ledger">
    <div className="project-task-filters">
      <SearchField ariaLabel="搜索项目任务" placeholder="搜索任务" value={query} onChange={setQuery}/>
      <Select ariaLabel="任务负责人" value={owner} onChange={setOwner} options={[{ value: "all", label: "全部负责人" }, ...owners.map((value) => ({ value, label: value }))]}/>
      <Select ariaLabel="任务阶段" value={phase} onChange={setPhase} options={[{ value: "all", label: "全部阶段" }, { value: "none", label: "未绑定阶段" }, ...props.phases.map((item) => ({ value: item.phaseId, label: item.name }))]}/>
      <Select ariaLabel="任务优先级" value={priority} onChange={setPriority} options={[{ value: "all", label: "全部优先级" }, ...Object.entries(taskPriorityLabel).map(([value, label]) => ({ value, label }))]}/>
      <button aria-pressed={!board} onClick={() => setBoard(false)} type="button">列表</button><button aria-pressed={board} onClick={() => setBoard(true)} type="button">看板</button>
    </div>
    <nav className="project-task-status-filter" aria-label="任务状态筛选">
      <button aria-pressed={props.status === "all"} onClick={() => props.onStatus("all")} type="button">全部 {filtered.length}</button>
      {projectTaskGroups.map((group) => <button aria-pressed={props.status === group.key} key={group.key} onClick={() => props.onStatus(group.key)} type="button">{group.label} {filtered.filter((task) => group.statuses.includes(task.status as never)).length}</button>)}
    </nav>
    {!filtered.some((task) => projectTaskGroups.some((group) => (props.status === "all" || props.status === group.key) && group.statuses.includes(task.status as never))) ? <p className="project-task-empty" role="status">没有匹配的任务</p> : null}
    <div className="project-task-columns" data-view={board ? "board" : "list"}>
      {projectTaskGroups.filter((group) => props.status === "all" || props.status === group.key).map((group) => {
        const tasks = filtered.filter((task) => group.statuses.includes(task.status as never));
        return <section data-status={group.key} data-empty={!tasks.length} key={group.key}>
          <header><h3>{group.label}</h3><span>{tasks.length}</span></header>
          <div>{tasks.map((task) => <article key={task.businessTaskId}>
            <button className="project-task-open" onClick={() => props.onOpenTask(task)} type="button">{task.title}</button>
            <span>{taskStatusLabel[task.status]}</span>
            <small>{task.phaseId ? phaseNames.get(task.phaseId) ?? "阶段不可用" : "未绑定阶段"}</small>
            <span>{task.assigneeLabel}</span><small>{taskPriorityLabel[task.priority] ?? task.priority}</small>
            <time>{formatBriefTime(task.updatedAt)}</time>
          </article>)}{!tasks.length ? <p>暂无任务</p> : null}</div>
        </section>;
      })}
    </div>
  </section>;
}

function formatBriefTime(value: number) {
  if (!value) return "";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

function AttentionPanel(props: {
  attentionCases: AttentionCaseItem[];
  busy: boolean;
  detail: string;
  decisionOptions: string;
  decisionImpact: string;
  decisionEvidence: string;
  decisionRecommendation: string;
  decisionConsequence: string;
  mode: "blocker" | "decision" | null;
  operatorReady: boolean;
  owner: string;
  phaseName: (phaseId: string | null) => string;
  priority: string;
  title: string;
  onCancel: () => void;
  onCreate: () => void;
  onDetail: (value: string) => void;
  onDecisionOptions: (value: string) => void;
  onDecisionImpact: (value: string) => void;
  onDecisionEvidence: (value: string) => void;
  onDecisionRecommendation: (value: string) => void;
  onDecisionConsequence: (value: string) => void;
  onMode: (mode: "blocker" | "decision") => void;
  onOwner: (value: string) => void;
  onPriority: (value: string) => void;
  onTitle: (value: string) => void;
  onTransition: (
    item: AttentionCaseItem,
    status: "open" | "acknowledged" | "resolved",
    decisionAction?: string,
    delegatedTo?: string,
  ) => void;
}) {
  const openCases = props.attentionCases.filter(
    (item) => item.status !== "resolved" && item.caseType !== "agent_run_incident",
  );
  const resolvedCases = props.attentionCases.filter(
    (item) => item.status === "resolved",
  );
  return (
    <section className="attention-panel">
      <header>
        <div>
          <h3>项目关注事项</h3>
          <p>阻塞、待决策和智能体异常都是项目事实，并会投影到个人工作台。</p>
        </div>
        <div>
          <button onClick={() => props.onMode("blocker")} type="button">
            <BlockerIcon />
            记录阻塞
          </button>
          <button onClick={() => props.onMode("decision")} type="button">
            <DecisionIcon />
            发起决策
          </button>
        </div>
      </header>
      {props.mode ? (
        <form
          className="attention-composer"
          onSubmit={(event) => {
            event.preventDefault();
            props.onCreate();
          }}
        >
          <div>
            <strong>
              {props.mode === "blocker" ? "记录项目阻塞" : "发起结构化决策请求"}
            </strong>
            <button onClick={props.onCancel} type="button">
              取消
            </button>
          </div>
          <label>
            标题
            <input
              autoFocus
              onChange={(event) => props.onTitle(event.target.value)}
              placeholder={
                props.mode === "blocker"
                  ? "什么正在阻碍项目推进？"
                  : "需要做出什么决策？"
              }
              value={props.title}
            />
          </label>
          <label>
            {props.mode === "blocker" ? "阻塞原因" : "决策问题"}
            <textarea
              onChange={(event) => props.onDetail(event.target.value)}
              rows={2}
              value={props.detail}
            />
          </label>
          {props.mode === "decision" ? (
            <div className="decision-card-fields">
              <label>
                可选方案
                <textarea
                  onChange={(event) =>
                    props.onDecisionOptions(event.target.value)
                  }
                  rows={3}
                  placeholder="列出至少两个可执行方案"
                  value={props.decisionOptions}
                />
              </label>
              <label>
                影响范围
                <textarea
                  onChange={(event) =>
                    props.onDecisionImpact(event.target.value)
                  }
                  rows={3}
                  placeholder="对阶段、任务、交付物和时间的影响"
                  value={props.decisionImpact}
                />
              </label>
              <label>
                证据
                <textarea
                  onChange={(event) =>
                    props.onDecisionEvidence(event.target.value)
                  }
                  rows={3}
                  placeholder="支持判断的成果版本、数据或风险事实"
                  value={props.decisionEvidence}
                />
              </label>
              <label>
                建议与理由
                <textarea
                  onChange={(event) =>
                    props.onDecisionRecommendation(event.target.value)
                  }
                  rows={3}
                  value={props.decisionRecommendation}
                />
              </label>
              <label>
                不决定的后果
                <textarea
                  onChange={(event) =>
                    props.onDecisionConsequence(event.target.value)
                  }
                  rows={3}
                  value={props.decisionConsequence}
                />
              </label>
            </div>
          ) : null}
          <div>
            <label>
              责任人
              <input
                onChange={(event) => props.onOwner(event.target.value)}
                placeholder="明确到个人"
                value={props.owner}
              />
            </label>
            <label>
              优先级
              <Select
                options={[
                  { value: "medium", label: "中" },
                  { value: "high", label: "高" },
                  { value: "critical", label: "紧急" },
                  { value: "low", label: "低" },
                ]}
                value={props.priority}
                onChange={props.onPriority}
              />
            </label>
          </div>
          <button
            className="primary"
            disabled={
              props.busy ||
              !props.operatorReady ||
              !props.title.trim() ||
              !props.detail.trim() ||
              !props.owner.trim() ||
              (props.mode === "decision" &&
                [
                  props.decisionOptions,
                  props.decisionImpact,
                  props.decisionEvidence,
                  props.decisionRecommendation,
                  props.decisionConsequence,
                ].some((value) => !value.trim()))
            }
            type="submit"
          >
            {props.mode === "blocker" ? "确认记录" : "确认发起"}
          </button>
        </form>
      ) : null}
      <div className="attention-list">
        {openCases.map((item) => (
          <AttentionCard
            item={item}
            key={item.attentionCaseId}
            busy={props.busy}
            operatorReady={props.operatorReady}
            phaseName={props.phaseName(item.phaseId)}
            onTransition={props.onTransition}
          />
        ))}
        {openCases.length === 0 ? (
          <div className="attention-empty">
            <AttentionIcon />
            <span>当前项目没有未解决的关注事项</span>
          </div>
        ) : null}
      </div>
      {resolvedCases.length ? (
        <details className="attention-history">
          <summary>已解决 {resolvedCases.length}</summary>
          {resolvedCases.map((item) => (
            <AttentionCard
              item={item}
              key={item.attentionCaseId}
              busy={props.busy}
              operatorReady={props.operatorReady}
              phaseName={props.phaseName(item.phaseId)}
              onTransition={props.onTransition}
            />
          ))}
        </details>
      ) : null}
    </section>
  );
}

function AttentionCard(props: {
  item: AttentionCaseItem;
  busy: boolean;
  operatorReady: boolean;
  phaseName: string;
  onTransition: (
    item: AttentionCaseItem,
    status: "open" | "acknowledged" | "resolved",
    decisionAction?: string,
    delegatedTo?: string,
  ) => void;
}) {
  const typeLabel =
    props.item.caseType === "project_blocker"
      ? "项目阻塞"
      : props.item.caseType === "decision_request"
        ? "待决策"
        : "智能体异常";
  const [decisionAction, setDecisionAction] = useState("approved");
  const [delegatedTo, setDelegatedTo] = useState("");
  const decisionReady =
    props.item.caseType !== "decision_request" ||
    Boolean(
      decisionAction && (decisionAction !== "delegated" || delegatedTo.trim()),
    );
  const submit = () =>
    props.onTransition(
      props.item,
      decisionAction === "delegated" ? "open" : "resolved",
      props.item.caseType === "decision_request" ? decisionAction : undefined,
      decisionAction === "delegated" ? delegatedTo.trim() : undefined,
    );
  return (
    <article
      className="attention-card"
      data-status={props.item.status}
      data-type={props.item.caseType}
    >
      <header>
        <span>
          {props.item.caseType === "project_blocker" ? (
            <BlockerIcon />
          ) : props.item.caseType === "decision_request" ? (
            <DecisionIcon />
          ) : (
            <AgentIcon />
          )}
          {typeLabel}
        </span>
        <small>
          {props.phaseName} ·{" "}
          {props.item.status === "acknowledged"
            ? "处置中"
            : props.item.status === "resolved"
              ? "已解决"
              : "待处理"}
        </small>
      </header>
      <strong>{props.item.title}</strong>
      <p>{props.item.detail}</p>
      {props.item.caseType === "decision_request" ? (
        <>
          <dl className="decision-card-detail">
            <div>
              <dt>可选方案</dt>
              <dd>{props.item.decisionOptions}</dd>
            </div>
            <div>
              <dt>影响</dt>
              <dd>{props.item.decisionImpact}</dd>
            </div>
            <div>
              <dt>证据</dt>
              <dd>{props.item.decisionEvidence}</dd>
            </div>
            <div>
              <dt>建议</dt>
              <dd>{props.item.decisionRecommendation}</dd>
            </div>
            <div>
              <dt>不决定的后果</dt>
              <dd>{props.item.decisionConsequence}</dd>
            </div>
          </dl>
          {props.item.status !== "resolved" ? (
            <div className="attention-decision-action">
              <Select
                ariaLabel="决策动作"
                options={[
                  { value: "approved", label: "通过" },
                  { value: "change_requested", label: "请求修改" },
                  { value: "rejected", label: "拒绝" },
                  { value: "delegated", label: "转交" },
                ]}
                value={decisionAction}
                onChange={setDecisionAction}
              />
              {decisionAction === "delegated" ? (
                <input
                  aria-label="转交给"
                  onChange={(event) => setDelegatedTo(event.target.value)}
                  placeholder="新的决策负责人"
                  value={delegatedTo}
                />
              ) : null}
            </div>
          ) : null}
        </>
      ) : null}
      {props.item.agentLabel ? (
        <p className="attention-agent">
          {props.item.agentLabel} · 运行 {props.item.agentRunId}
        </p>
      ) : null}
      <footer>
        <span>
          {props.item.responsibilityLabel} ·{" "}
          {props.item.priority === "critical"
            ? "紧急"
            : props.item.priority === "high"
              ? "高优先"
              : props.item.priority === "low"
                ? "低优先"
                : "中优先"}
        </span>
        <div>
          {props.item.caseType === "agent_run_incident" &&
          props.item.status === "open" ? (
            <button
              disabled={props.busy || !props.operatorReady}
              onClick={() => props.onTransition(props.item, "acknowledged")}
              type="button"
            >
              已知晓
            </button>
          ) : null}
          {props.item.status !== "resolved" ? (
            <button
              className="primary"
              disabled={props.busy || !props.operatorReady || !decisionReady}
              onClick={submit}
              type="button"
            >
              {props.item.caseType === "decision_request"
                ? decisionAction === "delegated"
                  ? "确认转交"
                  : "提交决策"
                : "标记已解决"}
            </button>
          ) : null}
        </div>
      </footer>
      {props.item.decisionActionSummary ? (
        <blockquote>
          {props.item.decisionAction === "delegated"
            ? `已转交给 ${props.item.decisionDelegatedTo}：`
            : "决策结论："}
          {props.item.decisionActionSummary}
        </blockquote>
      ) : props.item.resolutionSummary ? (
        <blockquote>{props.item.resolutionSummary}</blockquote>
      ) : null}
    </article>
  );
}

function PlanComposer(props: {
  busy: boolean;
  deliverables: DeliverableItem[];
  drafts: PhaseDraft[];
  summary: string;
  onSummary: (value: string) => void;
  onDraft: (index: number, patch: Partial<PhaseDraft>) => void;
  onAdd: () => void;
  onRemove: (index: number) => void;
  onCancel: () => void;
  onSubmit: () => void;
}) {
  const valid =
    props.summary.trim() &&
    props.drafts.length &&
    props.drafts.every(
      (phase) =>
        phase.name.trim() &&
        phase.goal.trim() &&
        phase.acceptanceCriteria.trim(),
    );
  return (
    <section className="plan-composer">
      <header>
        <div>
          <h2>创建阶段计划草案</h2>
          <p>通过前置关系表达串行或并行路线；无前置阶段的节点可以同时启动。</p>
        </div>
        <button onClick={props.onCancel} type="button">
          取消
        </button>
      </header>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          props.onSubmit();
        }}
      >
        <label>
          计划说明
          <input
            autoFocus
            value={props.summary}
            onChange={(event) => props.onSummary(event.target.value)}
            placeholder="例如：完成桌面端第一轮可验收交付"
          />
        </label>
        <div className="phase-draft-list">
          {props.drafts.map((phase, index) => (
            <article key={index}>
              <header>
                <span>{index + 1}</span>
                <strong>阶段 {index + 1}</strong>
                {props.drafts.length > 1 ? (
                  <button
                    aria-label={`删除阶段 ${index + 1}`}
                    onClick={() => props.onRemove(index)}
                    type="button"
                  >
                    删除
                  </button>
                ) : null}
              </header>
              <label>
                阶段名称
                <input
                  value={phase.name}
                  onChange={(event) =>
                    props.onDraft(index, { name: event.target.value })
                  }
                  placeholder="例如：需求定义"
                />
              </label>
              <label>
                阶段目标
                <textarea
                  value={phase.goal}
                  onChange={(event) =>
                    props.onDraft(index, { goal: event.target.value })
                  }
                  rows={2}
                  placeholder="这一阶段要形成什么变化？"
                />
              </label>
              <label>
                完成标准
                <textarea
                  value={phase.acceptanceCriteria}
                  onChange={(event) =>
                    props.onDraft(index, {
                      acceptanceCriteria: event.target.value,
                    })
                  }
                  rows={2}
                  placeholder="满足什么条件才能完成阶段？"
                />
              </label>
              <label>
                关联交付物（可选）
                <Select
                  placeholder="暂不关联"
                  options={props.deliverables.map((deliverable) => ({
                    value: deliverable.deliverableId,
                    label: deliverable.title,
                  }))}
                  value={phase.requiredDeliverableId}
                  onChange={(value) =>
                    props.onDraft(index, { requiredDeliverableId: value })
                  }
                />
              </label>
              <details className="phase-dependencies">
                <summary>前置阶段</summary>
                {index === 0 ? (
                  <p>首个阶段没有前置条件。</p>
                ) : (
                  props.drafts.slice(0, index).map((candidate, predecessor) => (
                    <label key={predecessor}>
                      <input
                        checked={phase.predecessorIndexes.includes(predecessor)}
                        onChange={(event) =>
                          props.onDraft(index, {
                            predecessorIndexes: event.target.checked
                              ? [...phase.predecessorIndexes, predecessor].sort(
                                  (left, right) => left - right,
                                )
                              : phase.predecessorIndexes.filter(
                                  (value) => value !== predecessor,
                                ),
                          })
                        }
                        type="checkbox"
                      />
                      <span>
                        {candidate.name.trim() || `阶段 ${predecessor + 1}`}
                      </span>
                    </label>
                  ))
                )}
              </details>
            </article>
          ))}
        </div>
        <div className="plan-composer-actions">
          <button onClick={props.onAdd} type="button">
            <PlusIcon />
            添加阶段
          </button>
          <button
            className="primary"
            disabled={props.busy || !valid}
            type="submit"
          >
            {props.busy ? "正在保存…" : "保存草案"}
          </button>
        </div>
      </form>
    </section>
  );
}

function TaskCard(props: {
  task: BusinessTaskItem;
  availablePhases: ProjectPhaseItem[];
  busy: boolean;
  operatorReady: boolean;
  onTransition: (task: BusinessTaskItem, target: string) => void;
  onMove: (
    task: BusinessTaskItem,
    input: {
      targetPhaseId: string;
      dependencyImpact: string;
      deliverableImpact: string;
      acceptanceImpact: string;
    },
  ) => Promise<void>;
  onOpenAi: () => void;
}) {
  const target = nextTaskStatus[props.task.status];
  const [showMove, setShowMove] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [targetPhaseId, setTargetPhaseId] = useState("");
  const [dependencyImpact, setDependencyImpact] = useState("");
  const [deliverableImpact, setDeliverableImpact] = useState("");
  const [acceptanceImpact, setAcceptanceImpact] = useState("");
  const movable =
    ["candidate", "todo", "rework"].includes(props.task.status) &&
    props.availablePhases.length > 0;
  const moveReady = Boolean(
    props.operatorReady &&
    targetPhaseId &&
    dependencyImpact.trim() &&
    deliverableImpact.trim() &&
    acceptanceImpact.trim(),
  );
  const submitMove = async () => {
    if (!moveReady) return;
    await props.onMove(props.task, {
      targetPhaseId,
      dependencyImpact,
      deliverableImpact,
      acceptanceImpact,
    });
    setShowMove(false);
    setTargetPhaseId("");
    setDependencyImpact("");
    setDeliverableImpact("");
    setAcceptanceImpact("");
  };
  return (
    <article
      className="business-task-card"
      data-expanded={expanded}
      data-status={props.task.status}
    >
      <button
        aria-expanded={expanded}
        className="business-task-toggle"
        onClick={() => setExpanded((value) => !value)}
        title={expanded ? "收起任务合同" : "展开任务合同"}
        type="button"
      >
        {expanded ? "收起" : "详情"}
      </button>
      <header>
        <div>
          <span>{taskStatusLabel[props.task.status] ?? props.task.status}</span>
          <i>
            {props.task.priority === "critical"
              ? "紧急"
              : props.task.priority === "high"
                ? "高优先"
                : props.task.priority === "low"
                  ? "低优先"
                  : "中优先"}
          </i>
        </div>
        <strong>{props.task.title}</strong>
        <p>{props.task.objective}</p>
      </header>
      {expanded ? (
        <dl>
          <div>
            <dt>预期输出</dt>
            <dd>{props.task.expectedOutput}</dd>
          </div>
          <div>
            <dt>完成标准</dt>
            <dd>{props.task.completionCriteria}</dd>
          </div>
          <div>
            <dt>执行尝试</dt>
            <dd>第 {props.task.attemptNo} 次</dd>
          </div>
        </dl>
      ) : null}
      {showMove ? (
        <section className="task-move-form">
          <header>
            <strong>移动到其他阶段</strong>
            <button onClick={() => setShowMove(false)} type="button">
              取消
            </button>
          </header>
          <label>
            目标阶段
            <Select
              autoFocus
              options={[
                { value: "", label: "请选择" },
                ...props.availablePhases.map((phase) => ({
                  value: phase.phaseId,
                  label: `${phase.name} · ${phaseStatusLabel[phase.status] ?? phase.status}`,
                })),
              ]}
              value={targetPhaseId}
              onChange={setTargetPhaseId}
            />
          </label>
          <label>
            任务依赖影响
            <textarea
              onChange={(event) => setDependencyImpact(event.target.value)}
              placeholder="前置输入、后续任务或阻塞会如何变化？"
              rows={2}
              value={dependencyImpact}
            />
          </label>
          <label>
            交付物影响
            <textarea
              onChange={(event) => setDeliverableImpact(event.target.value)}
              placeholder="任务输出将归属或影响哪个交付物？"
              rows={2}
              value={deliverableImpact}
            />
          </label>
          <label>
            阶段验收影响
            <textarea
              onChange={(event) => setAcceptanceImpact(event.target.value)}
              placeholder="原阶段与目标阶段的验收口径如何调整？"
              rows={2}
              value={acceptanceImpact}
            />
          </label>
          <button
            className="primary"
            disabled={props.busy || !moveReady}
            onClick={() => void submitMove()}
            type="button"
          >
            确认移动并记录影响
          </button>
        </section>
      ) : null}
      <footer>
        <span>
          {props.task.assigneeLabel} ·{" "}
          {props.task.assigneeType === "agent"
            ? "智能体"
            : props.task.assigneeType === "collaboration"
              ? "协作"
              : "个人"}
        </span>
        <div>
          <button onClick={props.onOpenAi} type="button">
            <AiIcon />
            AI 执行
          </button>
          {movable ? (
            <button
              onClick={() => setShowMove((value) => !value)}
              type="button"
            >
              移动阶段
            </button>
          ) : null}
          {props.task.status === "review" ? (
            <button
              disabled={props.busy || !props.operatorReady}
              onClick={() => props.onTransition(props.task, "rework")}
              type="button"
            >
              请求返工
            </button>
          ) : null}
          {target ? (
            <button
              disabled={
                props.busy || !props.operatorReady || !props.operatorReady
              }
              onClick={() => props.onTransition(props.task, target)}
              type="button"
            >
              {target === "todo"
                ? "纳入待办"
                : target === "in_progress"
                  ? "开始执行"
                  : target === "review"
                    ? "提交验收"
                    : "确认完成"}
            </button>
          ) : null}
        </div>
      </footer>
    </article>
  );
}

function replaceTask(
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

function PlusIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24">
      <path d="M12 5v14M5 12h14" />
    </svg>
  );
}
function CheckIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 20 20">
      <path d="m4 10 4 4 8-9" />
    </svg>
  );
}
function BackArrowIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 20 20">
      <path d="m11 5-5 5 5 5M6.5 10H15" />
    </svg>
  );
}
function RouteIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 48 48">
      <circle cx="11" cy="11" r="4" />
      <circle cx="37" cy="37" r="4" />
      <path d="M11 15v8c0 4 3 7 7 7h12c4 0 7 3 7 7M18 11h15" />
    </svg>
  );
}
function TaskIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 48 48">
      <path d="M9 10h30v28H9zM16 18h16M16 25h16M16 32h10" />
    </svg>
  );
}
function MoreIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 20 20">
      <path d="M4.8 10h.01M10 10h.01M15.2 10h.01" />
    </svg>
  );
}
function DeliverableIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24">
      <path d="M4 6.5h6l1.5 2H20v10H4z" />
      <path d="m8 14 2.5 2.5L16 11" />
    </svg>
  );
}
function BlockerIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 20 20">
      <path d="M10 2.5 17 6v5c0 3.5-2.5 5.5-7 7-4.5-1.5-7-3.5-7-7V6z" />
      <path d="M10 6v5M10 14h.01" />
    </svg>
  );
}
function DecisionIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 20 20">
      <path d="M4 3h12v14H4zM7 7h6M7 10h4M7 13h2" />
      <path d="m12 14 1.5 1.5L17 12" />
    </svg>
  );
}
function AgentIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 20 20">
      <rect x="3" y="5" width="14" height="11" rx="2" />
      <path d="M10 2v3M6.5 10h.01M13.5 10h.01M7 13h6" />
    </svg>
  );
}
function AttentionIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 40 40">
      <circle cx="20" cy="20" r="14" />
      <path d="M20 12v9M20 27h.01" />
    </svg>
  );
}
function AiIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24">
      <path d="m12 3 1.7 5.3L19 10l-5.3 1.7L12 17l-1.7-5.3L5 10l5.3-1.7L12 3Z" />
      <path d="m19 16 .7 2.3L22 19l-2.3.7L19 22l-.7-2.3L16 19l2.3-.7L19 16Z" />
    </svg>
  );
}

function progressAiContext(
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
