import { useEffect, useRef, useState } from "react";
import { invoke } from "./host";
import { writeClipboard } from "./clipboard";
import type { OpenPreviewResult, ReadTextResourceResult } from "./generated/protocol";
import { usePageActive } from "./page-visibility";
import { ERROR_LABELS } from "./preview-labels";

export async function readPreviewPage(descriptor: OpenPreviewResult, offset: number, signal: AbortSignal): Promise<ReadTextResourceResult> {
  signal.throwIfAborted();
  const page = await invoke<ReadTextResourceResult>("text_resource_read", {
    params: { ticketId: descriptor.ticketId, offset, maxBytes: 65_536 },
  });
  signal.throwIfAborted();
  if (page.contentRevision !== descriptor.contentRevision || page.offset !== offset
    || !Number.isSafeInteger(page.nextOffset) || page.nextOffset < offset
    || (!page.endOfResource && page.nextOffset === offset)
    || (descriptor.contentBytes != null && (page.nextOffset > descriptor.contentBytes
      || page.endOfResource !== (page.nextOffset === descriptor.contentBytes)))) {
    throw new Error("preview_source_changed");
  }
  return page;
}

export async function readPreviewText(descriptor: OpenPreviewResult, signal: AbortSignal): Promise<string> {
  const chunks: string[] = [];
  let offset = 0;
  for (;;) {
    const page = await readPreviewPage(descriptor, offset, signal);
    chunks.push(page.text);
    if (page.endOfResource) return chunks.join("");
    offset = page.nextOffset;
  }
}

export function PagedTextPreview({ descriptor, firstPage, onRetry }: {
  descriptor: OpenPreviewResult; firstPage: ReadTextResourceResult; onRetry: () => void;
}) {
  const active = usePageActive();
  const [offsets, setOffsets] = useState([0]);
  const [pageIndex, setPageIndex] = useState(0);
  const [page, setPage] = useState(firstPage);
  const [error, setError] = useState<string | null>(null);
  const [copy, setCopy] = useState<"idle" | "pending" | "done" | "error">("idle");
  const copyController = useRef<AbortController | null>(null);
  const text = useRef<HTMLPreElement>(null);
  const offset = offsets[pageIndex];
  useEffect(() => {
    if (!active || page.offset === offset) return;
    const controller = new AbortController();
    setError(null);
    const read = offset === 0 ? Promise.resolve(firstPage) : readPreviewPage(descriptor, offset, controller.signal);
    void read.then(next => {
      if (!controller.signal.aborted) {
        setPage(next);
        text.current?.closest(".preview-document")?.scrollTo?.(0, 0);
      }
    }).catch(reason => {
      if (!controller.signal.aborted) {
        const code = Object.keys(ERROR_LABELS).find(key => String(reason).includes(key));
        setError(code ? ERROR_LABELS[code] : "无法读取这一页。");
      }
    });
    return () => controller.abort();
  }, [active, descriptor, firstPage, offset, page.offset]);
  useEffect(() => {
    if (!active) setCopy("idle");
    return () => { copyController.current?.abort(); copyController.current = null; };
  }, [active]);
  const copyAll = async () => {
    if (copyController.current) return;
    const controller = new AbortController();
    copyController.current = controller;
    setCopy("pending");
    try {
      const value = firstPage.endOfResource ? firstPage.text : await readPreviewText(descriptor, controller.signal);
      controller.signal.throwIfAborted();
      await writeClipboard(value);
      if (!controller.signal.aborted) setCopy("done");
    } catch { if (!controller.signal.aborted) setCopy("error"); }
    finally { if (copyController.current === controller) copyController.current = null; }
  };
  const loading = page.offset !== offset;
  return <>
    <nav aria-label="正文分页" className="preview-text-pagination">
      {firstPage.endOfResource ? null : <>
        <button type="button" disabled={!pageIndex} onClick={() => setPageIndex(index => index - 1)}>上一页</button>
        <span>第 {pageIndex + 1} 页</span>
        <button type="button" disabled={loading || page.endOfResource} onClick={() => {
          setOffsets(current => current[pageIndex + 1] === page.nextOffset ? current : [...current.slice(0, pageIndex + 1), page.nextOffset]);
          setPageIndex(index => index + 1);
        }}>下一页</button>
      </>}
      <button type="button" disabled={!active || copy === "pending"} onClick={() => void copyAll()}>{copy === "pending" ? "正在复制" : "复制全文"}</button>
      {copy === "done" ? <span role="status">已复制</span> : copy === "error" ? <span role="alert">复制失败</span> : null}
    </nav>
    {error ? <p role="alert">{error} <button type="button" onClick={onRetry}>重试预览</button></p>
      : loading ? <p role="status">正在读取</p>
        : <pre ref={text} className="preview-plain-text"><code>{page.text}</code></pre>}
  </>;
}
