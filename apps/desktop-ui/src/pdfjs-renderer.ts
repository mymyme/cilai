import { PDFWorker, getDocument, type PDFDocumentLoadingTask, type PDFDocumentProxy, type RenderTask } from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

const MIN_RENDER_WIDTH = 320;
const MAX_RENDER_WIDTH = 2400;

export interface PdfJsRenderTask {
  promise: Promise<void>;
  cancel: () => void;
}

export interface PdfJsDocument {
  pageCount: number;
  renderPage: (canvas: HTMLCanvasElement, pageNumber: number, maxWidth: number) => PdfJsRenderTask;
  destroy: () => Promise<void>;
}

export interface PdfJsLoadingTask {
  promise: Promise<PdfJsDocument>;
  destroy: () => Promise<void>;
}

export function openPdfDocument(bytes: ArrayBuffer): PdfJsLoadingTask {
  if (bytes.byteLength === 0) throw pdfError("workspace_pdf_document_invalid");
  if (typeof Worker !== "function") throw pdfError("workspace_pdf_renderer_unavailable");

  let browserWorker: Worker;
  try {
    browserWorker = new Worker(pdfWorkerUrl, { type: "module" });
  } catch (error) {
    throw pdfError("workspace_pdf_renderer_unavailable", error);
  }

  let pdfWorker: PDFWorker;
  try {
    pdfWorker = PDFWorker.create({ port: browserWorker });
  } catch (error) {
    terminateWorker(browserWorker);
    throw pdfError("workspace_pdf_renderer_unavailable", error);
  }
  let loadingTask: PDFDocumentLoadingTask;
  try {
    const documentOptions = {
      data: new Uint8Array(bytes),
      worker: pdfWorker,
      cMapUrl: localAssetUrl("pdfjs/cmaps/"),
      cMapPacked: true,
      standardFontDataUrl: localAssetUrl("pdfjs/standard_fonts/"),
      isEvalSupported: false,
      useWorkerFetch: false,
      useWasm: false,
      wasmUrl: localAssetUrl("pdfjs/wasm/"),
    };
    loadingTask = getDocument(documentOptions);
  } catch (error) {
    releaseWorkers(pdfWorker, browserWorker);
    throw pdfError("workspace_pdf_document_invalid", error);
  }

  let destroyed = false;
  let workerFailed = false;
  let workersReleased = false;
  let loadingDestroyPromise: Promise<void> | null = null;
  let destroyPromise: Promise<void> | null = null;
  let rejectWorkerFailure: ((reason: Error) => void) | null = null;
  const workerFailure = new Promise<never>((_resolve, reject) => {
    rejectWorkerFailure = reject;
  });
  const removeWorkerListeners = () => {
    browserWorker.removeEventListener("error", onWorkerError);
    browserWorker.removeEventListener("messageerror", onWorkerError);
  };
  const release = () => {
    if (workersReleased) return;
    workersReleased = true;
    releaseWorkers(pdfWorker, browserWorker);
  };
  const startLoadingDestroy = (): Promise<void> => {
    if (loadingDestroyPromise) return loadingDestroyPromise;
    try {
      loadingDestroyPromise = Promise.resolve(loadingTask.destroy()).catch(() => {});
    } catch {
      loadingDestroyPromise = Promise.resolve();
    }
    return loadingDestroyPromise;
  };
  const destroy = (): Promise<void> => {
    if (destroyPromise) return destroyPromise;
    destroyed = true;
    rejectWorkerFailure?.(pdfError("workspace_pdf_render_cancelled"));
    destroyPromise = Promise.resolve().then(() => {
      void startLoadingDestroy();
      removeWorkerListeners();
      release();
    });
    return destroyPromise;
  };
  const onWorkerError = () => {
    if (workerFailed) return;
    workerFailed = true;
    destroyed = true;
    release();
    rejectWorkerFailure?.(pdfError("workspace_pdf_renderer_unavailable"));
    void startLoadingDestroy();
  };
  browserWorker.addEventListener("error", onWorkerError, { once: true });
  browserWorker.addEventListener("messageerror", onWorkerError, { once: true });

  const promise = Promise.race([loadingTask.promise, workerFailure])
    .then((document) => {
      if (destroyed) throw pdfError("workspace_pdf_render_cancelled");
      if (!Number.isSafeInteger(document.numPages) || document.numPages < 1) {
        throw pdfError("workspace_pdf_has_no_pages");
      }
      return createDocument(document, destroy, () => destroyed, workerFailure);
    })
    .catch(async (error: unknown) => {
      const reason = errorReason(error);
      await destroy();
      if (reason === "workspace_pdf_render_cancelled" || reason === "workspace_pdf_has_no_pages" || reason === "workspace_pdf_renderer_unavailable") {
        throw pdfError(reason, error);
      }
      throw pdfError("workspace_pdf_document_invalid", error);
    });

  return { promise, destroy };
}

function createDocument(
  document: PDFDocumentProxy,
  destroy: () => Promise<void>,
  isDestroyed: () => boolean,
  workerFailure: Promise<never>,
): PdfJsDocument {
  return {
    pageCount: document.numPages,
    renderPage: (canvas, pageNumber, maxWidth) => renderPage(document, canvas, pageNumber, maxWidth, isDestroyed, workerFailure),
    destroy,
  };
}

function renderPage(
  document: PDFDocumentProxy,
  canvas: HTMLCanvasElement,
  pageNumber: number,
  maxWidth: number,
  isDestroyed: () => boolean,
  workerFailure: Promise<never>,
): PdfJsRenderTask {
  let cancelled = false;
  let activeTask: RenderTask | null = null;
  const cancel = () => {
    cancelled = true;
    try {
      activeTask?.cancel();
    } catch {}
  };
  const promise = (async () => {
    if (!Number.isSafeInteger(pageNumber) || pageNumber < 1 || pageNumber > document.numPages) {
      throw pdfError("workspace_pdf_page_out_of_range");
    }
    let page;
    try {
      page = await Promise.race([document.getPage(pageNumber), workerFailure]);
    } catch (error) {
      const reason = errorReason(error);
      if (reason === "workspace_pdf_renderer_unavailable") {
        throw pdfError("workspace_pdf_renderer_unavailable", error);
      }
      if (reason === "workspace_pdf_render_cancelled" || isDestroyed()) {
        throw pdfError("workspace_pdf_render_cancelled", error);
      }
      throw pdfError("workspace_pdf_page_unavailable", error);
    }
    try {
      if (cancelled || isDestroyed()) throw pdfError("workspace_pdf_render_cancelled");

      let initialViewport;
      try {
        initialViewport = page.getViewport({ scale: 1 });
      } catch (error) {
        throw pdfError("workspace_pdf_render_buffer_invalid", error);
      }
      if (!Number.isFinite(initialViewport.width) || initialViewport.width <= 0 || !Number.isFinite(maxWidth)) {
        throw pdfError("workspace_pdf_render_buffer_invalid");
      }
      const targetWidth = Math.min(MAX_RENDER_WIDTH, Math.max(MIN_RENDER_WIDTH, Math.trunc(maxWidth)));
      const scale = targetWidth / initialViewport.width;
      let viewport;
      try {
        viewport = page.getViewport({ scale });
      } catch (error) {
        throw pdfError("workspace_pdf_render_buffer_invalid", error);
      }
      if (!Number.isFinite(viewport.width) || !Number.isFinite(viewport.height) || viewport.width <= 0 || viewport.height <= 0) {
        throw pdfError("workspace_pdf_render_buffer_invalid");
      }
      try {
        canvas.width = Math.ceil(viewport.width);
        canvas.height = Math.ceil(viewport.height);
        if (!canvas.getContext("2d")) throw pdfError("workspace_pdf_render_buffer_invalid");
      } catch (error) {
        throw pdfError("workspace_pdf_render_buffer_invalid", error);
      }

      try {
        activeTask = page.render({ canvas, viewport, background: "#fff" });
        await Promise.race([activeTask.promise, workerFailure]);
      } catch (error) {
        if (errorReason(error) === "workspace_pdf_renderer_unavailable") {
          try {
            activeTask?.cancel();
          } catch {}
          throw pdfError("workspace_pdf_renderer_unavailable", error);
        }
        if (cancelled || isDestroyed()) {
          try {
            activeTask?.cancel();
          } catch {}
          throw pdfError("workspace_pdf_render_cancelled", error);
        }
        throw pdfError("workspace_pdf_renderer_failed", error);
      }
    } finally {
      try {
        page.cleanup();
      } catch {}
    }
  })();
  return { promise, cancel };
}

function localAssetUrl(path: string): string {
  return new URL(path, document.baseURI).toString();
}

function releaseWorkers(pdfWorker: PDFWorker, browserWorker: Worker): void {
  try {
    pdfWorker.destroy();
  } catch {}
  terminateWorker(browserWorker);
}

function terminateWorker(browserWorker: Worker): void {
  try {
    browserWorker.terminate();
  } catch {}
}

function pdfError(reason: string, cause?: unknown): Error {
  return cause === undefined ? new Error(reason) : new Error(reason, { cause });
}

function errorReason(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
