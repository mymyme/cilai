import { useState } from "react";
import { useModal } from "./use-modal";
import { Select } from "./select";
import "./local-workspace.css";

export interface WorkspaceAddProject { projectId: string; displayName: string; workspaceIds?: string[] }

export function LocalWorkspaceDialog(props: {
  projects: WorkspaceAddProject[];
  target?: WorkspaceAddProject;
  onAddLocal: (project: WorkspaceAddProject) => void;
  onCreateProject: () => void;
  onClose: () => void;
}) {
  const [projectId, setProjectId] = useState(props.target?.projectId ?? props.projects[0]?.projectId ?? "");
  const project = props.projects.find((item) => item.projectId === projectId);
  const modalRef = useModal<HTMLDivElement>({ onClose: props.onClose });
  return <div aria-label="添加本地文件夹" aria-modal="true" className="local-workspace-backdrop" ref={modalRef} role="dialog">
    <section className="local-workspace-dialog">
      <header><h3>添加本地文件夹</h3></header>
      <label className="local-workspace-project">所属项目<Select ariaLabel="所属项目" value={projectId} onChange={setProjectId} options={props.projects.map((item) => ({ value: item.projectId, label: item.displayName }))} placeholder="请选择项目"/></label>
      {!props.projects.length ? <p>还没有项目。</p> : null}
      <footer><button onClick={props.onClose} type="button">取消</button><button onClick={props.onCreateProject} type="button">新建项目</button><button className="primary" disabled={!project} onClick={() => { if (project) props.onAddLocal(project); }} type="button">选择文件夹</button></footer>
    </section>
  </div>;
}
