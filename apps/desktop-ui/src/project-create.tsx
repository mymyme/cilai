import { useRef, useState, type ReactElement } from "react";
import { randomUuid } from "./uuid";
import { useModal } from "./use-modal";
import type { ProjectAttachWorkspaceResult, ProjectCreateResult, ProjectSummary, WorkspaceSummary } from "./generated/protocol";
import { rpc, sha256 } from "./app-server-rpc";
import { describeServerError } from "./server-error";
import { CloseIcon, TreeFolderIcon } from "./icons";
import { Select } from "./select";
import type { MessageSink } from "./app-message";

export function ProjectCreatePanel(props: {
  workspaces: WorkspaceSummary[];
  onCreated: (project: ProjectSummary, workspaceId: string | null) => void;
  onMessage: MessageSink;
  onCancel?: () => void;
  onAddWorkspace: () => Promise<WorkspaceSummary | null | undefined>;
  existingProjects: { projectId: string; displayName: string }[];
}) {
  const [selectedWorkspaces, setSelectedWorkspaces] = useState<WorkspaceSummary[]>([]);
  const [name, setName] = useState("");
  const [goal, setGoal] = useState("");
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState(false);
  const submission = useRef<{ fingerprint: string; operationId: string } | null>(null);
  const locked = busy || adding;
  const workspaceIds = selectedWorkspaces.map((workspace) => workspace.workspaceId);
  const workspaceId = workspaceIds[0] ?? null;

  const addWorkspace = async () => {
    if (locked) return;
    setAdding(true);
    try {
      const workspace = await props.onAddWorkspace();
      if (!workspace) return;
      setSelectedWorkspaces((current) => current.some((item) => item.workspaceId === workspace.workspaceId)
        ? current : [...current, workspace]);
      setName((current) => current.trim() ? current : workspace.displayName);
    } catch (error) {
      props.onMessage(describeServerError(error), "error");
    } finally {
      setAdding(false);
    }
  };

  const modalRef = useModal<HTMLDivElement>({ onClose: () => props.onCancel?.(), busy: locked });
  const submittable = name.trim().length > 0 && goal.trim().length > 0 && !locked;

  const createProject = async () => {
    if (!submittable) return;
    setBusy(true);
    try {
      const requestFingerprint = await sha256(JSON.stringify([workspaceIds, name.trim(), goal.trim()]));
      if (submission.current?.fingerprint !== requestFingerprint) submission.current = { fingerprint: requestFingerprint, operationId: `desktop-project-${randomUuid()}` };
      const clientOperationId = submission.current.operationId;
      const result = await rpc<ProjectCreateResult>("project/create", {
        workspaceIds,
        displayName: name.trim(),
        goal: goal.trim(),
        clientOperationId,
        requestFingerprint,
      });
      setGoal("");
      props.onCreated(result.project, result.project.primaryWorkspaceId ?? workspaceId);
      props.onMessage("项目已创建。");
    } catch (error) {
      props.onMessage(describeServerError(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const attachWorkspace = async (projectId: string) => {
    if (!workspaceIds.length || locked) return;
    setBusy(true);
    try {
      const requestFingerprint = await sha256(JSON.stringify([projectId, workspaceIds]));
      const result = await rpc<ProjectAttachWorkspaceResult>("project/attachWorkspace", {
        projectId,
        workspaceIds,
        clientOperationId: `desktop-attach-${randomUuid()}`,
        requestFingerprint,
      });
      props.onCreated(result.project, workspaceId);
      props.onMessage(`工作区已加入项目「${result.project.displayName}」，该项目现有 ${result.workspaceIds.length} 个工作区。`);
    } catch (error) {
      props.onMessage(describeServerError(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const shell = (body: ReactElement) => <div
    aria-label="新建项目"
    aria-modal="true"
    className="project-create-backdrop"
    ref={modalRef}
    // Clicking the backdrop dismisses, but only when the press started there: dragging a text
    // selection out of the dialog must not close it.
    onMouseDown={(event) => { if (event.target === event.currentTarget && !locked) props.onCancel?.(); }}
    role="dialog">{body}</div>;

  return <>{shell(<section className="project-create-panel">
    <h3>新建项目</h3>
    <form onSubmit={(event) => { event.preventDefault(); void createProject(); }}>
      <label>项目名称<input autoFocus disabled={locked} onChange={(event) => setName(event.target.value)} value={name}/></label>
      <label>项目说明<textarea disabled={locked} onChange={(event) => setGoal(event.target.value)} placeholder="用一句话说明目前想推进的事情，后续可以逐步补充。" rows={4} value={goal}/></label>
      {props.existingProjects.length ? <label>或加入已有项目
        <Select disabled={locked || !workspaceIds.length} options={[{ value: "", label: "保持新建项目" }, ...props.existingProjects.map((item) => ({ value: item.projectId, label: item.displayName }))]} value="" onChange={(target) => { if (target) void attachWorkspace(target); }}/>
      </label> : null}
      <section aria-label="本地文件夹（可选）" className="project-create-folders">
        <h4>本地文件夹（可选）</h4>
        {selectedWorkspaces.length ? <ul aria-label="已选择的文件夹" className="project-create-folder-list">
          {selectedWorkspaces.map((workspace) => <li key={workspace.workspaceId}>
            <TreeFolderIcon/>
            <div><strong>{workspace.displayName}</strong><small title={workspace.localRoot}>{workspace.localRoot}</small></div>
            <button aria-label={`移除文件夹：${workspace.displayName}`} disabled={locked} onClick={() => setSelectedWorkspaces((current) => current.filter((item) => item.workspaceId !== workspace.workspaceId))} title="移除文件夹" type="button"><CloseIcon/></button>
          </li>)}
        </ul> : null}
        <div className="project-create-workspace-actions">
          <button className="project-create-link" disabled={locked} onClick={() => void addWorkspace()} type="button">{adding ? "正在添加…" : "添加本地文件夹…"}</button>
        </div>
      </section>
      <div className="project-create-actions">
        {props.onCancel ? <button disabled={locked} onClick={props.onCancel} type="button">取消</button> : null}
        <button className="primary" disabled={!submittable} type="submit">{busy ? "正在创建…" : "创建项目"}</button>
      </div>
    </form>
  </section>)}
  </>;
}
