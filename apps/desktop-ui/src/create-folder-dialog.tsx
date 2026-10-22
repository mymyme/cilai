import { useState } from "react";
import { rpc } from "./app-server-rpc";
import { useModal } from "./use-modal";
import type { WorkspaceDirectoryCreateResult, WorkspaceSummary } from "./generated/protocol";
import { randomUuid } from "./uuid";
import "./local-workspace.css";

export function currentDirectory(file: { relativePath: string; fileType: string } | null): string {
  if (!file) return "";
  const path = file.relativePath.replaceAll("\\", "/");
  return file.fileType === "directory" ? path : path.split("/").slice(0, -1).join("/");
}

export function CreateFolderDialog(props: {
  workspace: WorkspaceSummary;
  parentRelativePath: string;
  onClose: () => void;
  onCreated: (relativePath: string) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [createdPath, setCreatedPath] = useState<string | null>(null);
  const [clientOperationId] = useState(() => `desktop-create-directory-${randomUuid()}`);
  const [error, setError] = useState<string | null>(null);
  const modalRef = useModal<HTMLDivElement>({ onClose: props.onClose, busy });
  const create = async () => {
    if (busy || !name.trim()) return;
    if (props.workspace.workspaceKind !== "local") { setError("个人版不支持此云端来源。"); return; }
    setBusy(true);
    setError(null);
    let path = createdPath;
    try {
      if (!path) {
        const result = await rpc<WorkspaceDirectoryCreateResult>("workspace/createDirectory", {
          workspaceId: props.workspace.workspaceId, parentRelativePath: props.parentRelativePath, name: name.trim(), clientOperationId,
        });
        path = result.relativePath;
        setCreatedPath(path);
      }
      await props.onCreated(path);
      props.onClose();
    } catch (caught) {
      const reason = String(caught);
      setError(path ? `文件夹已创建，刷新失败：${reason}`
        : reason.includes("already_exists") ? "已存在同名文件或文件夹。"
          : reason.includes("directory_name_invalid") ? "名称不能包含路径分隔符或系统保留字符。"
            : reason.includes("read_only") ? "此工作空间为只读。" : `创建失败：${reason}`);
    } finally { setBusy(false); }
  };
  return <div aria-label="新建文件夹" aria-modal="true" className="local-workspace-backdrop" ref={modalRef} role="dialog">
    <form className="local-workspace-dialog" onSubmit={(event) => { event.preventDefault(); void create(); }}>
      <header><h3>新建文件夹</h3></header>
      <div className="local-workspace-form">
        <p className="local-workspace-note">位置：{props.workspace.displayName}{props.parentRelativePath ? ` / ${props.parentRelativePath}` : ""}</p>
        <label>文件夹名称<input autoFocus disabled={busy || createdPath !== null} onChange={(event) => setName(event.target.value)} value={name}/></label>
        {error ? <p className="local-workspace-error" role="alert">{error}</p> : null}
      </div>
      <footer><button disabled={busy} onClick={props.onClose} type="button">取消</button><button className="primary" disabled={busy || !name.trim()} type="submit">{busy ? "处理中…" : createdPath ? "刷新" : "新建"}</button></footer>
    </form>
  </div>;
}
