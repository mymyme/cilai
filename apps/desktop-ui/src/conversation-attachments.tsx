import { useEffect,useState } from "react";
import {
readAiAttachment,
type AiMaterialReference,
type AiMessageAttachment
} from "./ai-runtime";
import type { DraftAttachment } from './ai-workbench';
import "./conversation-attachments.css";
import { fileNameFromPath } from './conversation-display';
import { CloseIcon,FileIcon,MediaIcon } from './conversation-icons';
import { useModal } from "./use-modal";

export function MaterialAttachments({ materials, onRemove }: { materials: AiMaterialReference[]; onRemove?: (material: AiMaterialReference) => void }) {
  return <div aria-label={onRemove ? "待发送文件" : "消息文件"} className={onRemove ? "ai-draft-attachments" : "ai-message-attachments"}>{materials.map((material) => <div className="ai-draft-attachment ai-file-attachment" key={`${material.workspaceId ?? ""}:${material.localFileId}:${material.fileRevision}`} title={material.relativePath}>
    <span className="ai-document-attachment"><FileIcon/><span>{fileNameFromPath(material.relativePath)}</span></span>
    {onRemove ? <button aria-label={`移除文件 ${fileNameFromPath(material.relativePath)}`} className="ai-attachment-remove" onClick={() => onRemove(material)} type="button"><CloseIcon/></button> : null}
  </div>)}</div>;
}

export function DraftAttachments({ items, onRemove, removable = true }: { items: DraftAttachment[]; onRemove: (attachmentId: string) => void; removable?: boolean }) {
  return <div aria-label="待发送附件" className="ai-draft-attachments">{items.map(({ attachment, previewUrl }) => <div className={`ai-draft-attachment${attachment.mediaType === "document" ? " ai-file-attachment" : ""}`} key={attachment.attachmentId}>
    <>{attachment.mediaType === "document" ? <span className="ai-document-attachment"><FileIcon/><span>{attachment.fileName}<small>{Math.ceil(attachment.sizeBytes / 1024)} KB</small></span></span> : previewUrl ? <MediaThumbnail attachment={attachment} previewUrl={previewUrl}/> : <StoredAttachmentThumbnail attachment={attachment}/>}</>
    {removable ? <button aria-label={`移除附件 ${attachment.fileName}`} className="ai-attachment-remove" onClick={() => onRemove(attachment.attachmentId)} type="button"><CloseIcon/></button> : null}
  </div>)}</div>;
}

export function MessageAttachments({ attachments }: { attachments: AiMessageAttachment[] }) {
  return <div aria-label="消息附件" className="ai-message-attachments">{attachments.map((attachment) => attachment.mediaType === "document" ? <span className="ai-document-attachment" key={attachment.attachmentId}><FileIcon/><span>{attachment.fileName}<small>已提供 · 读取记录见执行过程</small></span></span> : <StoredAttachmentThumbnail attachment={attachment} key={attachment.attachmentId}/>)}</div>;
}

export function StoredAttachmentThumbnail({ attachment }: { attachment: AiMessageAttachment }) {
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  useEffect(() => {
    let disposed = false;
    let objectUrl: string | null = null;
    void readAiAttachment(attachment.attachmentId).then((bytes) => {
      if (disposed) return;
      objectUrl = URL.createObjectURL(new Blob([bytes], { type: attachment.mimeType }));
      setPreviewUrl(objectUrl);
    }).catch(() => { if (!disposed) setPreviewUrl(""); });
    return () => {
      disposed = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [attachment.attachmentId, attachment.mimeType]);
  if (previewUrl === null) return <span aria-label={`正在加载 ${attachment.fileName}`} className="ai-attachment-thumbnail loading"/>;
  if (!previewUrl) return <span aria-label={`无法显示 ${attachment.fileName}`} className="ai-attachment-thumbnail unavailable"><MediaIcon/></span>;
  return <MediaThumbnail attachment={attachment} previewUrl={previewUrl}/>;
}

export function MediaThumbnail({ attachment, previewUrl }: { attachment: AiMessageAttachment; previewUrl: string }) {
  const [open, setOpen] = useState(false);
  return <>
    <button aria-label={`查看${attachment.mediaType === "image" ? "图片" : "视频"} ${attachment.fileName}`} className="ai-attachment-thumbnail" onClick={() => setOpen(true)} title="点击放大查看" type="button">
      {attachment.mediaType === "image"
        ? <img alt={attachment.fileName} src={previewUrl}/>
        : <video aria-label={attachment.fileName} muted playsInline preload="metadata" src={previewUrl}/>}
    </button>
    {open ? <MediaLightbox attachment={attachment} onClose={() => setOpen(false)} previewUrl={previewUrl}/> : null}
  </>;
}

export function MediaLightbox({ attachment, onClose, previewUrl }: { attachment: AiMessageAttachment; onClose: () => void; previewUrl: string }) {
  const modalRef = useModal<HTMLDivElement>({ onClose });
  return <div aria-label={`查看${attachment.mediaType === "image" ? "图片" : "视频"} ${attachment.fileName}`} aria-modal="true" className="ai-media-lightbox" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }} ref={modalRef} role="dialog">
    <section>
      <button aria-label="关闭媒体预览" autoFocus onClick={onClose} type="button"><CloseIcon/></button>
      {attachment.mediaType === "image"
        ? <img alt={attachment.fileName} src={previewUrl}/>
        : <video aria-label={attachment.fileName} autoPlay controls playsInline src={previewUrl}/>}
    </section>
  </div>;
}
