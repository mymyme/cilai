import { afterEach, describe, expect, it, vi } from "vitest";

const getDocument = vi.hoisted(() => vi.fn());
const createPdfWorker = vi.hoisted(() => vi.fn());
vi.mock("pdfjs-dist", () => ({
  getDocument,
  PDFWorker: { create: createPdfWorker },
}));
vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({ default: "/assets/pdf.worker.min.mjs" }));

import { openPdfDocument } from "./pdfjs-renderer";

class WorkerStub {
  static instances: WorkerStub[] = [];
  readonly terminate = vi.fn();
  private errorListener: EventListenerOrEventListenerObject | null = null;
  readonly addEventListener = vi.fn((type: string, listener: EventListenerOrEventListenerObject) => {
    if (type === "error") this.errorListener = listener;
  });
  readonly removeEventListener = vi.fn((type: string, listener: EventListenerOrEventListenerObject) => {
    if (type === "error" && this.errorListener === listener) this.errorListener = null;
  });

  constructor(readonly url: string | URL, readonly options?: WorkerOptions) {
    WorkerStub.instances.push(this);
  }

  emitError() {
    if (typeof this.errorListener === "function") {
      this.errorListener(new Event("error"));
    } else {
      this.errorListener?.handleEvent(new Event("error"));
    }
  }
}

afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  WorkerStub.instances = [];
});

describe("本地 PDF.js 渲染器", () => {
  it("只把字节交给显式 Web Worker，并关闭 WASM 与 worker 网络取件", async () => {
    vi.stubGlobal("Worker", WorkerStub);
    const pdfWorker = { destroy: vi.fn() };
    const loadingDestroy = vi.fn().mockResolvedValue(undefined);
    const pageCleanup = vi.fn();
    const nativeCancel = vi.fn();
    const nativeRender = { promise: Promise.resolve(), cancel: nativeCancel };
    const page = {
      getViewport: vi.fn(({ scale }: { scale: number }) => ({ width: 800 * scale, height: 1200 * scale })),
      render: vi.fn(() => nativeRender),
      cleanup: pageCleanup,
    };
    const documentProxy = { numPages: 2, getPage: vi.fn().mockResolvedValue(page) };
    createPdfWorker.mockReturnValue(pdfWorker);
    getDocument.mockReturnValue({ promise: Promise.resolve(documentProxy), destroy: loadingDestroy });

    const loading = openPdfDocument(new Uint8Array([1, 2, 3]).buffer);
    const pdfDocument = await loading.promise;
    expect(WorkerStub.instances).toHaveLength(1);
    expect(WorkerStub.instances[0]?.url).toBe("/assets/pdf.worker.min.mjs");
    expect(WorkerStub.instances[0]?.options).toEqual({ type: "module" });
    expect(createPdfWorker).toHaveBeenCalledWith({ port: WorkerStub.instances[0] });
    expect(getDocument).toHaveBeenCalledWith(expect.objectContaining({
      data: expect.any(Uint8Array),
      worker: pdfWorker,
      cMapPacked: true,
      isEvalSupported: false,
      useWorkerFetch: false,
      useWasm: false,
    }));
    const options = getDocument.mock.calls[0]?.[0];
    expect(new URL(options.cMapUrl).origin).toBe(window.location.origin);
    expect(new URL(options.standardFontDataUrl).origin).toBe(window.location.origin);
    expect(new URL(options.wasmUrl).origin).toBe(window.location.origin);

    const canvas = globalThis.document.createElement("canvas");
    vi.spyOn(canvas, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const rendering = pdfDocument.renderPage(canvas, 1, 1600);
    await rendering.promise;
    expect(documentProxy.getPage).toHaveBeenCalledWith(1);
    expect(page.render).toHaveBeenCalledWith(expect.objectContaining({ canvas, background: "#fff" }));
    expect(canvas.width).toBe(1600);
    expect(canvas.height).toBe(2400);
    expect(pageCleanup).toHaveBeenCalled();

    rendering.cancel();
    expect(nativeCancel).toHaveBeenCalled();
    await pdfDocument.destroy();
    expect(loadingDestroy).toHaveBeenCalled();
    expect(pdfWorker.destroy).toHaveBeenCalled();
    expect(WorkerStub.instances[0]?.terminate).toHaveBeenCalled();
  });

  it("没有 Web Worker 时明确拒绝，不进入 PDF.js 假 worker", () => {
    vi.stubGlobal("Worker", undefined);
    expect(() => openPdfDocument(new Uint8Array([1]).buffer)).toThrow("workspace_pdf_renderer_unavailable");
    expect(createPdfWorker).not.toHaveBeenCalled();
    expect(getDocument).not.toHaveBeenCalled();
  });

  it("PDFWorker 创建失败时终止已创建的浏览器 Worker", () => {
    vi.stubGlobal("Worker", WorkerStub);
    createPdfWorker.mockImplementation(() => { throw new Error("worker setup failed"); });

    expect(() => openPdfDocument(new Uint8Array([1]).buffer)).toThrow("workspace_pdf_renderer_unavailable");
    expect(WorkerStub.instances[0]?.terminate).toHaveBeenCalled();
    expect(getDocument).not.toHaveBeenCalled();
  });

  it("Worker 运行失败时不等待卡死的 loading task 就释放两层 Worker 并报错", async () => {
    vi.stubGlobal("Worker", WorkerStub);
    const pdfWorker = { destroy: vi.fn() };
    const loadingDestroy = vi.fn().mockReturnValue(new Promise(() => {}));
    createPdfWorker.mockReturnValue(pdfWorker);
    getDocument.mockReturnValue({ promise: new Promise(() => {}), destroy: loadingDestroy });

    const loading = openPdfDocument(new Uint8Array([1]).buffer);
    WorkerStub.instances[0]?.emitError();

    await expect(loading.promise).rejects.toThrow("workspace_pdf_renderer_unavailable");
    expect(loadingDestroy).toHaveBeenCalled();
    expect(pdfWorker.destroy).toHaveBeenCalled();
    expect(WorkerStub.instances[0]?.terminate).toHaveBeenCalled();
  });

  it("文档已加载后 Worker 运行失败仍释放所有资源", async () => {
    vi.stubGlobal("Worker", WorkerStub);
    const pdfWorker = { destroy: vi.fn() };
    const loadingDestroy = vi.fn().mockResolvedValue(undefined);
    createPdfWorker.mockReturnValue(pdfWorker);
    getDocument.mockReturnValue({
      promise: Promise.resolve({ numPages: 1, getPage: vi.fn() }),
      destroy: loadingDestroy,
    });

    const loading = openPdfDocument(new Uint8Array([1]).buffer);
    await loading.promise;
    WorkerStub.instances[0]?.emitError();
    await loading.destroy();

    expect(loadingDestroy).toHaveBeenCalledTimes(1);
    expect(pdfWorker.destroy).toHaveBeenCalledTimes(1);
    expect(WorkerStub.instances[0]?.terminate).toHaveBeenCalledTimes(1);
  });

  it("正常销毁不等待无响应的 loading task 就释放两层 Worker", async () => {
    vi.stubGlobal("Worker", WorkerStub);
    const pdfWorker = { destroy: vi.fn() };
    const loadingDestroy = vi.fn().mockReturnValue(new Promise(() => {}));
    createPdfWorker.mockReturnValue(pdfWorker);
    getDocument.mockReturnValue({ promise: new Promise(() => {}), destroy: loadingDestroy });

    const loading = openPdfDocument(new Uint8Array([1]).buffer);
    const loadingResult = expect(loading.promise).rejects.toThrow("workspace_pdf_render_cancelled");
    const destroying = loading.destroy();

    await destroying;
    await loadingResult;
    expect(loadingDestroy).toHaveBeenCalledTimes(1);
    expect(pdfWorker.destroy).toHaveBeenCalledTimes(1);
    expect(WorkerStub.instances[0]?.terminate).toHaveBeenCalledTimes(1);
  });

  it("文档已加载但页面读取卡住时 Worker 崩溃会结束渲染", async () => {
    vi.stubGlobal("Worker", WorkerStub);
    const pdfWorker = { destroy: vi.fn() };
    const loadingDestroy = vi.fn().mockReturnValue(new Promise(() => {}));
    const getPage = vi.fn().mockReturnValue(new Promise(() => {}));
    createPdfWorker.mockReturnValue(pdfWorker);
    getDocument.mockReturnValue({
      promise: Promise.resolve({ numPages: 1, getPage }),
      destroy: loadingDestroy,
    });

    const loading = openPdfDocument(new Uint8Array([1]).buffer);
    const pdfDocument = await loading.promise;
    const canvas = globalThis.document.createElement("canvas");
    const rendering = pdfDocument.renderPage(canvas, 1, 800);
    WorkerStub.instances[0]?.emitError();

    await expect(rendering.promise).rejects.toThrow("workspace_pdf_renderer_unavailable");
    expect(pdfWorker.destroy).toHaveBeenCalledTimes(1);
    expect(WorkerStub.instances[0]?.terminate).toHaveBeenCalledTimes(1);
  });

  it("页面渲染卡住时 Worker 崩溃会结束渲染", async () => {
    vi.stubGlobal("Worker", WorkerStub);
    const pdfWorker = { destroy: vi.fn() };
    const loadingDestroy = vi.fn().mockReturnValue(new Promise(() => {}));
    const nativeRender = { promise: new Promise(() => {}), cancel: vi.fn() };
    const page = {
      getViewport: vi.fn(({ scale }: { scale: number }) => ({ width: 800 * scale, height: 1200 * scale })),
      render: vi.fn(() => nativeRender),
      cleanup: vi.fn(),
    };
    createPdfWorker.mockReturnValue(pdfWorker);
    getDocument.mockReturnValue({
      promise: Promise.resolve({ numPages: 1, getPage: vi.fn().mockResolvedValue(page) }),
      destroy: loadingDestroy,
    });

    const loading = openPdfDocument(new Uint8Array([1]).buffer);
    const pdfDocument = await loading.promise;
    const canvas = globalThis.document.createElement("canvas");
    vi.spyOn(canvas, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const rendering = pdfDocument.renderPage(canvas, 1, 800);
    await vi.waitFor(() => expect(page.render).toHaveBeenCalledTimes(1));
    WorkerStub.instances[0]?.emitError();

    await expect(rendering.promise).rejects.toThrow("workspace_pdf_renderer_unavailable");
    expect(nativeRender.cancel).toHaveBeenCalledTimes(1);
    expect(page.cleanup).toHaveBeenCalledTimes(1);
    expect(pdfWorker.destroy).toHaveBeenCalledTimes(1);
    expect(WorkerStub.instances[0]?.terminate).toHaveBeenCalledTimes(1);
  });

  it("文档销毁会取消无响应的已创建渲染任务", async () => {
    vi.stubGlobal("Worker", WorkerStub);
    const pdfWorker = { destroy: vi.fn() };
    const loadingDestroy = vi.fn().mockReturnValue(new Promise(() => {}));
    const nativeRender = { promise: new Promise(() => {}), cancel: vi.fn() };
    const page = {
      getViewport: vi.fn(({ scale }: { scale: number }) => ({ width: 800 * scale, height: 1200 * scale })),
      render: vi.fn(() => nativeRender),
      cleanup: vi.fn(),
    };
    createPdfWorker.mockReturnValue(pdfWorker);
    getDocument.mockReturnValue({
      promise: Promise.resolve({ numPages: 1, getPage: vi.fn().mockResolvedValue(page) }),
      destroy: loadingDestroy,
    });

    const loading = openPdfDocument(new Uint8Array([1]).buffer);
    const pdfDocument = await loading.promise;
    const canvas = globalThis.document.createElement("canvas");
    vi.spyOn(canvas, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const rendering = pdfDocument.renderPage(canvas, 1, 800);
    await vi.waitFor(() => expect(page.render).toHaveBeenCalledTimes(1));

    await pdfDocument.destroy();

    await expect(rendering.promise).rejects.toThrow("workspace_pdf_render_cancelled");
    expect(nativeRender.cancel).toHaveBeenCalledTimes(1);
    expect(page.cleanup).toHaveBeenCalledTimes(1);
    expect(pdfWorker.destroy).toHaveBeenCalledTimes(1);
    expect(WorkerStub.instances[0]?.terminate).toHaveBeenCalledTimes(1);
  });

  it("文档销毁后迟到的页面不会进入渲染", async () => {
    vi.stubGlobal("Worker", WorkerStub);
    const pdfWorker = { destroy: vi.fn() };
    const loadingDestroy = vi.fn().mockResolvedValue(undefined);
    const page = {
      getViewport: vi.fn(),
      render: vi.fn(),
      cleanup: vi.fn(),
    };
    let resolvePage!: (value: typeof page) => void;
    const pagePromise = new Promise<typeof page>((resolve) => {
      resolvePage = resolve;
    });
    createPdfWorker.mockReturnValue(pdfWorker);
    getDocument.mockReturnValue({
      promise: Promise.resolve({ numPages: 1, getPage: vi.fn().mockReturnValue(pagePromise) }),
      destroy: loadingDestroy,
    });

    const loading = openPdfDocument(new Uint8Array([1]).buffer);
    const pdfDocument = await loading.promise;
    const canvas = globalThis.document.createElement("canvas");
    const rendering = pdfDocument.renderPage(canvas, 1, 800);

    await pdfDocument.destroy();
    resolvePage(page);

    await expect(rendering.promise).rejects.toThrow("workspace_pdf_render_cancelled");
    expect(page.render).not.toHaveBeenCalled();
  });

  it("页面切换取消后迟到的页面不会进入渲染", async () => {
    vi.stubGlobal("Worker", WorkerStub);
    const page = {
      getViewport: vi.fn(),
      render: vi.fn(),
      cleanup: vi.fn(),
    };
    let resolvePage!: (value: typeof page) => void;
    const pagePromise = new Promise<typeof page>((resolve) => {
      resolvePage = resolve;
    });
    createPdfWorker.mockReturnValue({ destroy: vi.fn() });
    getDocument.mockReturnValue({
      promise: Promise.resolve({ numPages: 1, getPage: vi.fn().mockReturnValue(pagePromise) }),
      destroy: vi.fn().mockResolvedValue(undefined),
    });

    const pdfDocument = await openPdfDocument(new Uint8Array([1]).buffer).promise;
    const rendering = pdfDocument.renderPage(globalThis.document.createElement("canvas"), 1, 800);
    rendering.cancel();
    resolvePage(page);

    await expect(rendering.promise).rejects.toThrow("workspace_pdf_render_cancelled");
    expect(page.render).not.toHaveBeenCalled();
    await pdfDocument.destroy();
  });
});
