import { delegateManagedScript } from './managed-verification.mjs';
import { createVerificationFixture } from './verification-fixture.mjs';
import { verificationEnvironment } from "./verification-environment.mjs";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { createInterface } from "node:readline";
import { once } from "node:events";

const delegatedStatus = await delegateManagedScript(import.meta.url, { pathArguments: [0] });
if (delegatedStatus !== null) process.exit(delegatedStatus);

const executable = process.argv[2];
const source = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const release = JSON.parse(await readFile(join(source, "third_party/openai-codex.release.json"), "utf8"));
const binaryHash = createHash("sha256").update(await readFile(executable)).digest("hex");
assert.ok(Object.values(release.packages).some((entry) => entry.executableSha256 === binaryHash), "探针必须使用清单固定的官方运行时");
const policySource = await readFile(join(source, "crates/workbench-runtime/src/report_workspace.rs"), "utf8");
const policyFlags = [...policySource.matchAll(/"((?:features\.[a-z_]+|skills\.bundled\.enabled)=(?:true|false))"/g)].map((match) => match[1]);
assert.ok(policyFlags.includes("features.shell_tool=false"));
assert.ok(policyFlags.includes("features.view_image=false"));
if (!executable) throw new Error("请传入随包 Codex App Server 路径；本探针只使用本机模拟模型");
const fixture = await createVerificationFixture("cilai-report-materials-probe-");
const root = fixture.root;
let fixtureFailure;
try {
const home = join(root, "harness");
const workspace = join(root, "workspace");
const outside = join(root,"outside-sensitive-material.txt");
await writeFile(outside,"OUTSIDE_SECRET_BODY");
await Promise.all([mkdir(home), mkdir(workspace)]);
const requests = [];
const forbiddenCalls = [];
const resourceRequests = [];
let resourceProbe = true;
let toolRevision = 1;
const backend = createServer(async (req, res) => {
  try {
    if (req.method !== "POST") { res.writeHead(405).end(); return; }
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks));
    if (req.url === "/leak") { forbiddenCalls.push(body); res.writeHead(403).end(); return; }
    if (req.url === "/mcp") {
      if (body.id === undefined) { res.writeHead(202).end(); return; }
      if (body.method.startsWith("resources/")) { resourceRequests.push(body); res.writeHead(200,{"content-type":"application/json"}).end(JSON.stringify({jsonrpc:"2.0",id:body.id,error:{code:-32601,message:"application_mcp_method_not_found"}})); return; }
      const result = body.method === "initialize"
        ? { protocolVersion: "2024-11-05", capabilities: { tools: {} }, serverInfo: { name: "probe", version: "1" } }
        : body.method === "tools/list"
          ? { tools: [{ name: `query_v${toolRevision}`, description: "Local verification query", inputSchema: { type: "object", properties: {} } }] }
          : { content: [{ type: "text", text: "local-result" }] };
      res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ jsonrpc: "2.0", id: body.id, result }));
      return;
    }
    assert.equal(req.url, "/v1/responses", "模型请求只能到本机 Responses 模拟端点");
    requests.push(body);
    const id = `response-${requests.length}`;
    const item = resourceProbe
      ? { id:"resource-attack", type:"function_call", call_id:"resource-call", name:"read_mcp_resource", arguments:JSON.stringify({server:"cilai_application",uri:pathToFileURL(outside).href}) }
      : { id: `msg-${requests.length}`, type: "message", role: "assistant", status: "completed", content: [{ type: "output_text", text: `local-reply-${requests.length}`, annotations: [] }] };
    resourceProbe = false;
    res.writeHead(200, { "content-type": "text/event-stream" });
    for (const event of [
      { type: "response.created", response: { id, status: "in_progress" } },
      { type: "response.output_item.added", output_index: 0, item: { ...item, status: "in_progress", content: [] } },
      ...(item.type === "message" ? [{ type: "response.output_text.delta", item_id: item.id, output_index: 0, content_index: 0, delta: item.content[0].text }] : []),
      { type: "response.output_item.done", output_index: 0, item },
      { type: "response.completed", response: { id, status: "completed", output: [item], usage: { input_tokens: 100, input_tokens_details: { cached_tokens: 0 }, output_tokens: 10, total_tokens: 110 } } },
    ]) res.write(`data: ${JSON.stringify(event)}\n\n`);
    res.end();
  } catch (error) { res.writeHead(500).end(String(error)); }
});
fixture.trackServer(backend);
backend.listen(0, "127.0.0.1");
await once(backend, "listening");
const origin = `http://127.0.0.1:${backend.address().port}`;
const baseConfig = `
model = "gpt-5.6-sol"
model_provider = "probe"
check_for_update_on_startup = false
cli_auth_credentials_store = "file"
project_doc_max_bytes = 0
web_search = "disabled"
[features]
plugins = false
memories = false
[model_providers.probe]
name = "Local protocol probe"
base_url = "${origin}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
`;
const catalog = JSON.parse(await readFile(join(source,"third_party/openai-codex/codex-rs/models-manager/models.json"),"utf8"));
for (const model of catalog.models) { model.use_responses_lite = false; model.shell_type = "disabled"; model.apply_patch_tool_type = null; model.supports_search_tool = true; model.tool_mode = null; model.multi_agent_version = null; }
const catalogPath = join(root,"models.json");
await writeFile(catalogPath,JSON.stringify(catalog));
await writeFile(join(home,"config.toml"), baseConfig + `\n[mcp_servers.inherited_leak]\nurl = "${origin}/leak"\n`);

const localOverrides = [
  'model="gpt-5.6-sol"', 'model_provider="probe"', 'check_for_update_on_startup=false',
  'project_doc_max_bytes=0', 'web_search="disabled"', 'features.memories=false',
  'model_providers.probe.name="Local protocol probe"',
  `model_providers.probe.base_url=${JSON.stringify(`${origin}/v1`)}`,
  'model_providers.probe.wire_api="responses"', 'model_providers.probe.request_max_retries=0',
  'model_providers.probe.stream_max_retries=0',
];
function start() {
  const child = fixture.track(spawn(resolve(executable), ["--listen", "stdio://", "--session-source", "app-server", "-c", `model_catalog_json=${JSON.stringify(catalogPath)}`, ...[...localOverrides,...policyFlags].flatMap((value) => ["-c", value]), "-c", `mcp_servers.cilai_application.url="${origin}/mcp"`], {
    cwd: workspace,
    env: verificationEnvironment(root, { CODEX_HOME: home, CODEX_SQLITE_HOME: home, OPENAI_API_KEY: "local-probe-only" }),
    stdio: ["pipe", "pipe", "pipe"],
  }));
  const pending = new Map();
  let fatal;
  const events = [];
  let id = 0;
  let stderr = "";
  child.stderr.on("data", (data) => { stderr = (stderr + data).slice(-6000); });
  const lines = createInterface({ input: child.stdout });
  lines.on("line", (line) => {
    let message;
    try { message = JSON.parse(line); }
    catch (error) { fixture.markFailure(error); fail(error); return; }
    if (!message || typeof message !== "object") { const error = new Error("服务输出不是有效消息"); fixture.markFailure(error); fail(error); return; }
    const request = pending.get(message.id);
    if (!request) { events.push(message); return; }
    pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  });
  const fail = (error) => { fatal ??= error; for (const request of pending.values()) { clearTimeout(request.timer); request.reject(error); } pending.clear(); };
  child.on("error", fail);
  child.stdin.on("error", fail);
  child.on("exit", (code) => fail(new Error(`Harness退出 ${code}: ${stderr}`)));
  const rpc = (method, params) => new Promise((resolveRequest, reject) => {
    if (fatal) { reject(fatal); return; }
    const requestId = ++id;
    const timer = setTimeout(() => { pending.delete(requestId); reject(new Error(`RPC超时 ${method}: ${stderr}`)); }, 20000);
    pending.set(requestId, { resolve: resolveRequest, reject, timer });
    child.stdin.write(JSON.stringify({ id: requestId, method, params }) + "\n");
  });
  return {
    rpc, events, pid:child.pid,
    async initialize() {
      await rpc("initialize", { clientInfo: { name: "cilai-thread-probe", version: "1" }, capabilities: { experimentalApi: true } });
      child.stdin.write(JSON.stringify({ method: "initialized", params: {} }) + "\n");
    },
    async waitTurn(turnId) {
      const deadline = Date.now() + 20000;
      while (Date.now() < deadline) {
        const event = events.find((item) => item.method === "turn/completed" && item.params.turn.id === turnId);
        if (event) { assert.equal(event.params.turn.status, "completed", JSON.stringify(event)); return; }
        await new Promise((done) => setTimeout(done, 20));
      }
      throw new Error(`等待回合超时: ${stderr}`);
    },
    async stop() {
      await fixture.stop(child);
      lines.close();
    },
  };
}

let server;
try {
  server = start();
  await server.initialize();
  const options = { cwd: workspace, model: "gpt-5.6-sol", modelProvider: "probe", approvalPolicy: "never", sandbox: "read-only" };
  const inherited = await server.rpc("config/read", {includeLayers:false});
  assert.ok(inherited.config.mcp_servers.inherited_leak, "真实API必须暴露未授权继承配置，以便启动前拒绝");
  assert.equal(forbiddenCalls.length,0,"配置预检不得先启动未授权MCP");
  assert.equal(requests.length,0,"配置拒绝必须发生在模型请求之前");
  await server.stop();
  await rm(join(home,"config.toml"));
  server = start();
  await server.initialize();
  const firstReportPid = server.pid;
  const effective = await server.rpc("config/read",{includeLayers:false});
  assert.deepEqual(Object.keys(effective.config.mcp_servers),["cilai_application"]);
  const created = await server.rpc("thread/start", options);
  let threadId = created.thread.id;
  const catalogStatus = await server.rpc("mcpServerStatus/list",{threadId,detail:"toolsAndAuthOnly",limit:100});
  assert.ok(catalogStatus.data.some((entry)=>entry.name==="cilai_application" && entry.runtimeStatus==="connected"));
  for (const phase of ["start", "resume", "fork"]) {
    if (phase === "resume") await server.rpc("thread/resume", { ...options, threadId });
    if (phase === "fork") threadId = (await server.rpc("thread/fork", { ...options, threadId })).thread.id;
    const turn = await server.rpc("turn/start", { threadId, input: [{ type: "text", text: "Use only authorized report material tools and return done." }] });
    await server.waitTurn(turn.turn.id);
    const request = requests.at(-1);
    assert.ok(request, "必须捕获真实运行时发出的模型请求");
    const names = [];
    const inspect = (tool) => {
      if (tool.name || tool.type === "tool_search") names.push(tool.name ?? tool.type);
      for (const child of tool.tools ?? []) inspect(child);
    };
    for (const tool of request.tools ?? []) inspect(tool);
    const banned = /(?:^exec$|^wait$|exec_command|write_stdin|shell|read_file|list_dir|grep_files|view_image|apply_patch|js_repl|code_mode|spawn_agent|delegate)/i;
    assert.deepEqual(names.filter((name) => banned.test(name)), [], `${phase}:模型不应取得通用文件或执行工具: ${names}`);
    assert.ok(names.includes("tool_search"), `${phase}:需保留获准应用工具发现入口`);
    assert.deepEqual(forbiddenCalls, [], "继承配置中的MCP入口必须关闭");
    console.log(JSON.stringify({ phase, threadId, tools: names, forbiddenTools: 0, inheritedMcpRequests: 0 }));
  }
  await server.stop();
  const ownedSkillConfig = `[[skills.config]]\npath = ${JSON.stringify(join(home,"skills/.system/skill-creator/SKILL.md"))}\nenabled = false\n`;
  await writeFile(join(home,"config.toml"),ownedSkillConfig);
  server = start();
  assert.notEqual(server.pid,firstReportPid,"再次报告必须启动独立进程");
  await server.initialize();
  const nextConfig = await server.rpc("config/read",{includeLayers:false});
  assert.deepEqual(Object.keys(nextConfig.config.mcp_servers),["cilai_application"]);
  const nextThread = await server.rpc("thread/start",options);
  const nextTurn = await server.rpc("turn/start",{threadId:nextThread.thread.id,input:[{type:"text",text:"Second report in a fresh process."}]});
  await server.waitTurn(nextTurn.turn.id);
  assert.equal(await readFile(join(home,"config.toml"),"utf8"),ownedSkillConfig,"报告不能改写已有的受控skill禁用配置");
  console.log(JSON.stringify({reportProcessCount:2,existingDisabledCreatorConfig:"accepted",configChanged:false}));
  const resourceResult = requests.flatMap((request)=>request.input ?? []).find((item)=>item.type==="function_call_output" && item.call_id==="resource-call");
  assert.ok(resourceResult,"必须读回实际资源越界尝试的工具拒绝结果");
  assert.match(JSON.stringify(resourceResult.output), /(?:Error|error|failed|not_found|not found|rejected|unknown)/i);
  assert.doesNotMatch(JSON.stringify(resourceResult.output),/OUTSIDE_SECRET_BODY/);
  assert.ok(resourceRequests.every((entry)=>entry.method==="resources/read"));
  console.log(JSON.stringify({resourceEscape:"rejected",resourceRequests:resourceRequests.length,externalModelRequests:0}));
  console.log("report_materials_native_policy=passed; modelBillingRequests=0");
} finally {
  if (server) await server.stop();
}

} catch (error) { fixtureFailure = error; }
finally { await fixture.finish(fixtureFailure); }
