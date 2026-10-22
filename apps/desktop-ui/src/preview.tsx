import { invoke } from "./host";
import { OfficeDocumentPreview, StructuredDocumentPreview } from "./document-preview/viewer";
import type { DocumentPreviewModel, OfficePreviewFormat } from "./document-preview/model";
import type { ReadBinaryPreviewResult } from "./generated/protocol";
import { rpc } from "./app-server-rpc";
import { hydrateFileForRead } from "./file-hydration";
import { useEffect, useRef, useState } from "react";
import { usePageActive } from "./page-visibility";
import type { WorkspaceFileItem } from "./generated/protocol";
import { ExternalOpenIcon } from "./icons";
import { MarkdownContent } from "./markdown-content";
import type { PdfJsDocument, PdfJsLoadingTask, PdfJsRenderTask } from "./pdfjs-renderer";
import type {
  OpenPreviewParams,
  OpenPreviewResult,
  PreviewFormat,
  PreviewSource,
  ReadTextResourceParams,
  ReadTextResourceResult,
} from "./generated/protocol";

export const SOURCE_LABELS: Record<PreviewSource, string> = {
  local: "当前文档",
  cloud: "旧云端来源",
  baseline: "上次确认",
  platform_mock: "平台版本",
};

export const ERROR_LABELS: Record<string, string> = {
  personal_cloud_source_unavailable: "个人版不支持此云端来源，请明确选择本地文件。",
  preview_file_too_large: "文件超过 32 MiB，无法在客户端预览。",
  preview_binary_content: "文件包含二进制内容，不能作为文本预览。",
  preview_encoding_unsupported: "当前编码不受支持，M0 仅支持 UTF-8。",
  preview_source_changed: "文件在读取过程中发生变化，请重新加载。",
  preview_permission_denied: "当前没有读取该文件的权限。",
  preview_file_missing: "文件已不存在。",
  preview_file_not_materialized: "系统尚未完成文件下载，请重试。",
  preview_source_not_materialized: "系统尚未完成文件下载，请重试。",
  preview_read_failed: "文件内容读取失败，请重试。",
  preview_binary_format_invalid: "文件内容与扩展名不匹配。",
  preview_descriptor_invalid: "预览数据不完整。",
  preview_resource_invalid: "预览图片资源不完整或不受支持。",
  preview_format_unsupported: "当前文件类型暂不支持应用内预览。",
  cloud_preview_format_unsupported: "当前文件类型暂不支持应用内预览。",
  cloud_preview_unavailable: "文档预览服务尚未配置。",
  file_hydration_cancelled: "已取消等待下载。",
  file_hydration_failed: "系统下载文件失败。",
  file_hydration_timeout: "系统下载文件超时。",
  file_hydration_provider_unsupported: "当前系统提供程序不支持自动下载。",
  file_hydration_source_changed: "下载期间文件位置或内容已变化，请重新打开。",
  file_hydration_download_failed: "系统下载文件失败。",
  file_hydration_file_unavailable: "文件当前不可访问。",
  file_hydration_operation_expired: "下载等待已过期，请重试。",
  file_hydration_busy: "系统下载繁忙，请稍后重试。",
  collaboration_permission_denied: "当前没有读取该云端文件的权限。",
  cloud_workspace_permission_revoked: "当前没有读取该云端空间的权限。",
  preview_package_invalid: "文档损坏、加密或格式不受支持。",
  preview_ticket_expired: "预览已过期，请重新加载。",
  preview_ticket_not_found: "预览资源已失效，请重新加载。",
  workspace_pdf_format_invalid: "当前文件不是 PDF。",
  workspace_pdf_not_file: "当前路径不是可读取的 PDF 文件。",
  workspace_pdf_not_materialized: "PDF 仅保存在云端，请先用系统打开完成下载。",
  workspace_pdf_too_large: "PDF 超过 32 MiB，无法在内容区打开。",
  workspace_pdf_signature_invalid: "文件扩展名是 PDF，但内容不是有效的 PDF 文件。",
  workspace_pdf_read_failed: "PDF 内容读取失败。",
  workspace_pdf_source_changed: "PDF 在读取过程中发生变化，请重新打开。",
  workspace_pdf_document_invalid: "PDF 文档无法解析。",
  workspace_pdf_page_unavailable: "PDF 页面无法读取。",
  workspace_pdf_render_buffer_invalid: "PDF 页面渲染失败。",
  workspace_pdf_page_count_invalid: "PDF 页数读取失败。",
  workspace_pdf_has_no_pages: "PDF 中没有可显示的页面。",
  workspace_pdf_page_out_of_range: "请求的 PDF 页面不存在。",
  workspace_pdf_renderer_unavailable: "应用内置的 PDF 渲染器不可用。",
  workspace_pdf_renderer_failed: "PDF 页面渲染失败。",
  workspace_pdf_render_unsupported: "当前平台不支持 PDF 预览。",
};

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
  | { phase: "ready-pdf"; descriptor: OpenPreviewResult; pageCount: number }
  | { phase: "ready-pdf-js"; descriptor: OpenPreviewResult }
  | { phase: "waiting-pdf-platform"; source: PreviewSource }
  | { phase: "unsupported-pdf-platform" }
  | { phase: "unsupported"; descriptor: OpenPreviewResult; application: WorkspaceApplication | null }
  | { phase: "unavailable"; descriptor: OpenPreviewResult }
  | { phase: "error"; source: PreviewSource; reason: string };

export interface FilePreviewPaneProps {
  workspaceId: string;
  workspaceKind: string;
  localRoot: string;
  hostPlatform: string | null;
  file: WorkspaceFileItem;
  view: PreviewView;
  onViewAvailabilityChange: (available: boolean) => void;
  /** 打开与基线的差异视图。两边都改过时这是唯一安全的下一步。 */
  onShowDiff: () => void;
  /** 该文件已有正式冲突记录。冲突横幅会说同一件事并带解决入口，状态条让位不重复。 */
  hasFormalConflict: boolean;
  /** 进入确认流程，把本地改动确认下来。 */
  onConfirm: () => void;
}

export function FilePreviewPane({ workspaceId, workspaceKind, localRoot, hostPlatform, file, view, onViewAvailabilityChange, onShowDiff, onConfirm, hasFormalConflict }: FilePreviewPaneProps) {
  const [retryRevision, setRetryRevision] = useState(0);
  const [applicationOpenError, setApplicationOpenError] = useState<string | null>(null);
  const localState = useFilePreview({ workspaceId, localFileId: file.localFileId, source: "local" }, localRoot, file.relativePath, hostPlatform, retryRevision, workspaceKind === "local", JSON.stringify([file.checksumAlgorithm,file.checksumValue]));
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

function PreviewBody({ state, onRetry, view, onOpenApplication, applicationOpenError, localRoot }: {
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
    if (state.descriptor.format === "pdf") return <WindowsPdfPreview descriptor={state.descriptor} localRoot="" bytes={state.bytes}/>;
    return <OfficeDocumentPreview bytes={state.bytes} format={state.descriptor.format as OfficePreviewFormat}/>;
  }
  if (state.phase === "loading") {
    return <PreviewMessage title={`正在读取${SOURCE_LABELS[state.source]}`} />;
  }
  if (state.phase === "waiting-pdf-platform") {
    return <PreviewMessage title="正在确认运行平台" />;
  }
  if (state.phase === "unsupported-pdf-platform") {
    return <PreviewMessage title="当前平台不支持 PDF 预览" />;
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
  if (state.phase === "ready-pdf") {
    return <PdfPreview descriptor={state.descriptor} localRoot={localRoot} pageCount={state.pageCount}/>;
  }
  if (state.phase === "ready-pdf-js") {
    return <WindowsPdfPreview descriptor={state.descriptor} localRoot={localRoot}/>;
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
  const { descriptor, content } = state;
  const fillsPreview = descriptor.format === "html" && view === "rendered";
  return (
    <div className={`preview-document${fillsPreview ? " preview-document-html" : ""}`}>
      <PreviewContent content={content} format={descriptor.format} view={view}/>
    </div>
  );
}

function PdfPreview({ descriptor, localRoot, pageCount }: { descriptor: OpenPreviewResult; localRoot: string; pageCount: number }) {
  const [pageNumber, setPageNumber] = useState(1);
  const [objectUrl, setObjectUrl] = useState<string | null>(null);
  const [renderError, setRenderError] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    let url: string | null = null;
    setObjectUrl(null);
    setRenderError(null);
    void invoke<ArrayBuffer | Uint8Array | number[]>("workspace_render_pdf_page", {
      localRoot,
      relativePath: descriptor.relativePath,
      pageNumber,
      maxWidth: 1600,
    }).then((bytes) => {
      if (disposed) return;
      url = URL.createObjectURL(new Blob([normalizeBinaryPayload(bytes)], { type: "image/png" }));
      setObjectUrl(url);
    }).catch((error: unknown) => {
      if (!disposed) setRenderError(extractReason(error));
    });
    return () => {
      disposed = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [descriptor.relativePath, localRoot, pageNumber]);
  return <div className="preview-pdf-document">
    <div className="preview-pdf-toolbar" aria-label="PDF 页面导航">
      <button disabled={pageNumber <= 1} onClick={() => setPageNumber((page) => page - 1)} type="button">上一页</button>
      <span>第 {pageNumber} / {pageCount} 页</span>
      <button disabled={pageNumber >= pageCount} onClick={() => setPageNumber((page) => page + 1)} type="button">下一页</button>
    </div>
    {renderError ? <PreviewMessage detail={ERROR_LABELS[renderError] ?? "PDF 页面渲染失败。"} title="无法显示 PDF"/> : null}
    {!renderError && objectUrl ? <div className="preview-pdf-page"><img alt={`PDF 第 ${pageNumber} 页`} src={objectUrl}/></div> : null}
    {!renderError && !objectUrl ? <div className="preview-pdf-loading">正在渲染第 {pageNumber} 页…</div> : null}
  </div>;
}

function WindowsPdfPreview({ descriptor, localRoot, bytes }: { descriptor: OpenPreviewResult; localRoot: string; bytes?: ArrayBuffer }) {
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
    void (bytes ? Promise.resolve(bytes) : invoke<ArrayBuffer | Uint8Array | number[]>("workspace_pdf_read_bytes", {
      localRoot,
      relativePath: descriptor.relativePath,
    })).then(async (payload) => {
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
  }, [bytes, descriptor.contentRevision, descriptor.relativePath, localRoot]);

  useEffect(() => {
    if (!document) return;
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
  }, [document, pageNumber]);

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
function decodeBase64(value: string): ArrayBuffer {
  const decoded = atob(value); const bytes = new Uint8Array(decoded.length);
  for (let i = 0; i < decoded.length; i += 1) bytes[i] = decoded.charCodeAt(i);
  return bytes.buffer;
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
  if (format === "html") {
    // 用户目录里的 HTML 是不可信输入，绝不能进主文档：主渲染进程持有宿主白名单通道，
    // 一段脚本可能借它读写本地文件。放进 sandbox 为空的内联框架里，脚本、
    // 表单、同源访问、跳转全部关闭，只剩排版和样式。应用自身的 CSP 是第二道拦截。
    return <iframe className="preview-html-frame" sandbox="" srcDoc={prepareScriptlessHtmlPreview(content)} title="网页预览"/>;
  }
  return <MarkdownContent diagramIdPrefix="preview">{content}</MarkdownContent>;
}

const SCRIPTLESS_PREVIEW_STYLE = `<style data-contexthub-static-preview>
  .reveal,
  [data-aos],
  [data-scroll-reveal] {
    opacity: 1 !important;
    visibility: visible !important;
    transform: none !important;
    transition: none !important;
  }
</style>`;

/**
 * 用户 HTML 的脚本在预览沙箱里不会执行，因此依赖滚动脚本解除隐藏的正文会永久透明。
 * 这里只改送入 iframe 的预览副本，让常见的滚动揭示容器静态可见；源文件和脚本权限都不变。
 */
export function prepareScriptlessHtmlPreview(content: string): string {
  const headEnd = content.search(/<\/head\s*>/i);
  if (headEnd >= 0) return `${content.slice(0, headEnd)}${SCRIPTLESS_PREVIEW_STYLE}\n${content.slice(headEnd)}`;
  const bodyStart = content.search(/<body\b/i);
  if (bodyStart >= 0) return `${content.slice(0, bodyStart)}${SCRIPTLESS_PREVIEW_STYLE}\n${content.slice(bodyStart)}`;
  return `${SCRIPTLESS_PREVIEW_STYLE}\n${content}`;
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
  const key = JSON.stringify([enabled,hostPlatform,localRoot,params.localFileId,params.source,params.workspaceId,relativePath,retryRevision,contentRevision]);
  const [state, setState] = useState<PreviewState>({ phase: "idle" });
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
      const next = await loadPreview(params, localRoot, relativePath, hostPlatform);
      if (!cancelled) { loadedKey.current = key; setState(next); }
    })().catch((error: unknown) => { if (!cancelled) { loadedKey.current = key; setState({ phase: "error", source: params.source, reason: extractReason(error) }); } });
    return cancelDownload;
  }, [enabled, hostPlatform, localRoot, params.localFileId, params.source, params.workspaceId, relativePath, retryRevision, contentRevision, pageActive, key]);
  return enabled && loadedKey.current !== key && !(state.phase === "hydrating" && loadingKey.current === key) ? {phase:"loading",source:params.source} : state;
}

async function loadPreview(params: OpenPreviewParams, localRoot: string, relativePath: string, hostPlatform: string | null): Promise<PreviewState> {
  const descriptor = await invoke<OpenPreviewResult>("preview_open", { params });
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
  if (["image", "docx", "pptx", "xlsx", "xmind"].includes(descriptor.format)) {
    const binary = await rpc<ReadBinaryPreviewResult>("preview/readBinary", { workspaceId: params.workspaceId, localFileId: params.localFileId });
    return { phase: "ready-binary", descriptor: { ...descriptor, contentRevision: binary.contentRevision, sizeBytes: binary.sizeBytes }, bytes: decodeBase64(binary.contentBase64), contentType: binary.contentType };
  }
  if (descriptor.format === "pdf") {
    if (hostPlatform === null) return { phase: "waiting-pdf-platform", source: params.source };
    if (hostPlatform === "macos") {
      const pageCount = await invoke<number>("workspace_pdf_page_count", { localRoot, relativePath });
      return { phase: "ready-pdf", descriptor, pageCount };
    }
    if (hostPlatform === "windows") return { phase: "ready-pdf-js", descriptor };
    return { phase: "unsupported-pdf-platform" };
  }
  if (!descriptor.ticketId || !descriptor.contentRevision) {
    throw new Error("preview_descriptor_invalid");
  }
  const chunks: string[] = [];
  let offset = 0;
  while (true) {
    const request: ReadTextResourceParams = {
      ticketId: descriptor.ticketId,
      offset,
      maxBytes: 64 * 1024,
    };
    const chunk = await invoke<ReadTextResourceResult>("text_resource_read", { params: request });
    if (chunk.contentRevision !== descriptor.contentRevision || chunk.offset !== offset) {
      throw new Error("preview_source_changed");
    }
    chunks.push(chunk.text);
    if (chunk.endOfResource) break;
    if (chunk.nextOffset <= offset) throw new Error("preview_read_failed");
    offset = chunk.nextOffset;
  }
  return { phase: "ready", descriptor, content: chunks.join("") };
}

function extractReason(error: unknown): string {
  const value = String(error);
  const known = Object.keys(ERROR_LABELS).find((reason) => value.includes(reason));
  return known ?? (value.includes("source_not_materialized") ? "preview_source_not_materialized" : "preview_read_failed");
}
