import { Children, Fragment, isValidElement, memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { jsx, jsxs } from "react/jsx-runtime";
import { toJsxRuntime } from "hast-util-to-jsx-runtime";
import type { Components } from "react-markdown";
import type { MarkdownPage } from "./file-markdown-model";
import type { FileMarkdownRequest, FileMarkdownResponse } from "./file-markdown-protocol";
import { MermaidDiagram } from "./markdown-content";
import { usePageActive } from "./page-visibility";
import { writeClipboard } from "./clipboard";

const components: Components = {
  a: ({ children }) => <span className="markdown-link">{children}</span>,
  img: ({ alt }) => <span className="markdown-image-placeholder">[图片：{alt ?? "无说明"}]</span>,
  pre: ({ children }) => {
    const child = Children.toArray(children)[0];
    if (isValidElement<{ className?: string; children?: ReactNode }>(child) && child.props.className?.split(" ").includes("language-mermaid")) {
      return <MermaidDiagram idPrefix="preview" source={Children.toArray(child.props.children).join("").replace(/\n$/, "")}/>;
    }
    return <pre>{children}</pre>;
  },
  table: ({ children }) => <div className="markdown-table-scroll"><table>{children}</table></div>,
};
type View = { scope: object; content: string; page: MarkdownPage | null; loading: boolean; error: string | null };
type CopyState = { content: string; state: "idle" | "copying" | "done" | "error" };
export const FILE_MARKDOWN_TIMEOUT_MS = 30_000;

export const FileMarkdownPreview = memo(function FileMarkdownPreview({ content }: { content: string }) {
  const active = usePageActive();
  const [attempt, setAttempt] = useState(0);
  const scope = useMemo(() => ({ content, attempt }), [content, attempt]);
  const [view, setView] = useState<View>({ scope, content, page: null, loading: true, error: null });
  const [requestedPage, setRequestedPage] = useState("1");
  const [copy, setCopy] = useState<CopyState>({ content, state: "idle" });
  const copyGeneration = useRef(0);
  const requestId = useRef(0);
  const frame = useRef<HTMLDivElement>(null);
  const channel = useRef<{ worker: Worker; content: string; scope: object; pending: boolean;
    send: (request: FileMarkdownRequest) => void; close: () => void } | null>(null);
  const activeRef = useRef(active); activeRef.current = active;
  const viewRef = useRef(view); viewRef.current = view;
  const terminal = useRef<object | null>(null);
  const current = view.scope === scope ? view : null;
  const page = current?.page ?? null;

  useEffect(() => {
    setView({ scope, content, page: null, loading: true, error: null }); setRequestedPage("1"); terminal.current = null;
    return () => { if (channel.current?.scope === scope) channel.current.close(); };
  }, [scope]);
  useEffect(() => {
    const previous = channel.current;
    if (!active) {
      if (previous?.scope === scope && previous.pending) {
        previous.close();
        setView(current => current.scope === scope ? { ...current, loading: false } : current);
      }
      return;
    }
    if (previous?.scope === scope || terminal.current === scope) return;
    let disposed = false; let timer: ReturnType<typeof setTimeout> | undefined; let worker: Worker | null = null;
    let expectedIndex = 0; let rebuilding = true;
    const saved = viewRef.current.scope === scope ? viewRef.current.page : null;
    const target = saved?.index ?? 0; const expand = Boolean(saved?.oversize && saved.tree);
    setView(current => current.scope === scope ? { ...current, loading: true, error: null } : { scope, content, page: null, loading: true, error: null });
    const close = () => {
      disposed = true; clearTimeout(timer); worker?.terminate();
      if (channel.current?.worker === worker) channel.current = null;
    };
    const fail = (message: string) => {
      if (disposed) return;
      terminal.current = scope; close();
      setView({ scope, content, page: null, loading: false, error: message });
    };
    try {
      worker = new Worker(new URL("./file-markdown-worker.ts", import.meta.url), { type: "module" });
      const send = (request: FileMarkdownRequest) => {
        if (disposed || !activeRef.current) return;
        expectedIndex = request.type === "page" ? request.index : 0;
        if (channel.current?.worker === worker) channel.current.pending = true;
        clearTimeout(timer); timer = setTimeout(() => fail("文档解析超时，请重试。"), FILE_MARKDOWN_TIMEOUT_MS);
        try { worker!.postMessage(request); } catch { fail("文档解析失败，请重试。"); }
      };
      channel.current = { worker, content, scope, pending: true, send, close };
      worker.onmessage = (event: MessageEvent<FileMarkdownResponse>) => {
        if (disposed) return;
        if (!event.data || typeof event.data.requestId !== "number") { fail("文档读取结果无效，请重试。"); return; }
        if (event.data.requestId !== requestId.current) return;
        clearTimeout(timer);
        if (event.data.type !== "page") { fail("文档解析失败，请重试。"); return; }
        const received = event.data.page;
        if (!received || received.index !== expectedIndex || !Number.isInteger(received.pageCount) || received.pageCount <= received.index
          || received.tree && (received.tree.type !== "root" || !Array.isArray(received.tree.children)) || !received.tree && !received.oversize) {
          fail("文档读取结果无效，请重试。"); return;
        }
        if (rebuilding && (target !== 0 || expand)) {
          rebuilding = false;
          if (target >= received.pageCount) { fail("文档读取结果无效，请重试。"); return; }
          send({ type: "page", requestId: ++requestId.current, index: target, expand }); return;
        }
        rebuilding = false;
        if (channel.current?.worker === worker) channel.current.pending = false;
        setView({ scope, content, page: received, loading: false, error: null }); setRequestedPage(String(received.index + 1));
      };
      worker.onerror = () => fail("文档解析失败，请重试。");
      worker.onmessageerror = () => fail("文档读取结果无效，请重试。");
      send({ type: "load", requestId: ++requestId.current, content });
    } catch { fail("文档解析服务无法启动，请重试。"); }
  }, [content, active, attempt, scope]);

  useEffect(() => {
    setCopy({ content, state: "idle" });
    return () => { copyGeneration.current += 1; };
  }, [content, active]);
  useLayoutEffect(() => {
    const container = frame.current?.closest(".preview-document");
    if (container) { container.scrollTop = 0; container.scrollLeft = 0; }
  }, [content, page?.index, Boolean(page?.tree)]);

  const rendered = useMemo(() => {
    if (!page?.tree) return { node: null, error: false };
    try { return { node: toJsxRuntime(page.tree, { Fragment, components, jsx, jsxs, passKeys: true, passNode: true, ignoreInvalidStyle: true }), error: false }; }
    catch { return { node: null, error: true }; }
  }, [page?.tree]);
  useEffect(() => {
    if (rendered.error) { terminal.current = scope; channel.current?.close(); }
  }, [rendered.error, scope]);
  const selectPage = (index: number, expand = false) => {
    const currentChannel = channel.current;
    if (!activeRef.current || !page || !currentChannel || currentChannel.content !== content || !Number.isInteger(index) || index < 0 || index >= page.pageCount) return;
    setView({ scope, content, page, loading: true, error: null });
    currentChannel.send({ type: "page", requestId: ++requestId.current, index, expand });
  };
  const copyAll = async () => {
    if (!activeRef.current) return;
    const generation = ++copyGeneration.current;
    setCopy({ content, state: "copying" });
    try { await writeClipboard(content); if (generation === copyGeneration.current) setCopy({ content, state: "done" }); }
    catch { if (generation === copyGeneration.current) setCopy({ content, state: "error" }); }
  };
  const error = current?.error ?? (rendered.error ? "文档呈现失败，请重试。" : null);
  if (error) return <div className="preview-message" role="alert"><p>{error}</p><button onClick={() => setAttempt(value => value + 1)} type="button">重试解析</button></div>;
  if (!page) return <div className="preview-message" role="status">正在解析文档</div>;
  const copyStatus = copy.content === content ? copy.state : "idle";
  return <div className="preview-markdown-document" ref={frame}>
    <nav className="preview-text-pagination" aria-label="文档分页">
      {page.pageCount > 1 ? <>
        <div className="preview-text-page-buttons">
          <button disabled={current?.loading || page.index === 0} onClick={() => selectPage(0)} type="button">首页</button>
          <button disabled={current?.loading || page.index === 0} onClick={() => selectPage(page.index - 1)} type="button">上一页</button>
          <span aria-live="polite">第 {page.index + 1} / {page.pageCount} 页</span>
          <button disabled={current?.loading || page.index === page.pageCount - 1} onClick={() => selectPage(page.index + 1)} type="button">下一页</button>
          <button disabled={current?.loading || page.index === page.pageCount - 1} onClick={() => selectPage(page.pageCount - 1)} type="button">末页</button>
        </div>
        <form onSubmit={(event) => { event.preventDefault(); selectPage(Number(requestedPage) - 1); }}>
          <input aria-label="文档页码" disabled={current?.loading} max={page.pageCount} min={1} onChange={(event) => setRequestedPage(event.target.value)} required step={1} type="number" value={requestedPage}/>
          <button disabled={current?.loading} type="submit">跳转</button>
        </form>
      </> : null}
      <div className="preview-text-copy">
        <button disabled={copyStatus === "copying"} onClick={() => void copyAll()} type="button">{copyStatus === "copying" ? "正在复制" : "复制全文"}</button>
      </div>
      {copyStatus === "done" ? <span role="status">已复制</span> : copyStatus === "error" ? <span role="alert">复制失败</span> : null}
      {current?.loading ? <span role="status">正在载入页面</span> : null}
    </nav>
    {page.oversize && !page.tree ? <div className="preview-markdown-oversize" role="status">
      <p>{page.oversize.label}{page.oversize.startLine ? ` · 第 ${page.oversize.startLine}–${page.oversize.endLine ?? page.oversize.startLine} 行` : ""}</p>
      <p>内容较大，尚未完整显示。</p>
      <button disabled={current?.loading} onClick={() => selectPage(page.index, true)} type="button">完整显示</button>
    </div> : rendered.node}
  </div>;
});
