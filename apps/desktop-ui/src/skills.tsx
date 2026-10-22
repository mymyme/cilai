import { MarkdownContent } from "./markdown-content";
import { reuseInstructions, type WorkReuseIntent } from "./work-reuse";
import { useCallback, useEffect, useId, useMemo, useRef, useState, type FormEvent } from "react";
import { createPortal } from "./page-portal";
import { usePageActive } from "./page-visibility";
import type {
  SkillDetail,
  SkillGetResult,
  SkillImportResult,
  SkillListResult,
  SkillSaveResult,
  SkillSetStateResult,
  SkillSummary,
} from "./generated/protocol";
import { rpc, sha256 } from "./app-server-rpc";
import type { MessageSink } from "./app-message";
import { ArchiveIcon, ChevronIcon, CloseIcon, FileIcon, PencilIcon, PlusIcon, MoreIcon, RefreshIcon, SkillIcon, SparkIcon, TrashIcon, TreeFolderIcon } from "./icons";
import { ConfirmDialog } from "./confirm-dialog";
import { SearchField } from "./search-field";
import { describeServerError } from "./server-error";
import { invoke } from "./host";
import { useModal } from "./use-modal";
import { randomUuid } from "./uuid";
import "./skills.css";

type EditorDraft = {
  name: string;
  description: string;
  instructions: string;
  enabled: boolean;
  automatic: boolean;
};

const emptyDraft: EditorDraft = {
  name: "",
  description: "",
  instructions: "",
  enabled: true,
  automatic: false,
};

const skillTabs = [
  { source: "user", label: "我的技能" },
  { source: "system", label: "系统技能" },
] as const;

export function SkillsPage(props: {
  onMessage: MessageSink;
  openSkillId?: string | null;
  reuseIntent?: WorkReuseIntent | null;
  onReuseConsumed?: () => void;
  onOpenConversation: (skillId: string, skillName: string, mode: "try" | "create") => void;
}) {
  const pageActive = usePageActive();
  const initiallyLoaded = useRef(false);
  const [skills, setSkills] = useState<SkillSummary[]>([]);
  const [activeSource, setActiveSource] = useState<"user" | "system">("user");
  const tabsId = useId();
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [busySkillId, setBusySkillId] = useState<string | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [detail, setDetail] = useState<SkillDetail | null>(null);
  const [editor, setEditor] = useState<{ skill: SkillDetail | null; draft: EditorDraft } | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<SkillDetail | null>(null);
  const consumedReuse = useRef<string | null>(null);
  useEffect(() => {
    const intent = props.reuseIntent;
    if (!intent || consumedReuse.current === intent.token) return;
    consumedReuse.current = intent.token;
    setActiveSource("user");
    setEditor({ skill: null, draft: { name: intent.title, description: "", instructions: reuseInstructions(intent), enabled: false, automatic: false } });
    props.onReuseConsumed?.();
  }, [props.reuseIntent]);


  const load = useCallback(async (background = false) => {
    if (!background) setLoading(true);
    try {
      const result = await rpc<SkillListResult>("skill/list", {});
      setSkills(result.skills);
      initiallyLoaded.current = true;
      setLoadError(null);
    } catch (error) {
      setLoadError(describeServerError(error));
    } finally {
      setLoading(false);
    }
  }, [props.onMessage]);

  useEffect(() => {
    if (!pageActive) return;
    if (!initiallyLoaded.current) void load();
    const timer = setInterval(() => void load(true), 2000);
    return () => clearInterval(timer);
  }, [load, pageActive]);
  useEffect(() => {
    if (!props.openSkillId) return;
    let active = true;
    void rpc<SkillGetResult>("skill/get", { skillId: props.openSkillId }).then((result) => {
      if (active) {
        setActiveSource("user");
        setDetail(result.skill);
      }
    }).catch((error) => { if (active) props.onMessage(describeServerError(error), "error"); });
    return () => { active = false; };
  }, [props.openSkillId, props.onMessage]);

  const visibleSkills = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return skills.filter((skill) => skill.source === activeSource
      && (!normalized || `${skill.name}\n${skill.description}`.toLocaleLowerCase().includes(normalized)));
  }, [activeSource, query, skills]);

  const openDetail = async (skill: SkillSummary) => {
    setBusySkillId(skill.skillId);
    try {
      const result = await rpc<SkillGetResult>("skill/get", { skillId: skill.skillId });
      setDetail(result.skill);
    } catch (error) {
      props.onMessage(describeServerError(error), "error");
    } finally {
      setBusySkillId(null);
    }
  };

  const setState = async (skill: SkillSummary, next: { enabled: boolean; automatic: boolean }) => {
    setBusySkillId(skill.skillId);
    try {
      const operation = randomUuid();
      const fingerprint = await sha256(JSON.stringify({
        skillId: skill.skillId,
        expectedRevision: skill.stateRevision,
        enabled: next.enabled,
        automatic: next.automatic,
      }));
      const result = await rpc<SkillSetStateResult>("skill/setState", {
        skillId: skill.skillId,
        enabled: next.enabled,
        automatic: next.automatic,
        expectedRevision: skill.stateRevision,
        clientOperationId: operation,
        requestFingerprint: fingerprint,
      });
      setSkills((current) => current.map((item) => item.skillId === skill.skillId ? result.skill : item));
      setDetail((current) => current?.skillId === skill.skillId ? {
        ...current,
        enabled: result.skill.enabled,
        automatic: result.skill.automatic,
        stateRevision: result.skill.stateRevision,
        updatedAt: result.skill.updatedAt,
      } : current);
    } catch (error) {
      props.onMessage(describeServerError(error), "error");
      await load();
    } finally {
      setBusySkillId(null);
    }
  };

  const beginEdit = (skill: SkillDetail | null) => {
    setEditor({
      skill,
      draft: skill ? {
        name: skill.name,
        description: skill.description,
        instructions: skill.instructions,
        enabled: skill.enabled,
        automatic: skill.automatic,
      } : emptyDraft,
    });
    setDetail(null);
    setMenuOpen(false);
  };

  const save = async (draft: EditorDraft) => {
    const current = editor?.skill ?? null;
    const operation = randomUuid();
    const fingerprint = await sha256(JSON.stringify({
      skillId: current?.skillId ?? null,
      expectedRevision: current?.stateRevision ?? null,
      name: draft.name.trim(),
      description: draft.description.trim(),
      instructions: draft.instructions.trim(),
      files: current ? null : [],
      enabled: draft.enabled,
      automatic: draft.automatic,
    }));
    const result = await rpc<SkillSaveResult>("skill/save", {
      skillId: current?.skillId ?? null,
      name: draft.name.trim(),
      description: draft.description.trim(),
      instructions: draft.instructions.trim(),
      files: current ? null : [],
      enabled: draft.enabled,
      automatic: draft.automatic,
      expectedRevision: current?.stateRevision ?? null,
      clientOperationId: operation,
      requestFingerprint: fingerprint,
    });
    const summary: SkillSummary = {
      skillId: result.skill.skillId,
      name: result.skill.name,
      description: result.skill.description,
      source: result.skill.source,
      enabled: result.skill.enabled,
      automatic: result.skill.automatic,
      stateRevision: result.skill.stateRevision,
      updatedAt: result.skill.updatedAt,
    };
    setSkills((items) => {
      const exists = items.some((item) => item.skillId === summary.skillId);
      return exists
        ? items.map((item) => item.skillId === summary.skillId ? summary : item)
        : [summary, ...items];
    });
    setEditor(null);
    setActiveSource("user");
    setQuery("");
    setDetail(result.skill);
    props.onMessage(current ? "技能已保存。" : "技能已创建。");
  };

  const importArchive = async () => {
    setMenuOpen(false);
    try {
      const archivePath = await invoke<string | null>("skill_pick_archive");
      if (!archivePath) return;
      setImporting(true);
      const clientOperationId = `desktop-skill-import-${randomUuid()}`;
      const requestFingerprint = await sha256(JSON.stringify({ archivePath }));
      const result = await rpc<SkillImportResult>("skill/import", {
        archivePath,
        clientOperationId,
        requestFingerprint,
      });
      const summary: SkillSummary = {
        skillId: result.skill.skillId,
        name: result.skill.name,
        description: result.skill.description,
        source: result.skill.source,
        enabled: result.skill.enabled,
        automatic: result.skill.automatic,
        stateRevision: result.skill.stateRevision,
        updatedAt: result.skill.updatedAt,
      };
      setSkills((items) => [summary, ...items.filter((item) => item.skillId !== summary.skillId)]);
      setActiveSource("user");
      setQuery("");
      setDetail(result.skill);
      props.onMessage(`技能“${result.skill.name}”已安装。`);
    } catch (error) {
      props.onMessage(describeServerError(error), "error");
    } finally {
      setImporting(false);
    }
  };

  const remove = async (skill: SkillDetail) => {
    setBusySkillId(skill.skillId);
    try {
      const operation = randomUuid();
      const fingerprint = await sha256(JSON.stringify({ skillId: skill.skillId, expectedRevision: skill.stateRevision }));
      await rpc("skill/delete", {
        skillId: skill.skillId,
        expectedRevision: skill.stateRevision,
        clientOperationId: operation,
        requestFingerprint: fingerprint,
      });
      setSkills((current) => current.filter((item) => item.skillId !== skill.skillId));
      setDetail(null);
      setDeleteTarget(null);
      props.onMessage("技能已删除。");
    } catch (error) {
      props.onMessage(describeServerError(error), "error");
      await load();
    } finally {
      setBusySkillId(null);
    }
  };

  return <section className="skills-page">
    <header className="skills-header">
      <div>
        <h1>技能</h1>
      </div>
      <div className="skills-toolbar">
        {activeSource === "user" ? <div className="skills-add">
          <button aria-expanded={menuOpen} className="primary" disabled={importing} onClick={() => setMenuOpen((value) => !value)} type="button"><PlusIcon/>{importing ? "安装中…" : "添加"}</button>
          {menuOpen ? <div className="skills-add-menu" role="menu">
            <button onClick={() => { setMenuOpen(false); props.onOpenConversation("system.skill-creator", "技能创建器", "create"); }} role="menuitem" type="button"><SparkIcon/><span><strong>用 AI 创建</strong><small>对话创建</small></span></button>
            <button onClick={() => void importArchive()} role="menuitem" type="button"><ArchiveIcon/><span><strong>导入 ZIP</strong><small>安装已有技能包</small></span></button>
            <button onClick={() => beginEdit(null)} role="menuitem" type="button"><PencilIcon/><span><strong>手动创建</strong><small>直接填写说明和规则</small></span></button>
          </div> : null}
        </div> : null}
        <button aria-label="刷新" className="skills-icon-button" disabled={loading} onClick={() => void load()} title="刷新" type="button"><RefreshIcon/></button>
      </div>
    </header>

    <SearchField ariaLabel="搜索技能" className="skills-search" clearable clearLabel="清空关键词" onChange={setQuery} placeholder="搜索技能" value={query}/>
    {loadError ? <div role="status">技能列表读取失败：{loadError}</div> : null}
    <div aria-label="技能分类" className="skills-tabs" role="tablist">
      {skillTabs.map((tab, index) => <button
        aria-controls={`${tabsId}-panel`}
        aria-selected={activeSource === tab.source}
        id={`${tabsId}-tab-${tab.source}`}
        key={tab.source}
        onClick={() => { setActiveSource(tab.source); setMenuOpen(false); }}
        onKeyDown={(event) => {
          let next: number;
          if (event.key === "ArrowRight") next = (index + 1) % skillTabs.length;
          else if (event.key === "ArrowLeft") next = (index + skillTabs.length - 1) % skillTabs.length;
          else if (event.key === "Home") next = 0;
          else if (event.key === "End") next = skillTabs.length - 1;
          else return;
          event.preventDefault();
          setActiveSource(skillTabs[next].source);
          setMenuOpen(false);
          tabRefs.current[next]?.focus();
        }}
        ref={(element) => { tabRefs.current[index] = element; }}
        role="tab"
        tabIndex={activeSource === tab.source ? 0 : -1}
        type="button"><strong>{tab.label}</strong><span aria-hidden="true">{skills.filter((skill) => skill.source === tab.source).length}</span></button>)}
    </div>
    <div aria-labelledby={`${tabsId}-tab-${activeSource}`} className="skills-panel" id={`${tabsId}-panel`} role="tabpanel" tabIndex={0}>
    {loading ? <div className="skills-empty" role="status">正在加载…</div>
      : visibleSkills.length ? <div className="skills-list">{visibleSkills.map((skill) => {
        const content = <>
          <span className="skill-mark"><SkillIcon/></span>
          <span className="skill-copy"><strong>{skill.name}</strong><small>{skill.description}</small>{skill.source === "user" && skill.automatic ? <em>允许意图调用</em> : null}</span>
        </>;
        return <article className="skill-row" data-source={skill.source} key={skill.skillId}>
          {skill.source === "user" ? <>
            <button className="skill-open" disabled={busySkillId === skill.skillId} onClick={() => void openDetail(skill)} type="button">{content}</button>
            <Toggle checked={skill.enabled} disabled={busySkillId === skill.skillId} label={`${skill.name}已${skill.enabled ? "启用" : "停用"}`} onChange={(enabled) => void setState(skill, { enabled, automatic: skill.automatic })}/>
          </> : <div className="skill-summary">{content}</div>}
        </article>;
      })}</div>
      : <div className="skills-empty"><SkillIcon/><strong>{query.trim() ? "没有匹配的技能" : activeSource === "user" ? "还没有个人技能" : "暂无系统技能"}</strong>{query.trim()
        ? <button onClick={() => setQuery("")} type="button">清除搜索</button>
        : activeSource === "user" ? <><span>创建后可按意图调用，也可输入 / 选择</span><button onClick={() => beginEdit(null)} type="button">手动创建</button></> : null}</div>}
    </div>

    {detail ? <SkillDetailDialog
      busy={busySkillId === detail.skillId}
      suspended={deleteTarget !== null}
      skill={detail}
      onClose={() => setDetail(null)}
      onDelete={() => setDeleteTarget(detail)}
      onEdit={() => beginEdit(detail)}
      onState={(next) => void setState(detail, next)}
      onTry={() => props.onOpenConversation(detail.skillId, detail.name, "try")}/> : null}
    {editor ? <SkillEditorDialog
      initial={editor.draft}
      title={editor.skill ? "编辑技能" : "创建技能"}
      onCancel={() => setEditor(null)}
      onError={(error) => props.onMessage(describeServerError(error), "error")}
      onSave={save}/> : null}
    {deleteTarget ? <ConfirmDialog
      busy={busySkillId === deleteTarget.skillId}
      confirmLabel="删除"
      danger
      description={<p>将删除「{deleteTarget.name}」及其受管技能文件。删除后不能在对话中继续使用。</p>}
      onCancel={() => setDeleteTarget(null)}
      onConfirm={() => void remove(deleteTarget)}
      title="删除这个技能？"/> : null}
  </section>;
}

function SkillDetailDialog(props: {
  skill: SkillDetail;
  busy: boolean;
  suspended: boolean;
  onClose: () => void;
  onDelete: () => void;
  onEdit: () => void;
  onState: (state: { enabled: boolean; automatic: boolean }) => void;
  onTry: () => void;
}) {
  const modalRef = useModal<HTMLDivElement>({ onClose: props.onClose, busy: props.busy, active: !props.suspended });
  const [detailTab, setDetailTab] = useState("overview");
  const [selectedPath, setSelectedPath] = useState("SKILL.md");
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  useEffect(() => { setSelectedPath("SKILL.md"); setCollapsedGroups(new Set()); }, [props.skill.skillId]);
  const selectedFile = props.skill.files.find((file) => file.path === selectedPath) ?? null;
  const manifestContent = useMemo(() => renderSkillManifest(props.skill), [props.skill]);
  const selectedContent = selectedFile?.content ?? manifestContent;
  const groupedFiles = useMemo(() => ["scripts", "references", "assets"].map((category) => ({
    category,
    files: props.skill.files.filter((file) => file.category === category),
  })).filter((group) => group.files.length), [props.skill.files]);
  return createPortal(<div aria-label={props.skill.name} aria-modal={props.suspended ? undefined : true} aria-hidden={props.suspended || undefined} inert={props.suspended} className="skills-dialog-backdrop" ref={modalRef} role="dialog" onMouseDown={(event) => { if (event.target === event.currentTarget && !props.busy && !props.suspended) props.onClose(); }}>
    <section className="skill-detail-dialog">
      <header>
        <span className="skill-mark"><SkillIcon/></span>
        <div><h2>{props.skill.name}</h2><p>{props.skill.description}</p></div>
        <div className="skill-availability-control"><span data-enabled={props.skill.enabled}>{props.skill.enabled ? "已启用" : "已停用"}</span><button disabled={props.busy} onClick={() => props.onState({ enabled: !props.skill.enabled, automatic: props.skill.automatic })} type="button">{props.skill.enabled ? "停用" : "启用"}</button></div>
        <button aria-label="关闭" className="skills-close" disabled={props.busy} onClick={props.onClose} type="button"><CloseIcon/></button>
      </header>
      <nav className="skill-detail-tabs" aria-label="技能详情视图"><button aria-pressed={detailTab === "overview"} onClick={() => setDetailTab("overview")} type="button">概览</button><button aria-pressed={detailTab === "files"} onClick={() => setDetailTab("files")} type="button">文件</button></nav>
      <div className="skill-detail-body">
        {detailTab === "overview" ? <section className="skill-overview"><h3>使用说明</h3><MarkdownContent>{props.skill.instructions}</MarkdownContent><p>进入对话后选择项目和模型，再发送任务。</p></section> : <section>
          <div className="skill-package-heading"><h3>技能包</h3><span>{props.skill.files.length ? `${props.skill.files.length} 个资源文件` : "仅主文件"}</span></div>
          <div className="skill-package-browser">
            <nav aria-label="技能文件">
              <button aria-current={selectedPath === "SKILL.md" ? "page" : undefined} className="skill-file-item skill-root-file" onClick={() => setSelectedPath("SKILL.md")} type="button"><FileIcon/><span>SKILL.md</span></button>
              {groupedFiles.map((group) => <section className="skill-file-group" key={group.category}>
                <button aria-expanded={!collapsedGroups.has(group.category)} className="skill-folder-item" onClick={() => setCollapsedGroups((current) => { const next = new Set(current); if (next.has(group.category)) next.delete(group.category); else next.add(group.category); return next; })} type="button"><ChevronIcon collapsed={collapsedGroups.has(group.category)}/><TreeFolderIcon/><strong>{group.category}</strong></button>
                {!collapsedGroups.has(group.category) ? group.files.map((file) => <button aria-current={selectedPath === file.path ? "page" : undefined} className="skill-file-item" key={file.path} onClick={() => setSelectedPath(file.path)} title={file.path} type="button"><FileIcon/><span>{file.path.slice(file.path.lastIndexOf("/") + 1)}</span></button>) : null}
              </section>)}
            </nav>
            <section className="skill-file-preview">
              <header><div><FileIcon/><strong>{selectedPath}</strong></div><span>{formatBytes(selectedFile?.sizeBytes ?? new TextEncoder().encode(manifestContent).length)}</span></header>
              <pre>{selectedContent}</pre>
            </section>
          </div>
        </section>}
        <section className="skill-auto-setting"><div><h3>允许意图调用</h3><p>开启后由模型按任务判断；关闭后仍可输入 / 选择。</p></div><div className="skill-state-control"><span>{props.skill.enabled ? (props.skill.automatic ? "已开启" : "未开启") : (props.skill.automatic ? "已保留" : "不可用")}</span><Toggle checked={props.skill.automatic} disabled={props.busy || !props.skill.enabled} label={`意图调用已${props.skill.automatic ? "开启" : "关闭"}`} onChange={(automatic) => props.onState({ enabled: props.skill.enabled, automatic })}/></div></section>
      </div>
      <footer>
        <div><button disabled={props.busy} onClick={props.onEdit} type="button"><PencilIcon/>编辑</button><details className="inline-more"><summary aria-label="更多技能操作"><MoreIcon/></summary><button className="danger-quiet" disabled={props.busy} onClick={props.onDelete} type="button"><TrashIcon/>删除</button></details></div>
        <button className="primary" disabled={props.busy || !props.skill.enabled} onClick={props.onTry} type="button">在对话中试用</button>
      </footer>
    </section>
  </div>, document.body);
}

function SkillEditorDialog(props: {
  title: string;
  initial: EditorDraft;
  onCancel: () => void;
  onSave: (draft: EditorDraft) => Promise<void>;
  onError: (error: unknown) => void;
}) {
  const [draft, setDraft] = useState(props.initial);
  const [busy, setBusy] = useState(false);
  const modalRef = useModal<HTMLDivElement>({ onClose: props.onCancel, busy });
  const valid = draft.name.trim() && draft.description.trim() && draft.instructions.trim();
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!valid || busy) return;
    setBusy(true);
    try { await props.onSave(draft); } catch (error) { props.onError(error); setBusy(false); }
  };
  return createPortal(<div aria-label={props.title} aria-modal="true" className="skills-dialog-backdrop" ref={modalRef} role="dialog">
    <form className="skill-editor-dialog" onSubmit={(event) => void submit(event)}>
      <header><div><span>个人技能</span><h2>{props.title}</h2></div><button aria-label="关闭" className="skills-close" disabled={busy} onClick={props.onCancel} type="button"><CloseIcon/></button></header>
      <div className="skill-editor-fields">
        <label>名称<input autoFocus disabled={busy} maxLength={80} onChange={(event) => setDraft({ ...draft, name: event.target.value })} placeholder="例如：周报整理" value={draft.name}/></label>
        <label>简介<input disabled={busy} maxLength={240} onChange={(event) => setDraft({ ...draft, description: event.target.value })} placeholder="一句话说明这个技能做什么" value={draft.description}/></label>
        <label>说明<textarea disabled={busy} maxLength={50_000} onChange={(event) => setDraft({ ...draft, instructions: event.target.value })} placeholder="写清适用范围、输入、步骤、输出和限制" rows={13} value={draft.instructions}/></label>
        <div className="skill-editor-options">
          <label><input checked={draft.enabled} disabled={busy} onChange={(event) => setDraft({ ...draft, enabled: event.target.checked })} type="checkbox"/><span><strong>启用</strong><small>可输入 / 选择</small></span></label>
          <label><input checked={draft.automatic} disabled={busy || !draft.enabled} onChange={(event) => setDraft({ ...draft, automatic: event.target.checked })} type="checkbox"/><span><strong>允许意图调用</strong><small>由模型按任务判断</small></span></label>
        </div>
      </div>
      <footer><button disabled={busy} onClick={props.onCancel} type="button">取消</button><button className="primary" disabled={busy || !valid} type="submit">{busy ? "保存中…" : "保存"}</button></footer>
    </form>
  </div>, document.body);
}

function Toggle(props: { checked: boolean; disabled?: boolean; label: string; onChange: (checked: boolean) => void }) {
  return <button aria-label={props.label} aria-pressed={props.checked} className="skill-toggle" disabled={props.disabled} onClick={(event) => { event.stopPropagation(); props.onChange(!props.checked); }} type="button"><span/></button>;
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / 1024).toFixed(bytes < 10_240 ? 1 : 0)} KB`;
}

function renderSkillManifest(skill: SkillDetail) {
  return `---\nname: ${JSON.stringify(skill.name)}\ndescription: ${JSON.stringify(skill.description)}\n---\n\n${skill.instructions.trim()}\n`;
}
