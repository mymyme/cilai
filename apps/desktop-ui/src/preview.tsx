import { invoke } from "./host";
import { OfficeDocumentPreview, StructuredDocumentPreview } from "./document-preview/viewer";
import type { DocumentPreviewModel, OfficePreviewFormat } from "./document-preview/model";
import { readBinaryPreview } from "./preview-binary";
import { rpc } from "./app-server-rpc";
import { hydrateFileForRead } from "./file-hydration";
import { useEffect, useRef, useState } from "react";
import { usePageActive } from "./page-visibility";
import { DocumentLayoutPreview } from "./document-layout-preview";
import { PagedTextPreview, readPreviewPage, readPreviewText } from "./preview-text-page";
import type { WorkspaceFileItem } from "./generated/protocol";
import { ExternalOpenIcon } from "./icons";
import { FileMarkdownPreview } from "./file-markdown-preview";
import type { PdfJsDocument, PdfJsLoadingTask, PdfJsRenderTask } from "./pdfjs-renderer";
import type {
  OpenPreviewParams,
  OpenPreviewResult,
  PreviewFormat,
  PreviewSource,
  ReadTextResourceResult,
  ReleaseTextResourceResult,
} from "./generated/protocol";

export { ERROR_LABELS, SOURCE_LABELS } from "./preview-labels";
import { ERROR_LABELS, SOURCE_LABELS } from "./preview-labels";

interface WorkspaceApplication {
  displayName: string;
}

type PreviewState =
  | { phase: "idle" }
  | { phase: "loading"; source: PreviewSource }
  | { phase: "hydrating"; cancel: () => void }
  | { phase: "ready-binary"; descriptor: OpenPreviewResult; bytes: ArrayBuffer; contentType: string }
  | { phase: "ready-structure"; descriptor: OpenPreviewResult; model: DocumentPreviewModel }
  | { phase: "ready"; descriptor: OpenPreviewResult; content: string }
  | { phase: "ready-text-page"; descriptor: OpenPreviewResult; firstPage: ReadTextResourceResult }
  | { phase: "unsupported"; descriptor: OpenPreviewResult; application: WorkspaceApplication | null }
  | { phase: "unavailable"; descriptor: OpenPreviewResult }
  | { phase: "error"; source: PreviewSource; reason: string };

export interface FilePreviewPaneProps {
  workspaceId: string;
  workspaceKind: string;
  localRoot: string;
  hostPlatform: string | null;
  file: WorkspaceFileItem;
  serviceScope?: string | null;
  view: PreviewView;
  onViewAvailabilityChange: (available: boolean) => void;

}

export function FilePreviewPane({ workspaceId, workspaceKind, localRoot, hostPlatform, file, serviceScope, view, onViewAvailabilityChange }: FilePreviewPaneProps) {
  const [retryRevision, setRetryRevision] = useState(0);
  const [applicationOpenError, setApplicationOpenError] = useState<string | null>(null);
  const localState = useFilePreview({ workspaceId, localFileId: file.localFileId, source: "local" }, localRoot, file.relativePath, hostPlatform, retryRevision, workspaceKind === "local", JSON.stringify([serviceScope,file.checksumAlgorithm,file.checksumValue]));
  const state: PreviewState = workspaceKind !== "local" ? { phase: "error", source: "cloud", reason: "personal_cloud_source_unavailable" } : localState;
  // 纯文本、代码这类本来就没有"渲染后"形态的文件不显示切换，两种看法的内容本就一样。
  const canSwitchView = state.phase === "ready" && hasRenderedView(state.descriptor.format);
  useEffect(() => { onViewAvailabilityChange(canSwitchView); }, [canSwitchView, onViewAvailabilityChange]);
  return (
    <section className="preview-pane" aria-label="文件预览">
      <PreviewBody
        applicationOpenError={applicationOpenError}
        localRoot={localRoot}
        state={state}
        view={view}
        onOpenApplication={() => {
          setApplicationOpenError(null);
          if (workspaceKind === "cloud_team") {
            setApplicationOpenError("云端文件不对应本机应用。");
            return;
          }
          void invoke("workspace_open_item", { localRoot, relativePath: file.relativePath, openMode: "default" })
            .catch(() => setApplicationOpenError("无法打开此文件。"));
        }}
        onRetry={() => setRetryRevision((revision) => revision + 1)}
      />
    </section>
  );
}

/** 正文的两种看法：渲染好的成品，和文件里真正写着的东西。 */
export type PreviewView = "rendered" | "source";

/** 只有存在"渲染后"这一形态的格式才谈得上两种看法。 */
function hasRenderedView(format: PreviewFormat): boolean {
  return format === "markdown" || format === "html";
}

function PreviewBody({ state, onRetry, view, onOpenApplication, applicationOpenError }: {
  state: PreviewState;
  onRetry: () => void;
  view: PreviewView;
  onOpenApplication: () => void;
  applicationOpenError: string | null;
  localRoot: string;
}) {
  if (state.phase === "idle") return <PreviewMessage title="请选择文件" />;
  if (state.phase === "hydrating") return <PreviewMessage title="正在下载" actionLabel="取消" onAction={state.cancel}/>;
  if (state.phase === "ready-structure") return <StructuredDocumentPreview model={state.model}/>;
  if (state.phase === "ready-binary") {
    if (state.descriptor.format === "image") return <ImagePreview bytes={state.bytes} contentType={state.contentType} name={state.descriptor.relativePath}/>;
    if (state.descriptor.format === "pdf") return <PdfPreview descriptor={state.descriptor} bytes={state.bytes}/>;
    return <OfficeDocumentPreview bytes={state.bytes} format={state.descriptor.format as OfficePreviewFormat}/>;
  }
  if (state.phase === "loading") {
    return <PreviewMessage title={`正在读取${SOURCE_LABELS[state.source]}`} />;
  }
  if (state.phase === "error") {
    return (
      <PreviewMessage
        detail={ERROR_LABELS[state.reason] ?? "预览请求失败，请查看诊断信息后重试。"}
        source={state.source}
        title="读取失败"
        actionLabel="重试预览"
        onAction={onRetry}
      />
    );
  }
  if (state.phase === "unsupported") {
    return <ExternalApplicationChooser
      application={state.application}
      openError={applicationOpenError}
      onOpen={onOpenApplication}/>;
  }
  if (state.phase === "unavailable") {
    return (
      <PreviewMessage
        detail={state.descriptor.reason === "cloud_preview_format_unsupported" ? "当前云端文件类型暂不支持应用内预览。" : "该来源的正文尚未物化到本地，未使用其他来源内容替代。"}
        path={state.descriptor.relativePath}
        source={state.descriptor.source}
        title="来源内容不可用"
      />
    );
  }
  if (state.phase === "ready-text-page") {
    return <div className="preview-document"><PagedTextPreview key={state.descriptor.ticketId} descriptor={state.descriptor} firstPage={state.firstPage} onRetry={onRetry}/></div>;
  }
  const { descriptor, content } = state;
  const fillsPreview = descriptor.format === "html" && view === "rendered";
  return (
    <div className={`preview-document${fillsPreview ? " preview-document-html" : ""}`}>
      <PreviewContent content={content} format={descriptor.format} view={view}/>
    </div>
  );
}

function PdfPreview({ descriptor, bytes }: { descriptor: OpenPreviewResult; bytes: ArrayBuffer }) {
  const active = usePageActive();
  const [document, setDocument] = useState<PdfJsDocument | null>(null);
  const [pageNumber, setPageNumber] = useState(1);
  const [renderedPage, setRenderedPage] = useState<number | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [renderError, setRenderError] = useState<string | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    let disposed = false;
    let loadingTask: PdfJsLoadingTask | null = null;
    setDocument(null);
    setPageNumber(1);
    setRenderedPage(null);
    setLoadError(null);
    setRenderError(null);
    void Promise.resolve(bytes).then(async (payload) => {
      if (disposed) return;
      const { openPdfDocument } = await import("./pdfjs-renderer");
      if (disposed) return;
      loadingTask = openPdfDocument(normalizeBinaryPayload(payload));
      const loaded = await loadingTask.promise;
      if (disposed) return;
      setDocument(loaded);
    }).catch((error: unknown) => {
      if (!disposed) setLoadError(extractReason(error));
    });
    return () => {
      disposed = true;
      void loadingTask?.destroy();
    };
  }, [bytes, descriptor.contentRevision, descriptor.relativePath]);

  useEffect(() => {
    if (!active || !document) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    let disposed = false;
    let task: PdfJsRenderTask | null = null;
    setRenderedPage(null);
    setRenderError(null);
    task = document.renderPage(canvas, pageNumber, 1600);
    void task.promise.then(() => {
      if (!disposed) setRenderedPage(pageNumber);
    }).catch((error: unknown) => {
      if (!disposed) setRenderError(extractReason(error));
    });
    return () => {
      disposed = true;
      task?.cancel();
    };
  }, [active, document, pageNumber]);

  if (loadError) {
    return <PreviewMessage detail={ERROR_LABELS[loadError] ?? "PDF 文档无法解析。"} title="无法显示 PDF"/>;
  }
  if (!document) return <div className="preview-pdf-loading">正在读取 PDF…</div>;
  return <div className="preview-pdf-document">
    <div className="preview-pdf-toolbar" aria-label="PDF 页面导航">
      <button disabled={pageNumber <= 1} onClick={() => setPageNumber((page) => page - 1)} type="button">上一页</button>
      <span>第 {pageNumber} / {document.pageCount} 页</span>
      <button disabled={pageNumber >= document.pageCount} onClick={() => setPageNumber((page) => page + 1)} type="button">下一页</button>
    </div>
    {renderError ? <PreviewMessage detail={ERROR_LABELS[renderError] ?? "PDF 页面渲染失败。"} title="无法显示 PDF"/> : null}
    {!renderError ? <div className="preview-pdf-page" aria-busy={renderedPage !== pageNumber}>
      <canvas aria-label={`PDF 第 ${pageNumber} 页`} hidden={renderedPage !== pageNumber} ref={canvasRef} role="img"/>
      {renderedPage !== pageNumber ? <div className="preview-pdf-loading">正在渲染第 {pageNumber} 页…</div> : null}
    </div> : null}
  </div>;
}

function ImagePreview({ bytes, contentType, name }: { bytes: ArrayBuffer; contentType: string; name: string }) {
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    const current = URL.createObjectURL(new Blob([bytes], { type: contentType }));
    setUrl(current); setFailed(false);
    return () => URL.revokeObjectURL(current);
  }, [bytes, contentType]);
  return failed ? <PreviewMessage title="图片无法解码"/> : <div className="preview-image-document">{url ? <img alt={name} onError={() => setFailed(true)} src={url}/> : null}</div>;
}
function normalizeBinaryPayload(payload: ArrayBuffer | Uint8Array | number[]): ArrayBuffer {
  if (payload instanceof ArrayBuffer) return payload;
  const bytes = payload instanceof Uint8Array ? payload : Uint8Array.from(payload);
  const normalized = new Uint8Array(bytes.byteLength);
  normalized.set(bytes);
  return normalized.buffer;
}

function ExternalApplicationChooser({ application, openError, onOpen }: {
  application: WorkspaceApplication | null;
  openError: string | null;
  onOpen: () => void;
}) {
  return <div className="external-app-chooser" role="status">
    {application ? <button className="external-app-default" onClick={onOpen} type="button">
      <ExternalOpenIcon/>
      <span>{application.displayName}</span>
    </button> : <p className="external-app-empty">没有可以打开此文件的软件</p>}
    {openError ? <p className="external-app-error" role="alert">{openError}</p> : null}
  </div>;
}

function PreviewContent({ content, format, view }: { content: string; format: PreviewFormat; view: PreviewView }) {
  if (view === "source" || !hasRenderedView(format)) {
    return <pre className="preview-plain-text"><code>{content}</code></pre>;
  }
  if (format === "html") return <DocumentLayoutPreview format="html" input={content}/>;
  return <FileMarkdownPreview content={content}/>;
}

function PreviewMessage({
  title,
  detail,
  source,
  path,
  actionLabel,
  onAction,
}: {
  title: string;
  detail?: string;
  source?: PreviewSource;
  path?: string;
  actionLabel?: string;
  onAction?: () => void;
}) {
  return (
    <div className="preview-message" role="status">
      {source ? <span className="preview-source-label">{SOURCE_LABELS[source]}</span> : null}
      <h3>{title}</h3>
      {path ? <code>{path}</code> : null}
      {detail ? <p>{detail}</p> : null}
      {actionLabel && onAction ? (
        <button className="secondary-button preview-retry-button" onClick={onAction} type="button">
          {actionLabel}
        </button>
      ) : null}
    </div>
  );
}

function useFilePreview(params: OpenPreviewParams, localRoot: string, relativePath: string, hostPlatform: string | null, retryRevision: number, enabled = true, contentRevision = ""): PreviewState {
  const pageActive = usePageActive();
  const loadedKey = useRef<string | null>(null);
  const loadingKey = useRef<string | null>(null);
  const ownedTicket = useRef<string | null>(null);
  const key = JSON.stringify([enabled,hostPlatform,localRoot,params.localFileId,params.source,params.workspaceId,relativePath,retryRevision,contentRevision]);
  const [state, setState] = useState<PreviewState>({ phase: "idle" });
  useEffect(() => () => {
    const ticket = ownedTicket.current;
    ownedTicket.current = null;
    if (ticket) releaseTextTicket(ticket);
  }, [key]);
  useEffect(() => {
    if (!pageActive) return;
    if (loadedKey.current === key) return;
    if (!enabled) { loadedKey.current = null; setState({ phase: "idle" }); return; }
    let cancelled = false;
    const controller = new AbortController();
    const cancelDownload = () => {
      cancelled = true;
      controller.abort();
    };
    loadingKey.current = key;
    setState({ phase: "loading", source: params.source });
    void (async () => {
      const downloaded = await hydrateFileForRead(params.workspaceId, relativePath, controller.signal, () => {
        setState({ phase: "hydrating", cancel: () => { cancelDownload(); loadedKey.current = key; setState({ phase: "error", source: "local", reason: "file_hydration_cancelled" }); } });
      });
      if (cancelled) return;
      if (downloaded) await rpc("workspace/indexPath", { workspaceId: params.workspaceId, relativePath });
      if (cancelled) return;
      const next = await loadPreview(params, localRoot, relativePath, controller.signal);
      if (cancelled) {
        if (next.phase === "ready-text-page" && next.descriptor.ticketId) releaseTextTicket(next.descriptor.ticketId);
        return;
      }
      if (next.phase === "ready-text-page") ownedTicket.current = next.descriptor.ticketId;
      loadedKey.current = key;
      setState(next);
    })().catch((error: unknown) => { if (!cancelled) { loadedKey.current = key; setState({ phase: "error", source: params.source, reason: extractReason(error) }); } });
    return cancelDownload;
  }, [enabled, hostPlatform, localRoot, params.localFileId, params.source, params.workspaceId, relativePath, retryRevision, contentRevision, pageActive, key]);
  return enabled && loadedKey.current !== key && !(state.phase === "hydrating" && loadingKey.current === key) ? {phase:"loading",source:params.source} : state;
}

async function loadPreview(params: OpenPreviewParams, localRoot: string, relativePath: string, signal: AbortSignal): Promise<PreviewState> {
  signal.throwIfAborted();
  const descriptor = await invoke<OpenPreviewResult>("preview_open", { params });
  if (signal.aborted && descriptor.ticketId) releaseTextTicket(descriptor.ticketId);
  signal.throwIfAborted();
  if (descriptor.availability === "unsupported") {
    try {
      const application = await invoke<WorkspaceApplication | null>("workspace_default_application", { localRoot, relativePath });
      return { phase: "unsupported", descriptor, application };
    } catch {
      return { phase: "unsupported", descriptor, application: null };
    }
  }
  if (descriptor.availability === "unavailable") {
    return { phase: "unavailable", descriptor };
  }
  if (["image", "pdf", "docx", "pptx", "xlsx", "xmind"].includes(descriptor.format)) {
    const binary = await readBinaryPreview(params, signal);
    return { phase: "ready-binary", descriptor: { ...descriptor, contentRevision: binary.contentRevision, sizeBytes: binary.bytes.byteLength }, bytes: binary.bytes, contentType: binary.contentType };
  }
  if (!descriptor.ticketId || !descriptor.contentRevision) {
    throw new Error("preview_descriptor_invalid");
  }
  if (descriptor.format === "plain_text") {
    try {
      const firstPage = await readPreviewPage(descriptor, 0, signal);
      return { phase: "ready-text-page", descriptor, firstPage };
    } catch (error) {
      releaseTextTicket(descriptor.ticketId);
      throw error;
    }
  }
  try {
    return { phase: "ready", descriptor, content: await readPreviewText(descriptor, signal) };
  } finally {
    releaseTextTicket(descriptor.ticketId);
  }
}

function releaseTextTicket(ticketId: string) {
  void rpc<ReleaseTextResourceResult>("textResource/release", { ticketId }).catch(error => console.error("预览资源未能释放", error));
}

function extractReason(error: unknown): string {
  const value = String(error);
  const known = Object.keys(ERROR_LABELS).find((reason) => value.includes(reason));
  return known ?? (value.includes("source_not_materialized") ? "preview_source_not_materialized" : "preview_read_failed");
}
