import { useState } from "react";
import { invoke } from "./host";
import { artifactFileName, type ConversationArtifactEntry } from "./ai-artifacts";
import { artifactErrorMessage, useArtifactImagePreview } from "./artifact-preview";
import { FileIcon, RevealIcon } from "./icons";

export type TaskArtifactPreview = ConversationArtifactEntry & { url: string };

export function TaskArtifactList({ entries, onPreview }: {
  entries: ConversationArtifactEntry[];
  onPreview: (preview: TaskArtifactPreview) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const visible = expanded ? entries : entries.slice(0, 4);
  return <div className="task-artifact-list">
    {visible.map((entry) => <TaskArtifactRow entry={entry} key={`${entry.runId}:${entry.artifact.artifactId}`} onPreview={onPreview}/>)}
    {entries.length > 4 ? <button aria-expanded={expanded} className="task-artifact-more" onClick={() => setExpanded((value) => !value)} type="button">{expanded ? "收起" : `查看全部 ${entries.length} 份产物`}</button> : null}
  </div>;
}

function TaskArtifactRow({ entry, onPreview }: { entry: ConversationArtifactEntry; onPreview: (preview: TaskArtifactPreview) => void }) {
  const { artifact, runId } = entry;
  const name = artifactFileName(artifact.path!);
  const image = artifact.artifactType === "image";
  const preview = useArtifactImagePreview(artifact, runId);
  const [openError, setOpenError] = useState("");
  const [busy, setBusy] = useState(false);
  const open = async (openMode: "default" | "finder") => {
    setBusy(true);
    setOpenError("");
    try {
      await invoke("ai_artifact_open", { runId, artifactId: artifact.artifactId, openMode });
    } catch (error) {
      setOpenError(`无法打开：${artifactErrorMessage(error)}`);
    } finally {
      setBusy(false);
    }
  };
  return <div className="task-artifact-row" data-image={image || undefined}>
    <button aria-label={`${image ? "预览图片" : "打开文件"}：${name}`} className="task-artifact-open" disabled={busy || (image && !preview.url)} onClick={() => image ? onPreview({ ...entry, url: preview.url }) : void open("default")} title={artifact.path!} type="button">
      <span className="task-artifact-thumbnail">{image && preview.url ? <img alt="" onError={preview.fail} src={preview.url}/> : image && !preview.error ? <span className="task-artifact-loading">正在加载…</span> : <FileIcon/>}</span>
      <span className="task-artifact-name"><strong>{artifact.label || name}</strong><small>{name}</small></span>
    </button>
    <button aria-label={`定位文件：${name}`} className="task-artifact-reveal" disabled={busy} onClick={() => void open("finder")} title="定位" type="button"><RevealIcon/></button>
    {image && preview.error ? <div className="task-artifact-error" role="status"><span>{preview.error}</span><button onClick={preview.retry} type="button">重试</button></div> : null}
    {openError ? <div className="task-artifact-error" role="alert">{openError}</div> : null}
  </div>;
}
