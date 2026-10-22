import { Select } from "./select";
import { useEffect, useState } from "react";
import { invoke } from "./host";
import { useModal } from "./use-modal";
type Tool = { name: string; description?: string; inputSchema: { properties?: Record<string, { type?: string; description?: string }>; required?: string[] } };
export function PersonalCapabilityRunner({ projectId, title, onClose }: { projectId: string; title: string; onClose: () => void }) {
  const [tools, setTools] = useState<Tool[]>([]);
  const [selected, setSelected] = useState("");
  const [values, setValues] = useState<Record<string, string>>({});
  const [error, setError] = useState("");
  const [result, setResult] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);
  const [selectedFile, setSelectedFile] = useState<{grantId:string;name:string;packageDigest:string} | null>(null);
  const [fileWrite, setFileWrite] = useState<{content:string;grantId:string;packageDigest:string} | null>(null);
  const modal = useModal<HTMLDivElement>({ onClose, busy });
  useEffect(() => { let active=true; void invoke<{ tools: Tool[] }>("personal_capability_tools", { creationProjectId: projectId }).then(({ tools }) => { if (active) setTools(tools); }).catch((error) => { if (active) setError(String(error)); }).finally(() => { if (active) setBusy(false); }); return () => { active=false; }; }, [projectId]);
  const tool = tools.find((tool) => tool.name === selected);
  const execute = async () => {
    if (!tool || busy) return;
    setBusy(true); setError(""); setResult(null); setFileWrite(null);
    try {
      const arguments_: Record<string, unknown> = {};
      for (const [name, schema] of Object.entries(tool.inputSchema.properties ?? {})) {
        if (name === "selectedFile" && selectedFile) continue;
        if (!values[name]?.trim() && !tool.inputSchema.required?.includes(name)) continue;
        arguments_[name] = schema.type === "string" ? values[name] ?? "" : JSON.parse(values[name] ?? "null");
      }
      const response = await invoke<{ result: { content?: Array<{ type: string; text?: string }>; isError?: boolean; structuredContent?: {fileWrite?: {content?: string}} } }>("personal_capability_call", { creationProjectId: projectId, tool: tool.name, arguments: arguments_, ...(selectedFile ? {fileGrantId:selectedFile.grantId,expectedDigest:selectedFile.packageDigest} : {}) });
      if (selectedFile && typeof response.result.structuredContent?.fileWrite?.content === "string") setFileWrite({content:response.result.structuredContent.fileWrite.content,grantId:selectedFile.grantId,packageDigest:selectedFile.packageDigest});
      const text = response.result.content?.filter((item) => item.type === "text").map((item) => item.text).join("\n") || JSON.stringify(response.result, null, 2);
      if (response.result.isError) setError(text); else setResult(text);
    } catch (error) { setError(String(error)); } finally { setSelectedFile(null); setBusy(false); }
  };
  return <div className="agent-picker-backdrop" role="dialog" aria-modal="true" aria-label="运行个人能力" ref={modal}><section className="artifact-review"><header><h3>{title}</h3><button type="button" disabled={busy} onClick={onClose}>关闭</button></header><p>本机运行</p>
    {busy ? <p role="status">正在处理…</p> : null}
    <Select ariaLabel="选择工具" value={selected} disabled={busy} options={[{value:"",label:"选择工具"}, ...tools.map((tool) => ({value:tool.name,label:tool.name}))]} onChange={(value) => { setSelected(value); setValues({}); setResult(null); setSelectedFile(null); setFileWrite(null); setError(""); }}/>
    {tool ? <form onSubmit={(event) => { event.preventDefault(); void execute(); }}>{Object.entries(tool.inputSchema.properties ?? {}).map(([name, schema]) => name === "selectedFile" ? <div key={name}><button type="button" disabled={busy} onClick={() => { setBusy(true); void invoke<typeof selectedFile>("personal_capability_select_file",{creationProjectId:projectId}).then(setSelectedFile).catch((error) => setError(String(error))).finally(() => setBusy(false)); }}>选择文件</button><span>{selectedFile?.name ?? "未选择"}</span></div> : <label key={name}>{schema.description ?? name}<textarea aria-label={name} required={tool.inputSchema.required?.includes(name)} placeholder={schema.type === "string" ? "" : "填写 JSON 值"} value={values[name] ?? ""} onChange={(event) => setValues((current) => ({ ...current, [name]: event.target.value }))}/></label>)}<button disabled={busy || !!tool.inputSchema.required?.includes("selectedFile") && !selectedFile} type="submit">运行</button></form> : null}
    {fileWrite ? <button disabled={busy} onClick={() => {setBusy(true); void invoke("personal_capability_confirm_file_write",{creationProjectId:projectId,...fileWrite}).then((result) => {if(result) {setResult("文件已写入。");setFileWrite(null);}}).catch((error) => setError(String(error))).finally(() => setBusy(false));}}>写入选定文件</button> : null}
    {error ? <p role="alert">{error}</p> : null}{result !== null ? <pre className="personal-capability-result">{result}</pre> : null}
  </section></div>;
}
