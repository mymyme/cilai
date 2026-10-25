import { useCallback, useEffect, useRef, useState } from "react";
import { loadAiRuntimeQuery, type AiConversation, type AiRunSummary } from "./ai-runtime";
import type { MessageSink } from "./app-message";
import { rpc } from "./app-server-rpc";
import type { AttentionCaseItem, AttentionCaseListResult, BusinessTaskItem, DeliverableItem, DeliverableListResult, PhasePlanItem, ProjectGetResult, ProjectListResult, ProjectProgressGetResult, ProjectSummary, WorkspaceSummary } from "./generated/protocol";
interface ProjectProgressDataOptions {
  workspace: WorkspaceSummary | null;
  eventSequence: number;
  initialProjectId: string | null;
  initialPhaseId: string | null;
  onMessage: MessageSink;
}
export function useProjectProgressData(props: ProjectProgressDataOptions) {
  const scopeActive = useRef(true);
  const loadGeneration = useRef(0);
  const mutationRevision = useRef(0);
  useEffect(() => {
    scopeActive.current = true;
    return () => { scopeActive.current = false; loadGeneration.current += 1; };
  }, []);
  const reportMessage = useCallback<MessageSink>((message, tone) => {
    if (scopeActive.current) props.onMessage(message, tone);
  }, [props.onMessage]);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [deliverables, setDeliverables] = useState<DeliverableItem[]>([]);
  const [plans, setPlans] = useState<PhasePlanItem[]>([]);
  const [projectTasks, setProjectTasks] = useState<BusinessTaskItem[]>([]);

  const [projectConversations, setProjectConversations] = useState<
    AiConversation[]
  >([]);
  const [projectRuns, setProjectRuns] = useState<AiRunSummary[]>([]);
  const [selectedPhaseId, setSelectedPhaseId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  // 读不出来和这里是空的必须分开：此前失败会落进"尚无阶段计划"，那是一句假话。
  const [loadError, setLoadError] = useState<unknown>(null);
  const [reloadToken, setReloadToken] = useState(0);

  const [attentionCases, setAttentionCases] = useState<AttentionCaseItem[]>([]);

  // Tracks the latest initialPhaseId without making loadProgress depend on it: onPhaseSelected
  // above writes every in-page phase click back into initialPhaseId, so making loadProgress react
  // to it would re-trigger the whole load cascade on every phase click.
  const initialPhaseIdRef = useRef(props.initialPhaseId);
  initialPhaseIdRef.current = props.initialPhaseId;

  const loadProgress = useCallback(async (nextProjectId: string, request: number) => {
    const revision = mutationRevision.current;
    const current = () => scopeActive.current && request === loadGeneration.current && revision === mutationRevision.current;
    const results =
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
        loadAiRuntimeQuery({ scope: "project", projectId: nextProjectId }),
      ]).catch((error) => {
        if (current()) throw error;
        return null;
      });
    if (!current() || !results) return;
    const [deliverableResult, progressResult, attentionResult, aiSnapshot] = results;
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
    async (workspaceId: string, request: number) => {
      const result = await rpc<ProjectListResult>("project/list", {
        workspaceId,
      });
      if (!scopeActive.current || request !== loadGeneration.current) return;
      setProjects(result.projects);
      const requestedProjectMissing =
        Boolean(props.initialProjectId) &&
        !result.projects.some(
          (project) => project.projectId === props.initialProjectId,
        );
      if (requestedProjectMissing)
        reportMessage(
          "指定的项目在当前工作区中未找到，可能已失效或已解除关联。",
          "error",
        );
      const nextProjectId = props.initialProjectId
        ? requestedProjectMissing
          ? null
          : props.initialProjectId
        : (result.projects[0]?.projectId ?? null);
      setProjectId(nextProjectId);
      if (nextProjectId) await loadProgress(nextProjectId, request);
      else {
        setDeliverables([]);
        setPlans([]);
        setProjectTasks([]);
        setAttentionCases([]);
        setProjectConversations([]);
        setProjectRuns([]);
      }
    },
    [loadProgress, props.initialProjectId, reportMessage],
  );

  useEffect(() => {
    const request = ++loadGeneration.current;
    const revision = mutationRevision.current;
    if (!props.workspace && !props.initialProjectId) {
      setProjects([]);
      setProjectId(null);
      setDeliverables([]);
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
          if (cancelled || request !== loadGeneration.current) return;
          setProjects([result.project]);
          setProjectId(result.project.projectId);
          await loadProgress(result.project.projectId, request);
        })
      : loadProjects(props.workspace!.workspaceId, request);
    void load
      .catch((error) => {
        if (!cancelled && request === loadGeneration.current && revision === mutationRevision.current) {
          setLoadError(error);
          reportMessage(String(error), "error");
        }
      })
      .finally(() => {
        if (!cancelled && request === loadGeneration.current) setLoading(false);
      });
    return () => {
      cancelled = true;
      loadGeneration.current += 1;
    };
  }, [
    loadProgress,
    loadProjects,
    props.eventSequence,
    props.initialProjectId,
    reportMessage,
    props.workspace,
    reloadToken,
  ]);

  const refresh = async () => {
    if (!scopeActive.current || !projectId) return;
    const request = ++loadGeneration.current;
    try { await loadProgress(projectId, request); }
    finally { if (scopeActive.current && request === loadGeneration.current) setLoading(false); }
  };

  const isScopeActive = useCallback(() => scopeActive.current, []);
  const markMutation = useCallback(() => { mutationRevision.current += 1; }, []);
  return { projects, projectId, deliverables, setDeliverables, plans, setPlans, projectTasks, setProjectTasks, projectConversations, projectRuns, selectedPhaseId, setSelectedPhaseId, loading, loadError, setReloadToken, attentionCases, setAttentionCases, refresh, reportMessage, isScopeActive, markMutation };
}
