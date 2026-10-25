import { useEffect,useMemo,useRef,useState } from "react";
import { conversationFileArtifacts } from "./ai-artifacts";
import {
type AiContext,
type AiConversationMode,
type AiConversationType
} from "./ai-runtime";
import type { MessageSink } from "./app-message";
import { rpc,sha256 } from "./app-server-rpc";
import { ImageArtifactLightbox } from "./artifact-preview";
import { ConfirmDialog } from "./confirm-dialog";
import type {
AttentionCaseItem,
AttentionCaseTransitionResult,
BusinessTaskItem,
BusinessTaskMoveResult,
BusinessTaskTransitionResult,
DecisionRequestCreateResult,
DeliverableItem,
MyWorkItem,
PhasePlanActivateResult,
PhasePlanCreateResult,
PhasePlanItem,
ProjectBlockerCreateResult,
ProjectPhaseItem,
ProjectPhaseTransitionResult,
WorkspaceFileItem,
WorkspaceSummary
} from "./generated/protocol";
import { LoadFailure } from "./load-failure";
import { TaskDetailDrawer } from "./my-work";
import { DeliverableManager } from "./project-deliverables";
import { AttentionPanel } from './project-progress-attention';
import { AiIcon,BackArrowIcon,CheckIcon,DeliverableIcon,MoreIcon,PlusIcon,RouteIcon,TaskIcon } from './project-progress-icons';
import { attentionCommentRequired,emptyPhase,phaseCommentRequired,PhaseDraft,phaseStatusLabel,progressAiContext,replaceTask,taskCommentRequired } from './project-progress-model';
import { ProjectOverview,ProjectTaskLedger } from './project-progress-overview';
import { PlanComposer,TaskCard } from './project-progress-plan';
import { type TaskArtifactPreview } from "./task-artifacts";
import { useProjectProgressData } from "./use-project-progress-data";
import { randomUuid } from "./uuid";

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

export function ProjectProgressPage(props: ProjectProgressPageProps) {
  return <ProjectProgressView key={JSON.stringify([props.initialProjectId, props.workspace?.workspaceId ?? null])} {...props}/>;
}

function ProjectProgressView(props: ProjectProgressPageProps) {
  const { projects, projectId, deliverables, plans, setPlans, projectTasks, setProjectTasks, projectConversations, projectRuns, selectedPhaseId, setSelectedPhaseId, loading, loadError, setReloadToken, attentionCases, setAttentionCases, refresh, reportMessage, isScopeActive, markMutation } = useProjectProgressData(props);
  const [activeTaskDetail, setActiveTaskDetail] = useState<MyWorkItem | null>(null);
  const [taskStatusFilter, setTaskStatusFilter] = useState("all");
  const [artifactPreview, setArtifactPreview] = useState<TaskArtifactPreview | null>(null);
  const [projectView, setProjectView] = useState<
    "overview" | "phases" | "tasks"
  >(props.initialPhaseId ? "phases" : "overview");
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
      if (!isScopeActive()) return;
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
      if (!isScopeActive()) return;
      markMutation();
      setPlans((current) => [result.plan, ...current]);
      setSelectedPhaseId(result.plan.phases[0]?.phaseId ?? null);
      setShowPlanForm(false);
      setPlanSummary("");
      setPhaseDrafts([emptyPhase()]);
      reportMessage("阶段计划草案已创建。激活前仍不会改变当前项目推进状态。");
    } catch (error) {
      reportMessage(String(error), "error");
    } finally {
      if (isScopeActive()) setBusy(false);
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
      if (!isScopeActive()) return;
      const result = await rpc<PhasePlanActivateResult>("phasePlan/activate", {
        planId: plan.planId,
        expectedRevision: plan.stateRevision,
        clientOperationId,
        requestFingerprint,
      });
      if (!isScopeActive()) return;
      markMutation();
      setPlans((current) =>
        current.map((item) =>
          item.planId === result.plan.planId ? result.plan : item,
        ),
      );
      setSelectedPhaseId(result.plan.phases[0]?.phaseId ?? null);
      reportMessage("阶段计划已激活，可以启动首个阶段。");
    } catch (error) {
      reportMessage(String(error), "error");
    } finally {
      if (isScopeActive()) setBusy(false);
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
      if (!isScopeActive()) return;
      await rpc<ProjectPhaseTransitionResult>("projectPhase/transition", {
        phaseId: phase?.phaseId ?? null,
        targetStatus,
        expectedRevision: phase.stateRevision,
        actorLabel: operator.trim(),
        comment,
        clientOperationId,
        requestFingerprint,
      });
      if (!isScopeActive()) return;
      markMutation();
      await refresh();
      if (!isScopeActive()) return;
      reportMessage(
        targetStatus === "completed"
          ? "阶段验收完成，后继阶段现在可以启动。"
          : "阶段状态已更新。",
      );
    } catch (error) {
      reportMessage(String(error), "error");
    } finally {
      if (isScopeActive()) setBusy(false);
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
      if (!isScopeActive()) return;
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
      if (!isScopeActive()) return;
      markMutation();
      setPlans((current) => replaceTask(current, result.task));
      setProjectTasks((current) =>
        current.map((item) =>
          item.businessTaskId === result.task.businessTaskId
            ? result.task
            : item,
        ),
      );
      reportMessage(
        targetStatus === "rework"
          ? "任务已进入新的返工尝试，原验收反馈继续保留。"
          : "任务状态已更新。",
      );
    } catch (error) {
      reportMessage(String(error), "error");
    } finally {
      if (isScopeActive()) setBusy(false);
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
      if (!isScopeActive()) return;
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
      if (!isScopeActive()) return;
      markMutation();
      await refresh();
      if (!isScopeActive()) return;
      reportMessage("任务已移动到目标阶段，影响说明已写入审计记录。");
    } catch (error) {
      reportMessage(String(error), "error");
    } finally {
      if (isScopeActive()) setBusy(false);
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
      if (!isScopeActive()) return;
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
        if (!isScopeActive()) return;
        markMutation();
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
        if (!isScopeActive()) return;
        markMutation();
        setAttentionCases((current) => [result.decisionRequest, ...current]);
      }
      resetAttentionForm();
      reportMessage(
        attentionMode === "blocker"
          ? "项目阻塞已记录，并同步进入个人工作台。"
          : "决策请求已发起，并同步进入个人工作台。",
      );
    } catch (error) {
      reportMessage(String(error), "error");
    } finally {
      if (isScopeActive()) setBusy(false);
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
      if (!isScopeActive()) return;
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
      if (!isScopeActive()) return;
      markMutation();
      setAttentionCases((current) =>
        current.map((item) =>
          item.attentionCaseId === result.attentionCase.attentionCaseId
            ? result.attentionCase
            : item,
        ),
      );
      reportMessage(
        targetStatus === "acknowledged"
          ? "智能体运行异常已知晓，已进入处置中。"
          : decisionAction === "delegated"
            ? "决策已转交，新的责任人已同步到个人工作台。"
            : "关注事项已解决，个人工作台投影已同步清理。 ",
      );
    } catch (error) {
      reportMessage(String(error), "error");
    } finally {
      if (isScopeActive()) setBusy(false);
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
      reportMessage("该交付物还没有绑定文件。", "error");
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
                            reportMessage(
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
        onMessage={reportMessage}
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
          onMessage={reportMessage}
          project={selectedProject}
          workspace={props.workspace}
        />
      ) : null}
    </>
  );
}
