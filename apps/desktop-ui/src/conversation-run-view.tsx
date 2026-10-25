import { useEffect, useState } from "react";
import { conversationRunLabel } from "@cilai/app-ui/conversation";
import { conversationFileArtifacts, type ConversationArtifact } from "./ai-artifacts";
import { aiErrorMessage, loadAiRunEventDetail, type AiAgentRun, type AiRunEvent, type AiRunEventDetail, type AiWindowsSandboxStatus } from "./ai-runtime";
import { ArtifactVisual, ImageArtifactLightbox, imageArtifactMimeType } from "./artifact-preview";
import { writeClipboard } from "./clipboard";
import { fileNameFromPath, isBackgroundToolDiscoveryEvent, runDurationLabel } from "./conversation-display";
import { ChevronIcon, CloseIcon, CommandActivityIcon, ContextActivityIcon, CopyIcon, ExpandIcon, FileChangeActivityIcon, ImageMaterialIcon, PlanActivityIcon, ReasoningActivityIcon, ToolActivityIcon } from "./conversation-icons";
import { invoke } from "./host";
import { MarkdownContent } from "./markdown-content";
import { createPortal } from "./page-portal";
import { useModal } from "./use-modal";
export type ArtifactContextHandler = (event: React.MouseEvent, runId: string, artifact: ConversationArtifact) => void;
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

export function PendingReplyIntro() {
  return <div aria-label="回复准备中" className="ai-pending-reply-intro" role="status"><span aria-hidden="true" className="ai-pending-reply-dots"><i/><i/><i/></span></div>;
}

export function RunExecutionProcess({ active = false, progress, readOnly = false, run }: { active?: boolean; progress?: string; readOnly?: boolean; run: Pick<AiAgentRun, "runId" | "events"> | null }) {
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

export function RunArtifacts({ onOpen, onContext, run }: { onOpen: (artifact: ConversationArtifact) => void; onContext: ArtifactContextHandler; run: AiAgentRun }) {
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

export function ImageGenerationPending({ phase }: { phase: "generating" | "saving" }) {
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

export function WindowsSandboxGate(props: { loading: boolean; settingUp: boolean; status: AiWindowsSandboxStatus | null; onOpenSettings: () => void }) {
  const label = props.settingUp ? "正在自动修复 Windows 安全沙箱，请在系统弹窗中批准"
    : props.loading ? "正在检查 Windows 安全沙箱"
    : props.status?.status === "notConfigured" ? "Windows 安全沙箱未初始化"
      : props.status?.status === "updateRequired" ? "Windows 安全沙箱需要更新"
        : "Windows 安全沙箱不可用";
  return <button aria-label={`${label}，打开设置`} className="ai-windows-sandbox-gate" onClick={props.onOpenSettings} type="button">
    <span><SandboxGateIcon/>{label}</span><strong>打开设置</strong>
  </button>;
}

export function classificationFailureDetail(reason: string) {
  const code = reason.match(/\bAI-CLASSIFY-00[12]\b/)?.[0]
    ?? (/格式|解析|schema|structured|invalid\s+json/i.test(reason) ? "AI-CLASSIFY-002" : "AI-CLASSIFY-001");
  const requestId = reason.match(/(?:请求编号|request(?:[ _-]?id)?)\s*[:：=]\s*([a-z0-9-]{8,})/i)?.[1];
  const summary = code === "AI-CLASSIFY-002"
    ? "工作归类结果格式不符合要求，本次回复已保留，任务状态未更新。"
    : "工作归类暂时失败，本次回复已保留，任务状态未更新。";
  return { summary, code, requestId, text: `${summary}\n错误代码：${code}${requestId ? `\n请求编号：${requestId}` : ""}` };
}

/** 正常运行的执行记录跟随对应回复；这里只保留无法附着到回复的异常事实。 */
export function ConversationRun({ hasAssistantMessage, modelLabel, run }: { hasAssistantMessage: boolean; modelLabel: string | null; run: AiAgentRun | null }) {
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

export function incompleteRunSummary(run: AiAgentRun) {
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

export function RunRecordIcon({ status }: { status: string }) {
  if (status === "completed") return <svg aria-hidden="true" viewBox="0 0 20 20"><circle cx="10" cy="10" r="7"/><path d="m6.8 10 2 2 4.4-4.4"/></svg>;
  if (["failed", "interrupted", "terminated"].includes(status)) return <svg aria-hidden="true" viewBox="0 0 20 20"><circle cx="10" cy="10" r="7"/><path d="M10 6.5v4M10 13.5h.01"/></svg>;
  return <svg aria-hidden="true" viewBox="0 0 20 20"><circle cx="10" cy="10" r="7"/><path d="M10 6v4l2.5 1.5"/></svg>;
}

function SandboxGateIcon() {
  return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="m10 2.5 6 2.6v4.2c0 4-2.4 6.9-6 8.2-3.6-1.3-6-4.2-6-8.2V5.1z"/><path d="M10 6.2v4.1M10 13.2h.01"/></svg>;
}

export function runStatusLabel(status: string) { return conversationRunLabel(status); }

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

export function runProgress(run: AiAgentRun | null) {
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
