import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const renderMermaid = vi.hoisted(() => vi.fn().mockResolvedValue({ svg: '<svg data-testid="rendered-mermaid"></svg>' }));
const openPdfDocument = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({
  invoke,
}));
vi.mock("./app-server-rpc", async (importOriginal) => ({ ...await importOriginal<typeof import("./app-server-rpc")>(), rpc: async (method: string, params: object) => method === "file/hydrate" ? { state: "ready" } : invoke("app_server_request", { method, params }) }));
vi.mock("mermaid", () => ({ default: { initialize: vi.fn(), render: renderMermaid } }));
vi.mock("./pdfjs-renderer", () => ({ openPdfDocument }));

import { FilePreviewPane, type PreviewView } from "./preview";
import type { WorkspaceFileItem } from "./generated/protocol";

afterEach(() => { cleanup(); vi.clearAllMocks(); });

const file = { localFileId: "f1", relativePath: "a.md", fileType: "file", presenceState: "present",
  displayState: null, baselineRelation: "platform_absent", localChangeState: "clean",
  confirmationState: "not_required", conflictState: "none", stateRevision: 1,
  checksumAlgorithm: "sha256", checksumValue: "abc", baselineVersionId: null } as WorkspaceFileItem;

const BODY = "# 标题\n\n<b>粗体</b>";

const ready = (format: string) => {
  invoke.mockImplementation((command: string) => command === "preview_open"
    ? Promise.resolve({ source: "local", availability: "ready", format, relativePath: "a",
        contentRevision: "rev1", ticketId: "t1", byteLength: BODY.length, truncated: false })
    : Promise.resolve({ contentRevision: "rev1", offset: 0, text: BODY,
        nextOffset: BODY.length, endOfResource: true }));
};

const readyWith = (format: string, body: string) => {
  invoke.mockImplementation((command: string) => command === "preview_open"
    ? Promise.resolve({ source: "local", availability: "ready", format, relativePath: "a",
        contentRevision: "rev1", ticketId: "t1", byteLength: body.length, truncated: false })
    : Promise.resolve({ contentRevision: "rev1", offset: 0, text: body,
        nextOffset: body.length, endOfResource: true }));
};

const pane = (selectedFile = file, view: PreviewView = "rendered", onViewAvailabilityChange = () => {}, hostPlatform: string | null = "macos") => <FilePreviewPane workspaceId="ws" workspaceKind="local" localRoot="/workspace" hostPlatform={hostPlatform} file={selectedFile} view={view} onViewAvailabilityChange={onViewAvailabilityChange}/>;

describe("正文的预览与源码两种看法", () => {
  it("Markdown 默认给渲染后的样子，切到源码看文件里真正写的内容", async () => {
    ready("markdown");
    const rendered = render(pane());
    expect(await screen.findByRole("heading", { name: "标题" })).not.toBeNull();
    expect(rendered.container.querySelector(".preview-document-meta")).toBeNull();

    rendered.rerender(pane(file, "source"));
    expect(screen.queryByRole("heading", { name: "标题" })).toBeNull();
    expect(screen.getByText(/# 标题/)).not.toBeNull();
  });

  it("Markdown 渲染 GFM 表格和 Mermaid 图表", async () => {
    readyWith("markdown", "| 项目 | 状态 |\n| --- | --- |\n| 词来 | 完成 |\n\n```mermaid\ngraph LR\nA-->B\n```");
    render(pane());

    expect(await screen.findByRole("table")).not.toBeNull();
    expect(screen.getByRole("columnheader", { name: "项目" })).not.toBeNull();
    await waitFor(() => expect(screen.getByTestId("rendered-mermaid")).not.toBeNull());
    expect(renderMermaid).toHaveBeenCalledWith(expect.stringMatching(/^preview-mermaid-/), "graph LR\nA-->B");
  });

  it("文件状态轮询更新时保留代码、表格和 Mermaid 的节点及滚动位置", async () => {
    readyWith("markdown", "```text\n一段很长的代码内容\n```\n\n| 项目 | 状态 |\n| --- | --- |\n| 词来 | 完成 |\n\n```mermaid\ngraph LR\nA-->B\n```");
    const rendered = render(pane());
    await waitFor(() => expect(screen.getByTestId("rendered-mermaid")).not.toBeNull());
    const code = rendered.container.querySelector(".preview-document pre") as HTMLElement;
    const table = rendered.container.querySelector(".markdown-table-scroll") as HTMLElement;
    const diagram = rendered.container.querySelector(".mermaid-diagram") as HTMLElement;
    code.scrollLeft = 31;
    table.scrollLeft = 47;
    diagram.scrollLeft = 59;

    rendered.rerender(pane({ ...file, stateRevision: 2 } as WorkspaceFileItem));

    expect(rendered.container.querySelector(".preview-document pre")).toBe(code);
    expect(rendered.container.querySelector(".markdown-table-scroll")).toBe(table);
    expect(rendered.container.querySelector(".mermaid-diagram")).toBe(diagram);
    expect([code.scrollLeft, table.scrollLeft, diagram.scrollLeft]).toEqual([31, 47, 59]);
    expect(renderMermaid).toHaveBeenCalledTimes(1);
  });

  it("HTML 的渲染必须关在禁脚本的沙箱框架里，绝不进主文档", async () => {
    readyWith("html", '<html><head><style>.reveal{opacity:0}</style></head><body><main class="reveal">正文</main><script>window.ran=true</script></body></html>');
    render(pane());
    const frame = await screen.findByTitle("网页预览");
    expect(frame.tagName).toBe("IFRAME");
    expect(frame.parentElement?.classList.contains("preview-document-html")).toBe(true);
    expect(frame.parentElement?.querySelector(".preview-document-meta")).toBeNull();
    // sandbox 为空串代表全部能力关闭：脚本、表单、同源、跳转都不放行。
    // 这个 webview 拿得到 Tauri 调用通道，用户目录里的 HTML 一旦能跑脚本，
    // 就等于把本地文件的读写权交了出去。
    expect(frame.getAttribute("sandbox")).toBe("");
    const source = frame.getAttribute("srcdoc") ?? "";
    expect(source).toContain("data-contexthub-static-preview");
    expect(source.indexOf("data-contexthub-static-preview")).toBeGreaterThan(source.indexOf(".reveal{opacity:0}"));
    expect(source).toContain("window.ran=true");
  });

  it("纯文本没有渲染后的形态，不向头部报告可切换", async () => {
    ready("plain_text");
    const onViewAvailabilityChange = vi.fn();
    render(pane(file, "rendered", onViewAvailabilityChange));
    expect(await screen.findByText(/# 标题/)).not.toBeNull();
    expect(onViewAvailabilityChange).not.toHaveBeenCalledWith(true);
  });

  it("macOS PDF 保持由宿主安全渲染为页面并在内容区逐页查看", async () => {
    const createObjectUrl = vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:pdf-preview");
    const revokeObjectUrl = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
    const pdf = { ...file, localFileId: "pdf-1", relativePath: "合同.pdf" } as WorkspaceFileItem;
    invoke.mockImplementation((command: string) => {
      if (command === "preview_open") return Promise.resolve({ source: "local", availability: "ready", format: "pdf", relativePath: "合同.pdf", sizeBytes: 2048 });
      if (command === "workspace_pdf_page_count") return Promise.resolve(2);
      return Promise.resolve([137, 80, 78, 71]);
    });

    const rendered = render(pane(pdf));
    const page = await screen.findByAltText("PDF 第 1 页");
    expect(page.getAttribute("src")).toBe("blob:pdf-preview");
    expect(rendered.container.querySelector(".preview-document-meta")).toBeNull();
    expect(invoke).toHaveBeenCalledWith("workspace_pdf_page_count", { localRoot: "/workspace", relativePath: "合同.pdf" });
    expect(invoke).toHaveBeenCalledWith("workspace_render_pdf_page", {
      localRoot: "/workspace", maxWidth: 1600, pageNumber: 1, relativePath: "合同.pdf",
    });
    expect(invoke).not.toHaveBeenCalledWith("workspace_pdf_read_bytes", expect.anything());
    expect(openPdfDocument).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "下一页" }));
    expect(await screen.findByText("第 2 / 2 页")).not.toBeNull();
    rendered.unmount();
    expect(createObjectUrl).toHaveBeenCalled();
    const image = createObjectUrl.mock.calls[0]?.[0] as Blob;
    expect(image.size).toBe(4);
    expect(image.type).toBe("image/png");
    expect(revokeObjectUrl).toHaveBeenCalledWith("blob:pdf-preview");
  });

  it("Windows 只读取 PDF 字节并使用本地 PDF.js canvas 翻页", async () => {
    const pdf = { ...file, localFileId: "pdf-windows", relativePath: "合同.pdf" } as WorkspaceFileItem;
    const cancel = vi.fn();
    const destroy = vi.fn().mockResolvedValue(undefined);
    const renderPage = vi.fn(() => ({ promise: Promise.resolve(), cancel }));
    openPdfDocument.mockReturnValue({
      promise: Promise.resolve({ pageCount: 2, renderPage, destroy }),
      destroy,
    });
    invoke.mockImplementation((command: string) => {
      if (command === "preview_open") return Promise.resolve({ source: "local", availability: "ready", format: "pdf", relativePath: "合同.pdf", sizeBytes: 2048 });
      if (command === "workspace_pdf_read_bytes") return Promise.resolve(new Uint8Array([37, 80, 68, 70, 45]));
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });

    const rendered = render(pane(pdf, "rendered", () => {}, "windows"));
    expect(await screen.findByRole("img", { name: "PDF 第 1 页" })).not.toBeNull();
    expect(invoke).toHaveBeenCalledWith("workspace_pdf_read_bytes", {
      localRoot: "/workspace",
      relativePath: "合同.pdf",
    });
    expect(invoke).not.toHaveBeenCalledWith("workspace_pdf_page_count", expect.anything());
    expect(invoke).not.toHaveBeenCalledWith("workspace_render_pdf_page", expect.anything());
    expect(openPdfDocument).toHaveBeenCalledWith(expect.any(ArrayBuffer));
    expect(renderPage).toHaveBeenCalledWith(expect.any(HTMLCanvasElement), 1, 1600);

    fireEvent.click(screen.getByRole("button", { name: "下一页" }));
    expect(await screen.findByRole("img", { name: "PDF 第 2 页" })).not.toBeNull();
    expect(renderPage).toHaveBeenLastCalledWith(expect.any(HTMLCanvasElement), 2, 1600);
    rendered.unmount();
    expect(cancel).toHaveBeenCalled();
    expect(destroy).toHaveBeenCalled();
  });

  it("平台未确认时等待，未知平台明确拒绝且不进入任何 PDF 渲染路径", async () => {
    const pdf = { ...file, localFileId: "pdf-other", relativePath: "合同.pdf" } as WorkspaceFileItem;
    invoke.mockImplementation((command: string) => command === "preview_open"
      ? Promise.resolve({ source: "local", availability: "ready", format: "pdf", relativePath: "合同.pdf", sizeBytes: 2048 })
      : Promise.reject(new Error(`unexpected command: ${command}`)));

    const rendered = render(pane(pdf, "rendered", () => {}, null));
    expect(await screen.findByText("正在确认运行平台")).not.toBeNull();
    expect(invoke.mock.calls.some(([command]) => String(command).startsWith("workspace_pdf_"))).toBe(false);
    expect(openPdfDocument).not.toHaveBeenCalled();

    rendered.rerender(pane(pdf, "rendered", () => {}, "other"));
    expect(await screen.findByText("当前平台不支持 PDF 预览")).not.toBeNull();
    expect(invoke.mock.calls.some(([command]) => String(command).startsWith("workspace_pdf_"))).toBe(false);
    expect(openPdfDocument).not.toHaveBeenCalled();
  });

  it("办公文件只显示系统默认应用并交给系统打开", async () => {
    const docx = { ...file, localFileId: "docx-1", relativePath: "方案.docx" } as WorkspaceFileItem;
    invoke.mockImplementation((command: string) => {
      if (command === "preview_open") return Promise.resolve({ source: "local", availability: "unsupported", format: "unsupported", relativePath: "方案.docx" });
      if (command === "workspace_default_application") return Promise.resolve({ displayName: "Microsoft Word" });
      return Promise.resolve();
    });

    render(pane(docx));
    const word = await screen.findByRole("button", { name: /Microsoft Word/ });
    expect(screen.queryByText("Pages")).toBeNull();
    expect(screen.queryByText("选择应用打开")).toBeNull();
    fireEvent.click(word);
    expect(invoke).toHaveBeenCalledWith("workspace_open_item", {
      localRoot: "/workspace",
      openMode: "default",
      relativePath: "方案.docx",
    });
  });

  it("系统没有默认应用时只显示无可用软件提示", async () => {
    const docx = { ...file, localFileId: "docx-2", relativePath: "方案.docx" } as WorkspaceFileItem;
    invoke.mockImplementation((command: string) => {
      if (command === "preview_open") return Promise.resolve({ source: "local", availability: "unsupported", format: "unsupported", relativePath: "方案.docx" });
      if (command === "workspace_default_application") return Promise.resolve(null);
      return Promise.resolve();
    });

    render(pane(docx));
    expect(await screen.findByText("没有可以打开此文件的软件")).not.toBeNull();
  });
});
