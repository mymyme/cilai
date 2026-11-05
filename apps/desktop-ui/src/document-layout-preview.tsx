import { useEffect, useState } from "react";
import { invoke } from "./host";
import { usePageActive } from "./page-visibility";

export function DocumentLayoutPreview({ format, input }: { format: "html" | "docx"; input: string | ArrayBuffer }) {
  const active = usePageActive();
  const [attempt, setAttempt] = useState(0);
  const [result, setResult] = useState<{ input: typeof input; attempt: number; html?: string; error?: string } | null>(null);
  const current = result?.input === input && result.attempt === attempt ? result : null;
  useEffect(() => {
    if (!active || current) return;
    const id = crypto.randomUUID();
    let pending = true;
    void invoke<{ html: string }>("document_layout_start", { id, format, input }).then(value => {
      if (!pending) return;
      if (typeof value?.html !== "string") throw new Error("preview_layout_invalid");
      pending = false;
      setResult({ input, attempt, html: value.html });
    }).catch(error => {
      if (!pending) return;
      pending = false;
      const reason = String(error);
      setResult({ input, attempt, error: reason.includes("preview_layout_capacity") ? "上一份预览正在关闭。"
        : reason.includes("preview_file_too_large") || reason.includes("preview_layout_memory_limit") || reason.includes("preview_layout_output_limit") ? "文档超过预览上限。"
          : reason.includes("preview_layout_timeout") ? "排版超时。" : "无法排版文档。" });
    });
    return () => {
      if (!pending) return;
      pending = false;
      void invoke("document_layout_cancel", { id }).catch(error => console.error("无法取消文档排版", error));
    };
  }, [active, current, format, input, attempt]);
  if (current?.error) return <div className="preview-message" role="alert"><p>{current.error}</p><button type="button" onClick={() => setAttempt(value => value + 1)}>重试预览</button></div>;
  if (current?.html === undefined) return <div className="preview-message" role="status">正在排版</div>;
  return <iframe className={format === "docx" ? "office-docx-frame" : "preview-html-frame"} sandbox="" srcDoc={current.html} title={format === "docx" ? "Word 文档预览" : "网页预览"}/>;
}
