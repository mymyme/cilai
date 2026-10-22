import { invoke } from "./host";
import { randomUuid } from "./uuid";
import { useModal } from "./use-modal";
import { useEffect, useMemo, useState } from "react";
import type {
  DeliverableCreateResult,
  DeliverableItem,
  DeliverableVersionCreateResult,
  DeliverableVersionItem,
  DeliverableVersionReviewResult,
  ProjectSummary,
  WorkspaceFileItem,
  WorkspaceSummary,
} from "./generated/protocol";
import { rpc, sha256 } from "./app-server-rpc";
import { Select } from "./select";
import type { MessageSink } from "./app-message";
import { auditMoment } from "./datetime";
import { hostPlatformUi } from "./host-platform-ui";

export function DeliverableManager(props: {
  hostPlatform: string | null;
  workspace: WorkspaceSummary | null;
  project: ProjectSummary;
  deliverables: DeliverableItem[];
  files: WorkspaceFileItem[];
  onClose: () => void;
  onChanged: () => Promise<void>;
  onMessage: MessageSink;
}) {
  const platformUi = hostPlatformUi(props.hostPlatform);
  const [selectedId, setSelectedId] = useState<string | null>(props.deliverables[0]?.deliverableId ?? null);
  const [showCreate, setShowCreate] = useState(props.deliverables.length === 0);
  const [busy, setBusy] = useState(false);
  const modalRef = useModal<HTMLDivElement>({ onClose: props.onClose, busy });
  const [title, setTitle] = useState("");
  const [type, setType] = useState("document");
  const [owner, setOwner] = useState("");
  const [criteria, setCriteria] = useState("");
  const [summary, setSummary] = useState("");
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(() => new Set());
  const [fileQuery, setFileQuery] = useState("");
  const [reviewer, setReviewer] = useState("");
  const [reviewComment, setReviewComment] = useState("");

  useEffect(() => {
    setSelectedId((current) => props.deliverables.some((item) => item.deliverableId === current)
      ? current
      : props.deliverables[0]?.deliverableId ?? null);
    if (props.deliverables.length) setShowCreate(false);
  }, [props.deliverables]);

  const selected = props.deliverables.find((item) => item.deliverableId === selectedId) ?? null;
  const candidate = selected?.versions.find((version) => version.status === "candidate") ?? null;
  const availableFiles = useMemo(() => {
    const query = fileQuery.trim().toLocaleLowerCase("zh-CN");
    return props.files.filter((file) => file.fileType === "file" && file.presenceState === "present" && (!query || file.relativePath.toLocaleLowerCase("zh-CN").includes(query)));
  }, [fileQuery, props.files]);

  const createDeliverable = async () => {
    if (busy || !title.trim() || !owner.trim() || !criteria.trim()) return;
    setBusy(true);
    try {
      const clientOperationId = `desktop-deliverable-${randomUuid()}`;
      const requestFingerprint = await sha256([props.project.projectId, title.trim(), type, owner.trim(), criteria.trim()].join("\n"));
      const result = await rpc<DeliverableCreateResult>("deliverable/create", {
        projectId: props.project.projectId,
        title: title.trim(),
        deliverableType: type,
        ownerLabel: owner.trim(),
        acceptanceCriteria: criteria.trim(),
        clientOperationId,
        requestFingerprint,
      });
      setTitle(""); setOwner(""); setCriteria(""); setShowCreate(false); setSelectedId(result.deliverable.deliverableId);
      await props.onChanged();
      props.onMessage("交付物已定义，可以绑定文件并提交候选版本。");
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const createVersion = async () => {
    if (!selected || busy || !summary.trim() || selectedFiles.size === 0) return;
    setBusy(true);
    try {
      const localFileIds = [...selectedFiles].sort();
      const clientOperationId = `desktop-deliverable-version-${randomUuid()}`;
      const requestFingerprint = await sha256([selected.deliverableId, summary.trim(), ...localFileIds].join("\n"));
      await rpc<DeliverableVersionCreateResult>("deliverable/version/create", {
        deliverableId: selected.deliverableId,
        summary: summary.trim(),
        localFileIds,
        carriers: [],
        clientOperationId,
        requestFingerprint,
      });
      setSummary(""); setSelectedFiles(new Set());
      await props.onChanged();
      props.onMessage("候选交付版本已创建，等待人工验收。");
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const reviewVersion = async (version: DeliverableVersionItem, decision: "accepted" | "rejected") => {
    if (!selected || busy || !reviewer.trim() || !reviewComment.trim()) return;
    setBusy(true);
    try {
      const clientOperationId = `desktop-deliverable-review-${randomUuid()}`;
      const requestFingerprint = await sha256([selected.deliverableId, version.versionId, decision, String(selected.stateRevision), reviewer.trim(), reviewComment.trim()].join("\n"));
      await rpc<DeliverableVersionReviewResult>("deliverable/version/review", {
        deliverableId: selected.deliverableId,
        versionId: version.versionId,
        decision,
        reviewerLabel: reviewer.trim(),
        comment: reviewComment.trim(),
        expectedDeliverableRevision: selected.stateRevision,
        clientOperationId,
        requestFingerprint,
      });
      setReviewComment("");
      await props.onChanged();
      props.onMessage(decision === "accepted" ? "交付版本已验收为正式版本。" : "交付版本已退回修改。");
    } catch (error) {
      props.onMessage(String(error), "error");
    } finally {
      setBusy(false);
    }
  };

  const openFile = async (relativePath: string, openMode: "default" | "finder") => {
    if (!props.workspace) {
      props.onMessage("请先为项目添加文件夹，再打开交付物文件。", "error");
      return;
    }
    if (props.workspace.workspaceKind === "cloud_team") {
      props.onMessage("云端文件请在文件页预览。", "error");
      return;
    }
    try {
      await invoke("workspace_open_item", { localRoot: props.workspace.localRoot, relativePath, openMode });
    } catch (error) {
      props.onMessage(`无法打开文件：${String(error)}`, "error");
    }
  };

  return <div className="deliverable-manager-backdrop" ref={modalRef} role="presentation">
    <section aria-label="项目交付物管理" aria-modal="true" className="deliverable-manager" role="dialog">
      <header className="deliverable-manager-header"><div><h2>{props.project.displayName} · 交付物</h2><p>在项目推进中定义、提交与验收交付物；这里不是独立的项目成果页面。</p></div><button aria-label="关闭交付物管理" onClick={props.onClose} type="button"><CloseIcon/></button></header>
      <div className="deliverable-manager-body">
        <aside className="deliverable-list-panel">
          <header><div><small>交付物清单</small><h2>{props.deliverables.length} 项</h2></div><button aria-label="新建交付物" onClick={() => setShowCreate(true)} type="button"><PlusIcon/></button></header>
          {showCreate ? <form className="deliverable-create-form" onSubmit={(event) => { event.preventDefault(); void createDeliverable(); }}>
            <label>名称<input autoFocus onChange={(event) => setTitle(event.target.value)} value={title}/></label>
            <div className="form-row"><label>类型<Select options={[{ value: "document", label: "文档" }, { value: "software", label: "软件" }, { value: "design", label: "设计" }, { value: "dataset", label: "数据" }, { value: "other", label: "其他" }]} value={type} onChange={setType}/></label><label>负责人<input onChange={(event) => setOwner(event.target.value)} value={owner}/></label></div>
            <label>验收标准<textarea onChange={(event) => setCriteria(event.target.value)} rows={3} value={criteria}/></label>
            <div className="form-actions"><button onClick={() => setShowCreate(false)} type="button">取消</button><button className="primary" disabled={busy || !title.trim() || !owner.trim() || !criteria.trim()} type="submit">保存</button></div>
          </form> : null}
          <div className="deliverable-cards">{props.deliverables.map((item) => <button className={item.deliverableId === selectedId ? "selected" : ""} key={item.deliverableId} onClick={() => setSelectedId(item.deliverableId)} type="button"><span className="deliverable-card-top"><i data-status={item.status}/>{deliverableStatus(item.status)}</span><strong>{item.title}</strong><span>{item.ownerLabel} · {item.versions.length} 个版本</span></button>)}{!props.deliverables.length && !showCreate ? <div className="deliverable-empty"><DeliverableIcon/><strong>尚无交付物</strong><span>先定义项目最终要验收的输出。</span><button onClick={() => setShowCreate(true)} type="button">新建交付物</button></div> : null}</div>
        </aside>
        <main className="deliverable-detail-panel">{selected ? <>
          <header className="deliverable-detail-header"><div><span className="deliverable-type">{selected.deliverableType}</span><h2>{selected.title}</h2><p>{selected.ownerLabel} · {deliverableStatus(selected.status)}</p></div><span>修订 {selected.stateRevision}</span></header>
          <section className="acceptance-criteria"><small>验收标准</small><p>{selected.acceptanceCriteria}</p></section>
          {!candidate ? <section className="candidate-composer"><div><h3>提交候选版本</h3><p>将本地文件快照绑定为这一版的验收证据。</p></div><input onChange={(event) => setSummary(event.target.value)} placeholder="版本说明" value={summary}/><button className="primary" disabled={busy || !summary.trim() || selectedFiles.size === 0} onClick={() => void createVersion()} type="button">提交候选</button><small>{selectedFiles.size ? `已选择 ${selectedFiles.size} 个文件` : "至少选择一个本地文件"}</small></section> : <section className="candidate-pending"><strong>V{candidate.versionNo} 正在等待验收</strong><span>验收完成前不能创建新的候选版本。</span></section>}
          {!candidate ? <section className="deliverable-file-picker"><header><h3>选择版本文件</h3><input aria-label="搜索项目文件" onChange={(event) => setFileQuery(event.target.value)} placeholder="搜索文件路径" value={fileQuery}/></header><div>{availableFiles.map((file) => <label key={file.localFileId}><input checked={selectedFiles.has(file.localFileId)} onChange={(event) => setSelectedFiles((current) => { const next = new Set(current); if (event.target.checked) next.add(file.localFileId); else next.delete(file.localFileId); return next; })} type="checkbox"/><span>{file.relativePath}</span><button aria-label={`在系统默认应用中打开 ${file.relativePath}`} onClick={(event) => { event.preventDefault(); void openFile(file.relativePath, "default"); }} type="button">打开</button></label>)}</div></section> : null}
          {candidate ? <section className="review-card"><h3>验收候选版本 V{candidate.versionNo}</h3><p>{candidate.summary}</p><label>验收人<input onChange={(event) => setReviewer(event.target.value)} value={reviewer}/></label><label>验收结论<textarea onChange={(event) => setReviewComment(event.target.value)} rows={3} value={reviewComment}/></label><div>{candidate.bindings.map((binding) => <button key={binding.localFileId} onClick={() => void openFile(binding.relativePath, "default")} type="button">{binding.relativePath}</button>)}</div><footer><button disabled={busy || !reviewer.trim() || !reviewComment.trim()} onClick={() => void reviewVersion(candidate, "rejected")} type="button">退回修改</button><button className="primary" disabled={busy || !reviewer.trim() || !reviewComment.trim()} onClick={() => void reviewVersion(candidate, "accepted")} type="button">验收通过</button></footer></section> : null}
          <section className="version-history"><header><h3>版本记录</h3><span>{selected.versions.length} 个</span></header>{selected.versions.length ? selected.versions.map((version) => <article className="version-card" key={version.versionId}><header><strong>V{version.versionNo} · {versionStatus(version.status)}</strong><time>{auditMoment(version.createdAt)}</time></header><p>{version.summary}</p><ul>{version.carriers.filter((carrier) => carrier.reference.startsWith("cilai-artifact:")).map((carrier) => <li key={carrier.carrierId}><FileIcon/><button type="button" onClick={() => { const [runId, artifactId] = carrier.reference.slice("cilai-artifact:".length).split("/"); void invoke("ai_artifact_open", { runId, artifactId, openMode: "default" }).catch((error) => props.onMessage(String(error), "error")); }}>{carrier.label} · {carrier.versionLabel.slice(0, 12)}</button></li>)}{version.bindings.map((binding) => <li key={binding.localFileId}><FileIcon/><button onClick={() => void openFile(binding.relativePath, "default")} type="button">{binding.relativePath}</button>{platformUi && props.workspace?.workspaceKind !== "cloud_team" ? <button onClick={() => void openFile(binding.relativePath, "finder")} type="button">{platformUi.fileManagerName}</button> : null}</li>)}</ul></article>) : <p>还没有候选版本。</p>}</section>
        </> : <div className="deliverable-detail-empty"><DeliverableIcon/><strong>选择一个交付物</strong><span>查看验收标准、候选版本和历史记录。</span></div>}</main>
      </div>
    </section>
  </div>;
}

function deliverableStatus(status: string) { return status === "accepted" ? "已验收" : status === "revision_needed" ? "需修改" : status === "candidate" ? "待验收" : "已定义"; }
function versionStatus(status: string) { return status === "accepted" ? "已验收" : status === "rejected" ? "已退回" : "候选"; }
function DeliverableIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M5 3h10l4 4v14H5z"/><path d="M15 3v5h5M8 13h8M8 17h6"/></svg>; }
function FileIcon() { return <svg aria-hidden="true" viewBox="0 0 20 20"><path d="M5 2.5h6l4 4v11H5z"/><path d="M11 2.5v4h4"/></svg>; }
function PlusIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>; }
function CloseIcon() { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m7 7 10 10M17 7 7 17"/></svg>; }
