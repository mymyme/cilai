import { useCallback, useEffect, useRef, useState } from "react";
import type { PersonalPackageImportCommitResult, PersonalPackageImportPreview, PersonalPackageImportTarget } from "./generated/protocol";
import { invoke } from "./host";
import { createPortal } from "./page-portal";
import { describeServerError } from "./server-error";
import { useModal } from "./use-modal";
import { isPersonalPageAiManifest, pageAiChanged, pageAiPermissionsMatch, PersonalPageAiDeclaration, personalPermissionLabel } from "./personal-page-ai-declaration";
import "./personal-capability-editor.css";
import "./personal-package-import.css";

const sameTarget = (a: PersonalPackageImportTarget, b: PersonalPackageImportTarget) => a.mode === b.mode
  && (a.mode === "new" || b.mode === "upgrade" && a.creationProjectId === b.creationProjectId && a.expectedRevision === b.expectedRevision);
function checkPreview(value: PersonalPackageImportPreview, profileId?: string) {
  if (!value || profileId && value.profileId !== profileId || value.source?.authenticated !== false || value.source.formatVersion !== 1
    || !["prepared", "committed", "cancelled"].includes(value.state)
    || !/^capability_[a-f0-9]{32}$/.test(value.localCreationProjectId)
    || !/^sha256:[a-f0-9]{64}$/.test(value.localPackageDigest)
    || typeof value.importId !== "string" || !value.importId || typeof value.previewDigest !== "string" || !value.previewDigest
    || typeof value.source.title !== "string" || typeof value.source.version !== "string" || typeof value.source.pluginId !== "string"
    || !Number.isSafeInteger(value.source.sourceBytes) || value.source.sourceBytes < 0
    || !Number.isSafeInteger(value.source.sourceFileCount) || value.source.sourceFileCount < 1
    || !value.target || !["new", "upgrade"].includes(value.target.mode)
    || !Array.isArray(value.requestedPermissions) || value.requestedPermissions.some(item => typeof item !== "string")
    || !pageAiPermissionsMatch(value.source.targetType, value.requestedPermissions, value.source.pageAi)
    || value.installedPageAi != null && !isPersonalPageAiManifest(value.installedPageAi)
    || new Set(value.requestedPermissions).size !== value.requestedPermissions.length
    || !Array.isArray(value.addedPermissions) || value.addedPermissions.some(item => !value.requestedPermissions.includes(item))) {
    throw new Error("personal_package_import_response_invalid");
  }
  return value;
}

export function PersonalPackageImport(props: {
  target: PersonalPackageImportTarget;
  profileId?: string;
  targetTitle?: string;
  onClose: () => void;
  onChanged: (projectId?: string) => Promise<void>;
}) {
  const [records, setRecords] = useState<PersonalPackageImportPreview[]>([]);
  const [preview, setPreview] = useState<PersonalPackageImportPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [reading, setReading] = useState(true);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [confirmed, setConfirmed] = useState(false);
  const [pageConfirmed, setPageConfirmed] = useState(false);
  const [unconfirmed, setUnconfirmed] = useState(false);
  const [listNeedsRefresh, setListNeedsRefresh] = useState(false);
  const mounted = useRef(true);
  const active = useRef(false);
  const sequence = useRef(0);
  const modal = useModal<HTMLDivElement>({ onClose: props.onClose, busy });
  const readRecords = useCallback(async () => {
    const seq = ++sequence.current;
    setReading(true);
    try {
      const result = await invoke<PersonalPackageImportPreview[]>("personal_package_import_list", { request: {} });
      if (!Array.isArray(result)) throw new Error("personal_package_import_response_invalid");
      const checked = result.map(item => checkPreview(item, props.profileId));
      if (mounted.current && seq === sequence.current) setRecords(checked);
    } catch (reason) {
      if (mounted.current && seq === sequence.current) setError(describeServerError(reason));
    } finally { if (mounted.current && seq === sequence.current) setReading(false); }
  }, [props.profileId]);
  useEffect(() => {
    mounted.current = true; void readRecords();
    return () => { mounted.current = false; ++sequence.current; };
  }, [readRecords]);
  const accept = (value: PersonalPackageImportPreview) => {
    setPreview(value); setConfirmed(false); setPageConfirmed(false); setUnconfirmed(false); setNotice(""); setError("");
    setListNeedsRefresh(value.state === "committed");
    setRecords(previous => [value, ...previous.filter(item => item.importId !== value.importId)]);
  };
  const run = async (action: () => Promise<void>) => {
    if (active.current) return;
    active.current = true; setBusy(true); setError(""); setNotice("");
    try { await action(); }
    catch (reason) { if (mounted.current) setError(describeServerError(reason)); }
    finally { active.current = false; if (mounted.current) setBusy(false); }
  };
  const choose = () => run(async () => {
    const value = await invoke<PersonalPackageImportPreview | null>("personal_package_import_prepare", { request: {
      target: props.target, clientOperationId: crypto.randomUUID(),
    } });
    if (!mounted.current || value === null) return;
    const checked = checkPreview(value, props.profileId);
    if (!sameTarget(checked.target, props.target)) throw new Error("personal_package_import_response_invalid");
    accept(checked);
  });
  const inspect = (record: PersonalPackageImportPreview) => run(async () => {
    const value = checkPreview(await invoke<PersonalPackageImportPreview>("personal_package_import_get", { request: { importId: record.importId } }), props.profileId);
    if (!mounted.current) return;
    if (value.importId !== record.importId || value.previewDigest !== record.previewDigest || !sameTarget(value.target, record.target)) {
      throw new Error("personal_package_import_response_invalid");
    }
    accept(value);
  });
  const commit = () => run(async () => {
    if (!preview || preview.state !== "prepared" || !confirmed || unconfirmed
      || pageAiChanged(preview.installedPageAi, preview.source.pageAi) && !pageConfirmed) return;
    const selected = preview;
    setConfirmed(false); setUnconfirmed(true);
    const value = await invoke<PersonalPackageImportCommitResult>("personal_package_import_commit", { request: {
      importId: selected.importId, previewDigest: selected.previewDigest,
      confirmedPermissions: selected.requestedPermissions, clientOperationId: `personal-import-${selected.importId}`,
    } });
    if (!mounted.current) return;
    if (value.importId !== selected.importId || value.localPackageDigest !== selected.localPackageDigest
      || value.project.creationProjectId !== selected.localCreationProjectId) throw new Error("personal_package_import_response_invalid");
    accept({ ...selected, state: "committed" });
    setNotice("导入已确认");
    try { await props.onChanged(selected.localCreationProjectId); if (mounted.current) setListNeedsRefresh(false); }
    catch { if (mounted.current) { setListNeedsRefresh(true); setNotice("导入已确认，应用列表尚未刷新。"); } }
  });
  const cancel = () => run(async () => {
    if (!preview || preview.state !== "prepared" || unconfirmed) return;
    const selected = preview;
    setConfirmed(false); setUnconfirmed(true);
    const value = checkPreview(await invoke<PersonalPackageImportPreview>("personal_package_import_cancel", { request: {
      importId: selected.importId, previewDigest: selected.previewDigest,
    } }), props.profileId);
    if (!mounted.current) return;
    if (value.importId !== selected.importId || value.previewDigest !== selected.previewDigest || value.state !== "cancelled") {
      throw new Error("personal_package_import_response_invalid");
    }
    accept(value); setNotice("已取消导入");
  });
  const refreshApps = () => run(async () => { await props.onChanged(preview?.localCreationProjectId); if (mounted.current) { setListNeedsRefresh(false); setNotice("应用列表已刷新"); } });
  const pending = records.filter(item => item.state === "prepared");
  return createPortal(<div className="agent-picker-backdrop" ref={modal} role="dialog" aria-modal="true" aria-label="导入应用">
    <section className="personal-editor personal-package-import">
      <header><h3>导入应用</h3><button type="button" disabled={busy} onClick={props.onClose}>关闭</button></header>
      <div className="personal-editor-toolbar"><button type="button" disabled={busy || reading} onClick={() => void choose()}>选择安装包</button>
        <span>{props.target.mode === "upgrade" ? `升级：${props.targetTitle ?? props.target.creationProjectId}` : "新建个人应用"}</span>
        <button type="button" disabled={busy || reading} onClick={() => { setError(""); void readRecords(); }}>刷新记录</button></div>
      {pending.length ? <section className="personal-import-records" aria-label="未完成导入"><h4>未完成</h4>{pending.map(item => <button type="button" disabled={busy || reading} key={item.importId}
        onClick={() => void inspect(item)}>{item.source.title}<small>{item.target.mode === "new" ? "新建" : "升级"} · {item.localVersion}</small></button>)}</section> : null}
      {preview ? <section className="personal-import-preview" aria-label="安装包预检">
        <header><h4>{preview.source.title}</h4><span>{({ prepared: "待确认", committed: "已安装", cancelled: "已取消" })[preview.state]}</span></header>
        <p className="personal-editor-notice">来源未认证</p>
        <dl><dt>本次操作</dt><dd>{preview.target.mode === "new" ? "新建个人应用" : `升级 ${preview.localCreationProjectId}`}</dd>
          <dt>来源版本</dt><dd>{preview.source.version}</dd><dt>本地版本</dt><dd>{preview.localVersion}</dd>
          <dt>包标识</dt><dd>{preview.source.pluginId}</dd><dt>文件</dt><dd>{preview.source.sourceFileCount} 项 · {preview.source.sourceBytes.toLocaleString()} 字节</dd>
          <dt>安装包摘要</dt><dd>{preview.source.archiveDigest}</dd></dl>
        {preview.source.summary ? <p>{preview.source.summary}</p> : null}
        {!preview.source.fullBusinessGoalPresent || !preview.source.acceptanceExamplesPresent ? <p className="personal-editor-notice">来源包未包含完整目标或验收样例。</p> : null}
        <h4>权限</h4>{preview.requestedPermissions.length ? <ul>{preview.requestedPermissions.map(item => <li key={item}>{personalPermissionLabel(item)}{preview.addedPermissions.includes(item) ? "（新增）" : ""}</li>)}</ul> : <p>未申请权限。</p>}
        <PersonalPageAiDeclaration before={preview.installedPageAi} after={preview.source.pageAi}/>
        {preview.state === "prepared" ? <>
          <p>确认后停止受管任务，以独立数据副本安装；原版本与数据保留。</p>
          {unconfirmed ? <p role="status">结果尚未确认，请先核对记录。</p> : <label className="personal-import-confirm"><input type="checkbox" disabled={busy} checked={confirmed} onChange={event => setConfirmed(event.target.checked)}/>我已核对来源、版本和权限</label>}
          {!unconfirmed && pageAiChanged(preview.installedPageAi, preview.source.pageAi) ? <label className="personal-page-ai-confirm"><input type="checkbox" disabled={busy} checked={pageConfirmed} onChange={event => setPageConfirmed(event.target.checked)}/>我已核对页面 AI 声明变更</label> : null}
          <footer><button type="button" disabled={busy || unconfirmed} onClick={() => void cancel()}>取消导入</button>
            <button type="button" disabled={busy} onClick={() => void inspect(preview)}>核对</button>
            <button type="button" className="primary" disabled={busy || !confirmed || unconfirmed || pageAiChanged(preview.installedPageAi, preview.source.pageAi) && !pageConfirmed} onClick={() => void commit()}>确认安装</button></footer>
        </> : null}
      </section> : null}
      {reading || busy ? <p role="status">{reading ? "正在读取…" : "正在处理…"}</p> : null}
      {error ? <p className="personal-editor-notice" data-tone="danger" role="alert">{error}</p> : null}
      {notice ? <p role="status">{notice}</p> : null}
      {listNeedsRefresh ? <button type="button" disabled={busy} onClick={() => void refreshApps()}>刷新应用</button> : null}
    </section>
  </div>, document.body);
}
