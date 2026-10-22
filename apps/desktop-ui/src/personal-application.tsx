import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "./host";
import { useModal } from "./use-modal";
import { personalPageAiBootstrap } from "./personal-page-ai-sdk";
import { PersonalPageAiConfirmation, PersonalPageAiStatus, usePersonalPageAi } from "./personal-page-ai";

type Call = { id: string; tool: string; arguments: Record<string, unknown>; fileGrantId?: string };
type Props = { projectId: string; packageDigest: string; html: string; title: string; active: boolean; preview?: boolean; onOpenModels?: () => void };

export function personalApplicationDocument(html: string, channel: string, pageAi = true) {
  const policy = "default-src 'none'; connect-src 'none'; img-src data: blob:; media-src data: blob:; font-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; object-src 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; base-uri 'none'; form-action 'none'";
  const bootstrap = `(() => {
    const channel=${JSON.stringify(channel)}, pending=new Map(); let next=0;
    window.addEventListener('message',event=>{const m=event.data;if(event.source!==parent||!m||m.channel!==channel||m.type!=='cilai-personal-result')return;const p=pending.get(m.id);if(!p)return;pending.delete(m.id);clearTimeout(p.timer);m.error?p.reject(new Error(m.error)):p.resolve(m.result);});
    const request=(method,params)=>new Promise((resolve,reject)=>{if(pending.size){reject(new Error('已有请求正在处理'));return;}const id=String(++next);const timer=setTimeout(()=>{pending.delete(id);reject(new Error('请求超时，结果未确认'));},60000);pending.set(id,{resolve,reject,timer});parent.postMessage({type:'cilai-personal-request',channel,id,method,params},'*');});
    Object.defineProperty(window,'cilaiPersonal',{value:Object.freeze({${pageAi ? "ai:window.__cilaiPersonalAi," : ""}listTools:()=>request('tools/list',{}),callTool:(tool,args,selectedFile)=>request('tools/call',{tool,arguments:args,fileGrantId:selectedFile?.grantId}),selectFile:()=>request('files/select',{}),writeFile:(selectedFile,content)=>request('files/write',{grantId:selectedFile?.grantId,content})}),writable:false,configurable:false});
  })();`;
  return `<!doctype html><meta http-equiv="Content-Security-Policy" content="${policy}">${pageAi ? `<script>${personalPageAiBootstrap(channel)}</script>` : ""}<script>${bootstrap}</script>${html}`;
}

export function PersonalApplication({ projectId, packageDigest, html, title, active, preview = false, onOpenModels }: Props) {
  const frame = useRef<HTMLIFrameElement>(null);
  const channel = useMemo(() => crypto.randomUUID(), [projectId, packageDigest, preview, html]);
  const source = useMemo(() => personalApplicationDocument(html, channel, !preview), [html, channel, preview]);
  const [pending, setPending] = useState<Call | null>(null);
  const [documentUrl, setDocumentUrl] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const pageAi = usePersonalPageAi({ frame, channel, url: documentUrl, active, enabled: !preview });
  const requestInFlight = useRef(false);
  const selectedFiles = useRef(new Map<string, {name:string;packageDigest:string}>());
  const toolsRequest = (tool?:string, arguments_:Record<string, unknown> = {}, fileGrantId?:string) => preview
    ? invoke("personal_capability_preview", {request:{creationProjectId:projectId,packageDigest,tool,arguments:arguments_}})
    : invoke(tool ? "personal_capability_call" : "personal_capability_tools", {creationProjectId:projectId,expectedDigest:packageDigest,tool,arguments:arguments_,fileGrantId});
  const alive = useRef(true);
  const respond = (id: string, result?: unknown, error?: string) => {
    if (alive.current) frame.current?.contentWindow?.postMessage({ type: "cilai-personal-result", channel, id, result, error }, "*");
  };
  useEffect(() => {
    let current = true;
    let url: string | undefined;
    alive.current = true;
    setDocumentUrl("");
    setError("");
    setPending(null);
    selectedFiles.current.clear();
    const release = (value: string) => {
      void invoke("personal_document_release", { url: value }).catch(error => console.error("应用页面释放失败", error));
    };
    void toolsRequest()
      .then(async () => {
        if (!current) return;
        const created = await invoke<string>("personal_document_create", { html: source, ...(!preview ? { pageBinding: { creationProjectId: projectId, expectedPackageDigest: packageDigest } } : {}) });
        if (!current) { release(created); return; }
        url = created;
        setDocumentUrl(created);
      })
      .catch((error) => { if (current) setError(String(error)); });
    return () => { current = false; alive.current = false; if (url) release(url); };
  }, [projectId, packageDigest, preview, source]);
  useEffect(() => {
    const receive = (event: MessageEvent) => {
      const message = event.data;
      if (event.source !== frame.current?.contentWindow || !message || message.channel !== channel || message.type !== "cilai-personal-request") return;
      if (typeof message.id !== "string" || !/^\d{1,12}$/.test(message.id)) return;
      if (!active || requestInFlight.current) { respond(message.id, undefined, "应用当前不能处理请求"); return; }
      if (message.method === "tools/list") {
        requestInFlight.current = true;
        void toolsRequest()
          .then((result) => respond(message.id, result))
          .catch((error) => respond(message.id, undefined, String(error)))
          .finally(() => { requestInFlight.current = false; });
      } else if (message.method === "tools/call") {
        const params = message.params;
        if (!params || typeof params.tool !== "string" || params.tool.length > 200 || !params.arguments || typeof params.arguments !== "object" || Array.isArray(params.arguments) || JSON.stringify(params.arguments).length > 256 * 1024) { respond(message.id, undefined, "工具参数无效"); return; }
        requestInFlight.current = true;
        if (params.fileGrantId !== undefined && (typeof params.fileGrantId !== "string" || !selectedFiles.current.has(params.fileGrantId))) { requestInFlight.current = false; respond(message.id, undefined, "请重新选择文件"); return; }
        setPending({ id: message.id, tool: params.tool, arguments: params.arguments, fileGrantId: params.fileGrantId });
      } else if ((message.method === "files/select" || message.method === "files/write") && !preview) {
        requestInFlight.current = true;
        const work = async () => {
          if (message.method === "files/select") {
            const selected = await invoke<{grantId:string;name:string;packageDigest:string}|null>("personal_capability_select_file", {creationProjectId:projectId});
            if (selected && selected.packageDigest !== packageDigest) throw new Error("版本已变化，请重新打开应用");
            if (selected) selectedFiles.current.set(selected.grantId, selected);
            return selected;
          }
          const params = message.params;
          if (!params || typeof params.grantId !== "string" || !selectedFiles.current.has(params.grantId) || typeof params.content !== "string") throw new Error("文件请求无效");
          const result = await invoke("personal_capability_confirm_file_write", {creationProjectId:projectId,packageDigest,grantId:params.grantId,content:params.content});
          if (result) selectedFiles.current.delete(params.grantId);
          return result;
        };
        void work().then(result=>respond(message.id,result)).catch(error=>respond(message.id,undefined,String(error))).finally(()=>{requestInFlight.current=false;});
      } else respond(message.id, undefined, preview ? "预览不支持外部文件，请安装后选择文件" : "不支持的请求");
    };
    window.addEventListener("message", receive);
    return () => window.removeEventListener("message", receive);
  }, [active, channel, projectId, packageDigest, preview]);
  const cancel = () => { if (pending) respond(pending.id, undefined, "用户已取消"); setPending(null); requestInFlight.current = false; };
  const execute = async () => {
    if (!pending || busy) return;
    setBusy(true);
    try {
      const result = await toolsRequest(pending.tool, pending.arguments, pending.fileGrantId);
      respond(pending.id, result);
    } catch (error) { respond(pending.id, undefined, String(error)); }
    finally { if (alive.current) { setBusy(false); setPending(null); } requestInFlight.current = false; }
  };
  useEffect(() => { if (!active && pending && !busy) cancel(); }, [active, pending, busy]);
  return <section className="personal-application-surface" data-personal-project={projectId} hidden={!active}>
    <PersonalPageAiStatus error={pageAi.error} needsModel={pageAi.needsModel} onOpenModels={onOpenModels}/>
    {error ? <p role="alert">{error}</p> : !documentUrl ? <p role="status">正在打开…</p> : <iframe ref={frame} className="personal-capability-preview" sandbox="allow-scripts" src={documentUrl} title={title} hidden={!active}/>}
    {pending ? <ToolConfirmation call={pending} busy={busy} active={active} onCancel={cancel} onExecute={() => void execute()}/> : null}
    {pageAi.pending ? <PersonalPageAiConfirmation key={pageAi.pending.writeRequest!.approval.confirmationId} pending={pageAi.pending} resolve={pageAi.resolve} active={active}/> : null}
  </section>;
}

function ToolConfirmation({ call, busy, active, onCancel, onExecute }: { call: Call; busy: boolean; active: boolean; onCancel: () => void; onExecute: () => void }) {
  const modal = useModal<HTMLDivElement>({ onClose: onCancel, busy, active });
  return <div className="agent-picker-backdrop" role="dialog" aria-modal="true" aria-label="运行个人工具" ref={modal}><section className="artifact-review"><header><h3>运行个人工具</h3><button disabled={busy} onClick={onCancel}>取消</button></header><p>{call.tool}{call.fileGrantId ? " · 使用选定文件" : ""}</p><pre className="personal-capability-result">{JSON.stringify(call.arguments, null, 2)}</pre><button className="primary" disabled={busy} onClick={onExecute}>{busy ? "正在运行…" : "运行"}</button></section></div>;
}
