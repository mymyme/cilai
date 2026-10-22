import type { PersonalPageAiManifest, PersonalPageTool } from "./generated/protocol";
import "./personal-page-ai-declaration.css";

export const personalPermissions = [
  { value: "personal_file.read", label: "读取选定文件", page: false },
  { value: "personal_file.write_confirmed", label: "确认后写入文件", page: false },
  { value: "workbench_artifact.create", label: "创建成果", page: false },
  { value: "page_ai.context", label: "本页上下文", page: true },
  { value: "page_ai.ui_actions_confirmed", label: "确认后操作页面", page: true },
  { value: "page_ai.model", label: "页面模型", page: true },
  { value: "page_ai.conversation_handoff", label: "主动转交对话", page: true },
  { value: "page_ai.tools.read", label: "页面读取工具", page: true },
  { value: "page_ai.tools.write_confirmed", label: "确认后使用页面写入工具", page: true },
];

export function permissionsForTarget(target: string) {
  return personalPermissions.filter(item => target === "web_mcp_application"
    || target === "local_mcp" && !item.page
    || target === "web_application" && item.page && !item.value.startsWith("page_ai.tools."));
}
export function personalPermissionLabel(value: string) {
  const permission = personalPermissions.find(item => item.value === value);
  if (!permission) throw new Error("personal_page_permission_mismatch");
  return permission.label;
}
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === "string";
const texts = (value: unknown): value is string[] => Array.isArray(value) && value.every(text);
const keys = (value: Record<string, unknown>, expected: string[]) => Object.keys(value).length === expected.length && expected.every(key => Object.hasOwn(value, key));
const schema = (value: unknown) => object(value) && value.type === "object";
const tools = (value: unknown): value is PersonalPageTool[] => Array.isArray(value) && value.every(item => object(item)
  && keys(item, ["name", "title", "description", "inputSchema"]) && text(item.name) && text(item.title) && text(item.description) && schema(item.inputSchema));

export function isPersonalPageAiManifest(value: unknown): value is PersonalPageAiManifest {
  return object(value) && keys(value, ["schemaVersion", "contextTypes", "uiActions", "model", "conversationHandoff", "readTools", "writeTools"])
    && value.schemaVersion === 1 && typeof value.model === "boolean" && typeof value.conversationHandoff === "boolean"
    && Array.isArray(value.contextTypes) && value.contextTypes.every(item => object(item)
      && keys(item, ["contextType", "schemaVersion", "dataSchema"]) && text(item.contextType) && text(item.schemaVersion) && schema(item.dataSchema))
    && Array.isArray(value.uiActions) && value.uiActions.every(item => object(item)
      && keys(item, ["actionId", "title", "description", "inputSchema", "riskLevel", "preconditions", "expectedEffects", "confirmationPolicy"])
      && text(item.actionId) && text(item.title) && text(item.description) && schema(item.inputSchema)
      && ["low", "medium", "high"].includes(String(item.riskLevel)) && texts(item.preconditions) && texts(item.expectedEffects) && item.confirmationPolicy === "always")
    && tools(value.readTools) && tools(value.writeTools);
}

export function pageAiPermissionsMatch(target: string, permissions: string[], manifest: unknown): boolean {
  if (!["web_application", "local_mcp", "web_mcp_application"].includes(target)) return false;
  const allowed = permissionsForTarget(target).map(item => item.value);
  if (permissions.some(item => !allowed.includes(item)) || new Set(permissions).size !== permissions.length) return false;
  if (manifest === undefined || manifest === null) return !permissions.some(item => item.startsWith("page_ai."));
  if (!isPersonalPageAiManifest(manifest) || target === "local_mcp") return false;
  const required = [
    ["page_ai.context", manifest.contextTypes.length > 0], ["page_ai.ui_actions_confirmed", manifest.uiActions.length > 0],
    ["page_ai.model", manifest.model], ["page_ai.conversation_handoff", manifest.conversationHandoff],
    ["page_ai.tools.read", manifest.readTools.length > 0], ["page_ai.tools.write_confirmed", manifest.writeTools.length > 0],
  ] as const;
  return required.some(([, needed]) => needed) && required.every(([permission, needed]) => permissions.includes(permission) === needed);
}

function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (object(value)) return `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`;
  return JSON.stringify(value);
}
export function pageAiChanged(before?: PersonalPageAiManifest | null, after?: PersonalPageAiManifest | null) {
  return canonical(before ?? null) !== canonical(after ?? null);
}

function Schema({ value }: { value: unknown }) {
  return <details className="personal-page-ai-schema"><summary>参数约束</summary><pre>{JSON.stringify(value, null, 2)}</pre></details>;
}
function ToolList({ title, tools: items, write }: { title: string; tools: PersonalPageTool[]; write?: boolean }) {
  return <section><h6>{title}</h6>{items.length ? items.map(tool => <article key={tool.name}>
    <strong>{tool.title}</strong><code>{tool.name}</code><p>{tool.description}</p>{write ? <span>每次确认</span> : null}<Schema value={tool.inputSchema}/>
  </article>) : <p className="personal-page-ai-none">无</p>}</section>;
}
function Declaration({ manifest, label }: { manifest?: PersonalPageAiManifest | null; label: string }) {
  return <section className="personal-page-ai-version" aria-label={label}><h5>{label}</h5>{manifest ? <>
    <dl className="personal-page-ai-flags"><dt>声明版本</dt><dd>{manifest.schemaVersion}</dd><dt>页面模型</dt><dd>{manifest.model ? "允许" : "未授权"}</dd>
      <dt>主动转交对话</dt><dd>{manifest.conversationHandoff ? "允许" : "未授权"}</dd></dl>
    <section><h6>本页上下文</h6>{manifest.contextTypes.length ? manifest.contextTypes.map(context => <article key={context.contextType}>
      <strong>{context.contextType}</strong><span>格式版本 {context.schemaVersion}</span><Schema value={context.dataSchema}/>
    </article>) : <p className="personal-page-ai-none">无</p>}</section>
    <section><h6>页面操作</h6>{manifest.uiActions.length ? manifest.uiActions.map(action => <article key={action.actionId}>
      <strong>{action.title}</strong><code>{action.actionId}</code><p>{action.description}</p>
      <dl><dt>风险</dt><dd>{({ low: "低", medium: "中", high: "高" })[action.riskLevel]}</dd><dt>确认</dt><dd>每次确认</dd>
        <dt>前提</dt><dd>{action.preconditions.length ? <ul>{action.preconditions.map((item, i) => <li key={i}>{item}</li>)}</ul> : "无"}</dd>
        <dt>预期效果</dt><dd>{action.expectedEffects.length ? <ul>{action.expectedEffects.map((item, i) => <li key={i}>{item}</li>)}</ul> : "无"}</dd></dl><Schema value={action.inputSchema}/>
    </article>) : <p className="personal-page-ai-none">无</p>}</section>
    <ToolList title="读取工具" tools={manifest.readTools}/><ToolList title="写入工具" tools={manifest.writeTools} write/>
  </> : <p className="personal-page-ai-none">未授权页面 AI</p>}</section>;
}

export function PersonalPageAiDeclaration({ before, after }: { before?: PersonalPageAiManifest | null; after?: PersonalPageAiManifest | null }) {
  if (!before && !after) return null;
  const changed = pageAiChanged(before, after);
  return <section className="personal-page-ai-declaration" aria-label="页面 AI 声明核对">
    <header><h4><svg aria-hidden="true" viewBox="0 0 20 20"><path d="M10 2 17 5v5c0 4-7 8-7 8s-7-4-7-8V5z"/><path d="m6.5 9.5 2.3 2.3 4.7-4.6"/></svg>页面 AI</h4><span>{changed ? "声明有变化" : "声明未变化"}</span></header>
    <div className="personal-page-ai-comparison"><Declaration manifest={before} label="已安装"/><Declaration manifest={after} label="本次版本"/></div>
  </section>;
}
