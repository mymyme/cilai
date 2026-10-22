import { useEffect, useState } from "react";
import { useModal } from "./use-modal";
import type { ProjectSummary, ProjectUpdateResult, WorkspaceSummary } from "./generated/protocol";
import { rpc } from "./app-server-rpc";
import "./project-edit.css";
import type { MessageSink } from "./app-message";
import { hostPlatformUi } from "./host-platform-ui";
import { RevealIcon } from "./icons";

export interface ProjectEditTarget {
  projectId: string;
  displayName: string;
  goal: string;
  stateRevision: number;
  workspaceIds: string[];
}

/**
 * Everything about a project that a person owns: what it is called, what it is for, and which
 * folders on disk belong to it.
 *
 * These used to be split across a page the file switcher happened to link to, which made the
 * project page read-only and put folder ownership somewhere the project was not. A project is an
 * abstract container, so the place you edit the container is the place its folders are managed.
 */
export function ProjectEditDialog(props: {
  hostPlatform: string | null;
  target: ProjectEditTarget;
  workspaces: WorkspaceSummary[];
  connectingWorkspace: boolean;
  onAddFolder: () => void;
  onRemoveFolder: (workspace: WorkspaceSummary) => void;
  onReveal: (workspace: WorkspaceSummary) => void;
  onSaved: (project: ProjectSummary) => void;
  onMessage: MessageSink;
  onClose: () => void;
}) {
  const [displayName, setDisplayName] = useState(props.target.displayName);
  const [goal, setGoal] = useState(props.target.goal);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [initialWorkspaceIds] = useState(() => [...props.target.workspaceIds]);
  const { onClose } = props;
  const locked = busy || props.connectingWorkspace;
  const platformUi = hostPlatformUi(props.hostPlatform);

  const modalRef = useModal<HTMLDivElement>({ onClose, busy: locked });

  const folders = props.workspaces.filter((workspace) => props.target.workspaceIds.includes(workspace.workspaceId));
  const metadataDirty = displayName.trim() !== props.target.displayName || goal.trim() !== props.target.goal;
  const foldersDirty = props.target.workspaceIds.length !== initialWorkspaceIds.length
    || props.target.workspaceIds.some((workspaceId) => !initialWorkspaceIds.includes(workspaceId));
  const dirty = metadataDirty || foldersDirty;
  const save = async () => {
    if (!displayName.trim() || locked || !dirty) return;
    if (!metadataDirty) {
      props.onClose();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const result = await rpc<ProjectUpdateResult>("project/update", {
        projectId: props.target.projectId,
        displayName: displayName.trim(),
        goal: goal.trim(),
        expectedRevision: props.target.stateRevision,
      });
      props.onMessage("项目信息已更新。");
      props.onSaved(result.project);
    } catch (caught) {
      const reason = String(caught);
      setError(reason.includes("project_revision_conflict")
        ? "这个项目在你编辑期间已被改动，请关闭后重新打开再改一次。"
        : `保存失败：${reason}`);
    } finally {
      setBusy(false);
    }
  };

  return <div
    aria-label="编辑项目"
    aria-modal="true"
    className="project-edit-backdrop"
    ref={modalRef}
    onMouseDown={(event) => { if (event.target === event.currentTarget && !locked) props.onClose(); }}
    role="dialog">
    <section className="project-edit">
      <header><h3>编辑项目</h3></header>

      <label>项目名称<input
        autoFocus
        disabled={locked}
        onChange={(event) => setDisplayName(event.target.value)}
        placeholder="这个项目对外怎么称呼"
        value={displayName}/></label>

      <label>项目说明<textarea
        disabled={locked}
        onChange={(event) => setGoal(event.target.value)}
        placeholder="目前想推进什么，后续可以逐步补充"
        rows={3}
        value={goal}/></label>

      <section className="project-edit-folders">
        <header><h4>工作空间</h4><div className="project-edit-folder-create-actions"><button disabled={locked} onClick={props.onAddFolder} type="button"><FolderPlusIcon/>添加本地文件夹</button></div></header>
        {folders.map((workspace) => <article key={workspace.workspaceId}>
          <div>
            <strong>{workspace.displayName}{workspace.workspaceKind === "cloud_team" ? " · 旧云端来源" : ""}</strong>
            <small title={workspace.workspaceKind === "cloud_team" ? "个人版不支持此云端来源" : workspace.localRoot}>{workspace.workspaceKind === "cloud_team" ? "个人版不支持此云端来源" : workspace.localRoot}</small>
          </div>
          <div className="project-edit-folder-actions">
            {platformUi && workspace.workspaceKind === "local" ? <button aria-label={`在${platformUi.fileManagerName}中显示`} disabled={locked} onClick={() => props.onReveal(workspace)} title={`在${platformUi.fileManagerName}中显示`} type="button"><RevealIcon/></button> : null}
            <button disabled={locked} onClick={() => props.onRemoveFolder(workspace)} type="button">移出项目</button>
          </div>
        </article>)}
        {!folders.length ? <p className="project-edit-empty">这个项目还没有工作空间。</p> : null}
      </section>

      {error ? <p className="project-edit-error" role="alert">{error}</p> : null}
      <footer>
        <button disabled={locked} onClick={props.onClose} type="button">关闭</button>
        <button className="primary" disabled={locked || !dirty || !displayName.trim()} onClick={() => void save()} type="button">{busy ? "保存中…" : "保存"}</button>
      </footer>
    </section>
  </div>;
}

function FolderPlusIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M3 6h7l2 2h9v11H3z"/><path d="M12 12v5M9.5 14.5h5"/></svg>; }
