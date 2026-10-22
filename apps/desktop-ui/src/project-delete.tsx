import { useState } from "react";
import { randomUuid } from "./uuid";
import { useModal } from "./use-modal";
import type { ProjectDeleteResult } from "./generated/protocol";
import { rpc, sha256 } from "./app-server-rpc";
import "./project-delete.css";
import type { MessageSink } from "./app-message";

/**
 * Deleting a project removes the process data the app accumulated — phases, tasks, deliverable
 * metadata, blockers and decisions — and releases the workspaces it held. Files on disk are never
 * touched, and the change history keeps its record of what happened.
 */
export function ProjectDeleteDialog(props: {
  projectId: string;
  displayName: string;
  taskTotal: number;
  deliverableTotal: number;
  attentionTotal: number;
  onDeleted: (result: ProjectDeleteResult) => void;
  onMessage: MessageSink;
  onCancel: () => void;
}) {
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const nameMatches = confirmation.trim() === props.displayName.trim();
  const modalRef = useModal<HTMLDivElement>({ onClose: props.onCancel, busy });

  const deleteProject = async () => {
    if (!nameMatches || busy) return;
    setBusy(true);
    try {
      const requestFingerprint = await sha256([props.projectId, props.displayName].join("\n"));
      const result = await rpc<ProjectDeleteResult>("project/delete", {
        projectId: props.projectId,
        clientOperationId: `desktop-delete-${randomUuid()}`,
        requestFingerprint,
      });
      props.onDeleted(result);
      props.onMessage(`项目「${props.displayName}」已删除；释放了 ${result.releasedWorkspaceIds.length} 个工作区，磁盘文件未改动。`);
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  return <div className="project-delete-backdrop" ref={modalRef} role="dialog" aria-modal="true" aria-label="删除项目确认">
    <section className="project-delete-dialog">
      <h3>删除项目「{props.displayName}」</h3>
      <p className="project-delete-scope">将从应用中移除该项目累积的过程信息：</p>
      <ul className="project-delete-list">
        <li>{props.taskTotal} 个任务，以及它们所属的阶段与阶段计划</li>
        <li>{props.deliverableTotal} 个交付物的版本与验收记录</li>
        <li>{props.attentionTotal} 条阻塞、决策与异常</li>
        <li>该项目绑定的 AI 会话记录</li>
      </ul>
      <p className="project-delete-keep"><strong>磁盘上的文件不会被删除。</strong>工作区会被释放但保留，你仍然可以浏览这些文件，也可以把它重新加入其他项目。变更历史保留该项目发生过的记录。</p>
      <label>请输入项目名称以确认
        <input
          autoFocus
          onChange={(event) => setConfirmation(event.target.value)}
          placeholder={props.displayName}
          value={confirmation}/>
      </label>
      <div className="project-delete-actions">
        <button disabled={busy} onClick={props.onCancel} type="button">取消</button>
        <button className="danger" disabled={!nameMatches || busy} onClick={() => void deleteProject()} type="button">
          {busy ? "正在删除…" : "删除项目"}
        </button>
      </div>
    </section>
  </div>;
}
