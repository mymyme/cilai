import { useEffect, useState } from "react";
import { createPortal } from "./page-portal";
import { invoke } from "./host";
import { useModal } from "./use-modal";
import { CloseIcon, FileIcon } from "./icons";
import { artifactFileName, type ConversationArtifact } from "./ai-artifacts";
import "./artifact-preview.css";

export function artifactErrorMessage(error: unknown): string {
  const message = String(error);
  if (/ai_artifact_unavailable|ai_artifact_not_found|agent_run_not_found|conversation_not_found|ai_artifact_path_missing/.test(message)) return "产物不存在或已失效";
  if (message.includes("ai_artifact_image_invalid")) return "图片不是有效文件或超过 64 MiB";
  if (message.includes("ai_artifact_not_image")) return "该产物不是图片";
  if (message.includes("ai_artifact_image_decode_failed")) return "图片无法解码";
  return `读取失败：${message}`;
}

export function useArtifactImagePreview(artifact: ConversationArtifact, runId: string) {
  const key = JSON.stringify([runId, artifact.artifactId, artifact.path, artifact.artifactType]);
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState({ key: "", url: "", error: "" });
  useEffect(() => {
    if (artifact.artifactType !== "image") return;
    let disposed = false;
    let objectUrl: string | null = null;
    setState({ key, url: "", error: "" });
    void invoke<ArrayBuffer | Uint8Array | number[]>("ai_artifact_read_image", { runId, artifactId: artifact.artifactId })
      .then((payload) => {
        if (disposed) return;
        if (!(payload instanceof ArrayBuffer) && !(payload instanceof Uint8Array) && !Array.isArray(payload)) throw new Error("图片数据无效");
        const source = payload instanceof ArrayBuffer ? new Uint8Array(payload) : Uint8Array.from(payload);
        if (!source.byteLength) throw new Error("图片数据为空");
        objectUrl = URL.createObjectURL(new Blob([source], { type: imageArtifactMimeType(artifact.path) }));
        setState({ key, url: objectUrl, error: "" });
      }).catch((error: unknown) => { if (!disposed) setState({ key, url: "", error: artifactErrorMessage(error) }); });
    return () => { disposed = true; if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [key, attempt, artifact.artifactId, artifact.artifactType, artifact.path, runId]);
  return {
    url: state.key === key ? state.url : "",
    error: state.key === key ? state.error : "",
    retry: () => setAttempt((value) => value + 1),
    fail: () => setState({ key, url: "", error: "图片无法解码" }),
  };
}

export function ArtifactVisual({ artifact, runId }: { artifact: ConversationArtifact; runId: string }) {
  const preview = useArtifactImagePreview(artifact, runId);
  if (artifact.artifactType !== "image") return <span aria-hidden="true" className="ai-artifact-visual"><FileIcon/></span>;
  return <span className={`ai-artifact-visual image${preview.error ? " unavailable" : preview.url ? "" : " loading"}`} title={preview.error || undefined}>
    {preview.url ? <img alt="" onError={preview.fail} src={preview.url}/> : preview.error ? <span className="artifact-preview-error">{preview.error}</span> : <ImageMaterialIcon/>}
  </span>;
}

export function ImageArtifactLightbox({ activeIndex, artifacts, onChange, onClose, previews }: {
  activeIndex: number;
  artifacts: ConversationArtifact[];
  onChange: (index: number) => void;
  onClose: () => void;
  previews: Record<string, string>;
}) {
  const modalRef = useModal<HTMLDivElement>({ onClose });
  const artifact = artifacts[activeIndex];
  const previewUrl = previews[artifact.artifactId] ?? null;
  const previous = () => onChange((activeIndex - 1 + artifacts.length) % artifacts.length);
  const next = () => onChange((activeIndex + 1) % artifacts.length);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (artifacts.length < 2) return;
      if (event.key === "ArrowLeft") previous();
      if (event.key === "ArrowRight") next();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });
  return createPortal(<div aria-label={`查看图片 ${artifactFileName(artifact.path!)}`} aria-modal="true" className="ai-artifact-lightbox" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }} ref={modalRef} role="dialog">
    <figure>
      {previewUrl ? <img alt={artifactFileName(artifact.path!)} src={previewUrl}/> : <span aria-label="正在加载图片" className="ai-run-image-loading"><ImageMaterialIcon/></span>}
      <figcaption><strong>{artifactFileName(artifact.path!)}</strong>{artifacts.length > 1 ? <span>{activeIndex + 1} / {artifacts.length}</span> : null}</figcaption>
    </figure>
    <button aria-label="关闭图片预览" autoFocus className="close" onClick={onClose} type="button"><CloseIcon/></button>
    {artifacts.length > 1 ? <>
      <button aria-label="上一张" className="previous" onClick={previous} type="button"><ChevronIcon direction="left"/></button>
      <button aria-label="下一张" className="next" onClick={next} type="button"><ChevronIcon direction="right"/></button>
    </> : null}
  </div>, document.body);
}

export function imageArtifactMimeType(path: string | null) {
  const extension = path?.split(".").pop()?.toLowerCase();
  if (extension === "jpg" || extension === "jpeg") return "image/jpeg";
  if (extension === "webp") return "image/webp";
  if (extension === "gif") return "image/gif";
  if (extension === "svg") return "image/svg+xml";
  return "image/png";
}

function ImageMaterialIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><rect x="4" y="4" width="16" height="16" rx="2.5"/><circle cx="9" cy="9" r="1.5"/><path d="m6.5 17 4-4 2.5 2.5 2-2 2.5 3.5"/></svg>; }
function ChevronIcon({ direction }: { direction: "left" | "right" }) { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d={direction === "right" ? "m10 7 5 5-5 5" : "m14 7-5 5 5 5"}/></svg>; }
