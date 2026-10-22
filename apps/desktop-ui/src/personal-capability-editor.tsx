import { personalWebTemplate } from "./personal-template";
import { useEffect, useState } from "react";
import type { CapabilityCreationProject, PersonalPageAiManifest } from "./generated/protocol";
import { invoke } from "./host";
import { useModal } from "./use-modal";
import { Select } from "./select";
import { PersonalApplication } from "./personal-application";
import { isPersonalPageAiManifest, pageAiChanged, pageAiPermissionsMatch, PersonalPageAiDeclaration, personalPermissionLabel, permissionsForTarget } from "./personal-page-ai-declaration";
import "./personal-capability-editor.css";

type SourceFile = { path: string; content: string };
type Editor = { project: CapabilityCreationProject; sourceFiles: SourceFile[]; title: string; businessGoal: string;
  requestedPermissions: string[]; addedPermissions: string[]; installedDigest: string | null;
  candidateDigest: string | null; candidateVersion: string | null; versions: { version: string; packageDigest: string; createdAt: number }[];
  pageAi?: PersonalPageAiManifest | null; installedPageAi?: PersonalPageAiManifest | null };

export function personalTemplate(target: string): SourceFile[] {
  const files: SourceFile[] = [{ path: "README.md", content: "# 个人能力\n\n仅在工作台受管环境中运行。\n" }];
  if (target !== "local_mcp") files.push({ path: "web/index.html", content: personalWebTemplate(target !== "web_application") });
  if (target !== "web_application") files.push({ path: "mcp/server.mjs", content: `import readline from 'node:readline';
const tool = { name: 'inspect_text', description: '检查文本长度与行数', inputSchema: { type: 'object', properties: { text: { type: 'string', description: '待检查文本' } }, required: ['text'], additionalProperties: false } };
const fileTool = {name:'inspect_file',description:'检查选定文本文件',inputSchema:{type:'object',properties:{selectedFile:{type:'object',description:'选择本次检查的文件'}},required:['selectedFile'],additionalProperties:false}};
const cleanTool = {name:'trim_file',description:'清除选定文件的行末空白，确认后写入',inputSchema:fileTool.inputSchema};
for await (const line of readline.createInterface({ input: process.stdin })) {
  const message = JSON.parse(line);
  if (message.id === undefined) continue;
  let result;
  if (message.method === 'initialize') result = { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'personal-inspector', version: '1.0.0' } };
  else if (message.method === 'tools/list') result = { tools: [tool, fileTool, cleanTool] };
  else if (message.method === 'tools/call' && message.params.name === tool.name) {
    const text = message.params.arguments.text;
    result = { content: [{ type: 'text', text: JSON.stringify({ characters: [...text].length, lines: text.split('\\n').length }) }] };
  } else if (message.method === 'tools/call' && [fileTool.name,cleanTool.name].includes(message.params.name)) {
    const selected = message.params.arguments.selectedFile;
    const text = new TextDecoder('utf-8',{fatal:true}).decode(Buffer.from(selected.content,'base64'));
    const content = text.split('\\n').map(line=>line.trimEnd()).join('\\n');
    result = {content:[{type:'text',text:JSON.stringify({name:selected.name,characters:[...text].length,lines:text.split('\\n').length})}],...(message.params.name===cleanTool.name?{structuredContent:{fileWrite:{content}}}:{})};
  } else { console.log(JSON.stringify({ jsonrpc: '2.0', id: message.id, error: { code: -32601, message: 'Unknown method' } })); continue; }
  console.log(JSON.stringify({ jsonrpc: '2.0', id: message.id, result }));
}
` });
  return files;
}

export function PersonalCapabilityEditor({ projectId, onClose, onChanged }: {
  projectId: string; onClose: () => void; onChanged: (project: CapabilityCreationProject, upgraded: boolean) => void;
}) {
  const [state, setState] = useState<Editor | null>(null);
  const [files, setFiles] = useState<SourceFile[]>([]);
  const [selected, setSelected] = useState("");
  const [newPath, setNewPath] = useState("");
  const [chosenPermissions, setChosenPermissions] = useState<string[]>([]);
  const [busy, setBusy] = useState(true);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState("");
  const [confirm, setConfirm] = useState(false);
  const [pageConfirmed, setPageConfirmed] = useState(false);
  const [preview, setPreview] = useState<{ html?: string; tools?: { name: string; inputSchema: unknown }[] } | null>(null);
  const [tool, setTool] = useState("");
  const [argumentsText, setArgumentsText] = useState('{"text":"待检查文本"}');
  const [result, setResult] = useState("");
  const [discard, setDiscard] = useState(false);
  const modal = useModal<HTMLDivElement>({ onClose: () => dirty ? setDiscard(true) : onClose(), busy });
  const accept = (value: Editor) => {
    if (!pageAiPermissionsMatch(value.project.targetType, value.requestedPermissions, value.pageAi)
      || value.installedPageAi != null && !isPersonalPageAiManifest(value.installedPageAi)) throw new Error("personal_page_manifest_invalid");
    setState(value); setFiles(value.sourceFiles); setChosenPermissions(value.requestedPermissions); setSelected(value.sourceFiles[0]?.path ?? ""); setDirty(false); setConfirm(false); setPageConfirmed(false); setPreview(null); setResult("");
  };
  useEffect(() => { let active = true; void invoke<Editor>("personal_capability_editor", { request: { creationProjectId: projectId } }).then((value) => { if (active) accept(value); }).catch((error) => { if (active) setError(String(error)); }).finally(() => { if (active) setBusy(false); }); return () => { active = false; }; }, [projectId]);
  const run = async (action: "build" | "upgrade" | "preview", toolName?: string) => {
    if (!state || busy) return;
    if (action === "upgrade" && pageAiChanged(state.installedPageAi, state.pageAi) && !pageConfirmed) return;
    setBusy(true); setError("");
    try {
      const common = { creationProjectId: projectId, expectedRevision: state.project.stateRevision, clientOperationId: crypto.randomUUID() };
      if (action === "preview") {
        const value = await invoke<{ html?: string; tools?: { name: string; inputSchema: unknown }[]; result?: unknown }>("personal_capability_preview", { request: { creationProjectId: projectId, packageDigest: state.candidateDigest, ...(toolName ? { tool: toolName, arguments: JSON.parse(argumentsText) } : {}) } });
        if (toolName) setResult(JSON.stringify(value.result, null, 2)); else { setPreview(value); setTool(value.tools?.[0]?.name ?? ""); }
      } else {
        const value = await invoke<Editor>(`personal_capability_${action}`, { request: action === "build"
          ? { ...common, title: state.title, businessGoal: state.businessGoal, requestedPermissions: chosenPermissions, sourceFiles: files }
          : { ...common, packageDigest: state.candidateDigest, confirmedPermissions: state.requestedPermissions } });
        accept(value); onChanged(value.project, action === "upgrade");
      }
    } catch (error) { setError(String(error)); } finally { setBusy(false); }
  };
  const modify = (next: SourceFile[]) => { setFiles(next); setDirty(true); setConfirm(false); setPageConfirmed(false); setPreview(null); };
  return <div className="agent-picker-backdrop" role="dialog" aria-modal="true" aria-label="编辑个人能力" ref={modal}><section className="personal-editor">
    <header><div><span className="personal-editor-eyebrow">我的创建</span><h3>{state?.title ?? "个人能力"}</h3><small><span className="personal-editor-version">{state?.candidateVersion ?? "初始版本"}</span>{dirty ? <span className="personal-editor-dirty">未保存 · 未构建</span> : <span>已保存</span>}<span>已安装：{state?.installedDigest ? state.versions.find((version) => version.packageDigest === state.installedDigest)?.version ?? "版本未知" : "未安装"}</span></small></div><button disabled={busy} onClick={() => dirty ? setDiscard(true) : onClose()}>关闭</button></header>
    {error ? <p className="personal-editor-notice" data-tone="danger" role="alert">{error}</p> : null}{busy ? <p className="personal-editor-notice" role="status">正在处理…</p> : null}
    {state ? <>
      <div className="personal-editor-toolbar"><button disabled={busy || files.length > 0} onClick={() => { const next = personalTemplate(state.project.targetType); modify(next); setSelected(next[0].path); setChosenPermissions(state.project.targetType!=="web_application"?["personal_file.read","personal_file.write_confirmed"]:[]); }}>使用模板</button><span className="personal-editor-toolbar-spacer"/><button disabled={busy || !files.length} onClick={() => void run("build")}>保存并构建</button><button title={dirty ? "先保存并构建修改" : !state.candidateVersion ? "先构建候选版本" : `预览 ${state.candidateVersion}`} disabled={busy || dirty || !state.candidateVersion} onClick={() => void run("preview")}>预览</button><button className="primary" title={dirty ? "先保存并构建修改" : state.candidateDigest === state.installedDigest ? "当前候选版本已安装" : "确认版本和权限后安装"} disabled={busy || dirty || !state.candidateVersion || state.candidateDigest === state.installedDigest} onClick={() => { setPageConfirmed(false); setConfirm(true); }}>{state.installedDigest ? "升级" : "安装"}</button></div>
      <div className="personal-editor-source"><nav aria-label="源码文件"><span className="personal-editor-label">文件</span>{files.map((file) => <button className="personal-editor-file" key={file.path} aria-pressed={selected === file.path} onClick={() => setSelected(file.path)}><FileGlyph/><span>{file.path}</span></button>)}<div className="personal-editor-add"><input aria-label="新增文件路径" placeholder="web/style.css" value={newPath} disabled={busy} onChange={(event) => setNewPath(event.target.value)}/><button disabled={busy || !newPath.trim()} onClick={() => { if (files.some((file) => file.path === newPath.trim())) { setError("文件已存在。"); return; } modify([...files, { path: newPath.trim(), content: "" }]); setSelected(newPath.trim()); setNewPath(""); }}>添加文件</button></div></nav><div className="personal-editor-code">{selected ? <><div className="personal-editor-code-head"><span>{selected}</span><button className="personal-editor-delete" disabled={busy} onClick={() => { const next = files.filter((file) => file.path !== selected); modify(next); setSelected(next[0]?.path ?? ""); }}>删除文件</button></div><textarea spellCheck={false} aria-label={`源码 ${selected}`} value={files.find((file) => file.path === selected)?.content ?? ""} disabled={busy} onChange={(event) => modify(files.map((file) => file.path === selected ? { ...file, content: event.target.value } : file))}/></> : <p className="personal-editor-empty">添加源码或使用模板。</p>}</div></div>
      <fieldset className="personal-editor-permissions" disabled={busy}><legend>权限</legend>{permissionsForTarget(state.project.targetType).map((permission) => <label key={permission.value}><input type="checkbox" checked={chosenPermissions.includes(permission.value)} onChange={(event) => { setChosenPermissions(event.target.checked ? [...chosenPermissions, permission.value] : chosenPermissions.filter((item) => item !== permission.value)); setDirty(true); setConfirm(false); setPageConfirmed(false); }}/>{permission.label}</label>)}</fieldset>
      {state.project.targetType !== "local_mcp" ? <PersonalPageAiDeclaration before={state.installedPageAi} after={state.pageAi}/> : null}
      {confirm ? <div className="personal-editor-confirm"><strong>确认{state.installedDigest ? "升级" : "安装"}至 {state.candidateVersion}</strong><p>将停止受管任务，以独立数据副本安装。原版本与数据保留。{state.addedPermissions.length ? `新增权限：${state.addedPermissions.map(personalPermissionLabel).join("、")}` : "权限名称未增加。"}</p>
        {pageAiChanged(state.installedPageAi, state.pageAi) ? <label className="personal-page-ai-confirm"><input type="checkbox" disabled={busy} checked={pageConfirmed} onChange={event => setPageConfirmed(event.target.checked)}/>我已核对页面 AI 声明变更</label> : null}
        <button disabled={busy} onClick={() => setConfirm(false)}>取消</button><button className="primary" disabled={busy || pageAiChanged(state.installedPageAi, state.pageAi) && !pageConfirmed} onClick={() => void run("upgrade")}>确认</button></div> : null}
      {preview?.html && state.project.targetType === "web_mcp_application" ? <PersonalApplication projectId={projectId} packageDigest={state.candidateDigest!} html={preview.html} title="候选版本预览" active preview/> : preview?.html ? <iframe title="候选版本预览" sandbox="" srcDoc={`<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; form-action 'none'; base-uri 'none'">${preview.html}`}/> : null}
      {preview?.tools ? <div className="personal-editor-preview"><Select ariaLabel="预览工具" value={tool} options={preview.tools.map((item) => ({ value: item.name, label: item.name }))} onChange={setTool}/><details><summary>参数格式</summary><pre>{JSON.stringify(preview.tools.find((item) => item.name === tool)?.inputSchema, null, 2)}</pre></details><textarea aria-label="预览参数" value={argumentsText} onChange={(event) => setArgumentsText(event.target.value)}/><button disabled={busy || !tool} onClick={() => void run("preview", tool)}>运行预览</button>{result ? <pre>{result}</pre> : null}</div> : null}
      {discard ? <div className="personal-editor-confirm"><p>尚有未保存的修改。</p><button onClick={() => setDiscard(false)}>返回编辑</button><button onClick={onClose}>放弃修改</button></div> : null}
    </> : null}
  </section></div>;
}

function FileGlyph() {
  return <svg aria-hidden="true" viewBox="0 0 16 16"><path d="M4 1.75h5l3 3v9.5H4z"/><path d="M9 1.75v3h3"/></svg>;
}
