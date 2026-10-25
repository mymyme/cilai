import { useEffect, useRef } from "react";
import type { AgentIncidentReportParams, AgentIncidentReportResult } from "./generated/protocol";
import { loadAiRuntimeQuery, type AiRunSummary, type AiRuntimeQuerySnapshot } from "./ai-runtime";
import { rpc, sha256 } from "./app-server-rpc";
import { describeServerError, serverErrorReason } from "./server-error";

/**
 * 把失败的智能体运行送进工作台。
 *
 * 运行页头部一直写着"失败与中断会作为关注事项进入工作台"，但这条链的前端一侧
 * 从来没有接上：服务端的 agentIncident/report 早就路由好了，没有任何调用方。
 * 结果是一次失败的运行只在运行页可见——不主动去那一页就永远不知道它失败了。
 *
 * 只上报当前任务已归类运行的明确重试链；同一对话中其他历史失败只保留在对话记录。
 * terminated 是用户自己叫停的，不算事故；
 * waiting_permission 是还活着的运行在等一个即时答复，给它建一张持久待办卡，
 * 运行结束后卡还留着，反而变成需要清理的垃圾。
 */
const incidentStatuses = ["failed", "interrupted"];

/** 轮询间隔比 AI 页面自己的要稀疏得多：这里只负责补记，不负责实时呈现。 */
const pollInterval = 15_000;

/**
 * 补记本身就该被跳过的两种情况。
 *
 * 这次运行已经有事故卡了，或者项目已被删除——都不是用户要处理的事，静静跳过即可。
 * 除此之外的失败都必须说出来：此处原先是一个不分青红皂白的 catch，于是上报被服务端
 * 整条拒绝时前端毫无察觉，事故卡长期没有建出来过而没有人知道。
 */
const skippableReasons = new Set(["agent_run_incident_exists", "project_not_found"]);

export function useAgentIncidentReporting(options: {
  workspaceRoot: string | null;
  /** 补记成功后通知调用方刷新工作台计数。 */
  onReported: () => void;
  /** 补记失败且不属于可跳过情形时告知调用方。 */
  onError: (message: string) => void;
}): void {
  const { workspaceRoot, onReported, onError } = options;
  // 同一次运行在一个会话里只尝试一次，失败也不重试：重试要么撞幂等要么撞
  // agent_run_incident_exists，都只是白跑一趟 IPC。
  const attempted = useRef<Set<string>>(new Set());
  const reportedRef = useRef(onReported);
  reportedRef.current = onReported;
  const errorRef = useRef(onError);
  errorRef.current = onError;

  useEffect(() => {
    if (workspaceRoot === null) return;
    let cancelled = false;
    let timer: number | null = null;

    const sweep = async () => {
      try {
        const snapshot = await loadAiRuntimeQuery({ scope: "incidents" });
        if (cancelled) return;
        let reported = false;
        for (const run of snapshot.runs) {
          if (!incidentStatuses.includes(run.status) || attempted.current.has(run.runId)) continue;
          attempted.current.add(run.runId);
          try {
            if (await reportIncident(run, snapshot)) reported = true;
          } catch (error) {
            // 单次补记失败不该让整轮扫描停下：后面几次运行各自独立，还有机会补记成功。
            errorRef.current(`运行事故未能记入工作台：${describeServerError(error)}`);
          }
          if (cancelled) return;
        }
        if (reported) reportedRef.current();
      } catch (error) {
        // 取快照失败属于这一轮读不到数据，下次唤醒会再看一遍；真正的补记失败在上面逐次报出。
        if (!cancelled) errorRef.current(`读取运行记录失败：${describeServerError(error)}`);
      } finally {
        if (!cancelled) timer = window.setTimeout(() => void sweep(), pollInterval);
      }
    };

    void sweep();
    return () => { cancelled = true; if (timer !== null) window.clearTimeout(timer); };
  }, [workspaceRoot]);
}

async function reportIncident(run: AiRunSummary, snapshot: AiRuntimeQuerySnapshot): Promise<boolean> {
  const conversation = snapshot.conversations.find((item) => item.conversationId === run.conversationId);
  const taskId = incidentTaskId(run, snapshot);
  // 关注事项只能挂到运行当时已确认的任务重试链。不能用对话现在的绑定
  // 反推一条历史运行，否则无关失败会把当前任务错误标成阻塞。
  const projectId = conversation?.context.projectId;
  if (!projectId || !taskId) return false;

  const agent = snapshot.agents.find((item) => item.agentId === run.agentId);
  const agentLabel = agent?.name ?? run.agentId;
  // operationId 由 runId 推导而不是随机：重复上报要命中幂等分支返回已有记录，
  // 用随机值会绕过它、撞上"这次运行已经有事故了"的冲突。
  const clientOperationId = `desktop-agent-incident-${run.runId}`;
  const params: AgentIncidentReportParams = {
    projectId,
    phaseId: conversation?.context.phaseId ?? null,
    taskId,
    runId: run.runId,
    agentLabel,
    title: `${agentLabel} 的运行${run.status === "failed" ? "失败" : "中断"}`,
    errorSummary: run.errorReason ?? (run.status === "failed" ? "运行失败，未给出原因。" : "运行在结束前被中断。"),
    responsibilityLabel: "user",
    reporterLabel: "desktop",
    // 服务端只接受 low/medium/high/critical。此处原为 normal，四级里没有这一档，
    // 于是每一次上报都被拒，事故卡从未被建出来过，而下面的 catch 把拒绝当成了
    // "已经有卡了"。补记不是紧急事项，取原意对应的中等档。
    priority: "medium",
    clientOperationId,
    requestFingerprint: await sha256([projectId, run.runId, run.status].join("\n")),
  };

  try {
    await rpc<AgentIncidentReportResult>("agentIncident/report", params);
    return true;
  } catch (error) {
    const reason = serverErrorReason(error);
    if (reason && skippableReasons.has(reason)) return false;
    throw error;
  }
}

function incidentTaskId(run: AiRunSummary, snapshot: AiRuntimeQuerySnapshot): string | null {
  const conversation = snapshot.conversations.find((item) => item.conversationId === run.conversationId);
  const classification = conversation?.workClassification;
  const taskId = classification?.taskId;
  if (!conversation || !classification || !taskId || conversation.context.taskId !== taskId) return null;

  const runsById = new Map(snapshot.runs.map((item) => [item.runId, item]));
  const seen = new Set<string>();
  let current: AiRunSummary | undefined = run;
  while (current) {
    if (current.runId === classification.runId) return taskId;
    const parentId = current.retryOfRunId;
    if (!parentId || seen.has(parentId)) return null;
    seen.add(parentId);
    current = runsById.get(parentId);
    if (current?.conversationId !== run.conversationId) return null;
  }
  return null;
}
