import { verificationEnvironment } from "./verification-environment.mjs";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdtemp, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { once } from "node:events";

const executable = process.argv[2];
if (!executable) throw new Error("请传入随包 Codex App Server 路径；本探针只使用本机模拟模型");
const root = await realpath(await mkdtemp(join(tmpdir(), "cilai-thread-probe-")));
const home = join(root, "harness");
const workspace = join(root, "workspace");
await Promise.all([mkdir(home), mkdir(workspace)]);
const requests = [];
let toolRevision = 1;
const backend = createServer(async (req, res) => {
  try {
    if (req.method !== "POST") { res.writeHead(405).end(); return; }
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = JSON.parse(Buffer.concat(chunks));
    if (req.url === "/mcp") {
      if (body.id === undefined) { res.writeHead(202).end(); return; }
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
    const item = { id: `msg-${requests.length}`, type: "message", role: "assistant", status: "completed", content: [{ type: "output_text", text: `local-reply-${requests.length}`, annotations: [] }] };
    res.writeHead(200, { "content-type": "text/event-stream" });
    for (const event of [
      { type: "response.created", response: { id, status: "in_progress" } },
      { type: "response.output_item.added", output_index: 0, item: { ...item, status: "in_progress", content: [] } },
      { type: "response.output_text.delta", item_id: item.id, output_index: 0, content_index: 0, delta: item.content[0].text },
      { type: "response.output_item.done", output_index: 0, item },
      { type: "response.completed", response: { id, status: "completed", output: [item], usage: { input_tokens: 100, input_tokens_details: { cached_tokens: 0 }, output_tokens: 10, total_tokens: 110 } } },
    ]) res.write(`data: ${JSON.stringify(event)}\n\n`);
    res.end();
  } catch (error) { res.writeHead(500).end(String(error)); }
});
backend.listen(0, "127.0.0.1");
await once(backend, "listening");
const origin = `http://127.0.0.1:${backend.address().port}`;
await writeFile(join(home, "config.toml"), `
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
[mcp_servers.probe]
url = "${origin}/mcp"
`);

function start() {
  const child = spawn(resolve(executable), ["--listen", "stdio://"], {
    cwd: workspace,
    env: verificationEnvironment(root, { CODEX_HOME: home, CODEX_SQLITE_HOME: home, OPENAI_API_KEY: "local-probe-only" }),
    stdio: ["pipe", "pipe", "pipe"],
  });
  const pending = new Map();
  const events = [];
  let id = 0;
  let stderr = "";
  child.stderr.on("data", (data) => { stderr = (stderr + data).slice(-6000); });
  const lines = createInterface({ input: child.stdout });
  lines.on("line", (line) => {
    const message = JSON.parse(line);
    const request = pending.get(message.id);
    if (!request) { events.push(message); return; }
    pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  });
  const fail = (error) => { for (const request of pending.values()) { clearTimeout(request.timer); request.reject(error); } pending.clear(); };
  child.on("error", fail);
  child.on("exit", (code) => fail(new Error(`Harness退出 ${code}: ${stderr}`)));
  const rpc = (method, params) => new Promise((resolveRequest, reject) => {
    const requestId = ++id;
    const timer = setTimeout(() => { pending.delete(requestId); reject(new Error(`RPC超时 ${method}: ${stderr}`)); }, 20000);
    pending.set(requestId, { resolve: resolveRequest, reject, timer });
    child.stdin.write(JSON.stringify({ id: requestId, method, params }) + "\n");
  });
  return {
    rpc, events,
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
      lines.close();
      if (child.exitCode === null) { const exited = once(child, "exit"); child.kill(); await exited; }
    },
  };
}

let server;
try {
  server = start();
  await server.initialize();
  const options = { cwd: workspace, model: "gpt-5.6-sol", modelProvider: "probe", approvalPolicy: "never", sandbox: "read-only" };
  const { thread } = await server.rpc("thread/start", { ...options, dynamicTools: [{ name: "stable_artifact", description: "Persistent dynamic definition", inputSchema: { type: "object", properties: {} } }] });
  await server.rpc("thread/name/set", {threadId:thread.id,name:"公开协议持久化验证"});
  await server.rpc("thread/inject_items", { threadId: thread.id, items: [
    { type: "message", role: "user", content: [{ type: "input_text", text: "imported-fact-17" }] },
    { type: "message", role: "assistant", content: [{ type: "output_text", text: "imported-reply-17" }] },
    { type: "tool_search_call", call_id: "search-1", execution: "client", status: "completed", arguments: { query: "prior tool" } },
    { type: "tool_search_output", call_id: "search-1", execution: "client", status: "completed", tools: [{ type: "function", name: "prior_definition", description: "previously loaded", parameters: { type: "object", properties: {} } }] },
  ] });
  assert.equal(requests.length, 0, "历史注入本身不能发模型请求");
  await server.rpc("thread/name/set", {threadId:thread.id,name:"公开协议持久化验证"});
  await server.stop(); server = start(); await server.initialize();
  await server.rpc("thread/resume", {...options,threadId:thread.id,excludeTurns:true});
  await server.rpc("thread/name/set", {threadId:thread.id,name:"公开协议持久化验证"});
  const emptyPage=await server.rpc("thread/turns/list",{threadId:thread.id,itemsView:"full"});
  assert.deepEqual(emptyPage.data,[],"首次真实消息前，历史补充必须可重启恢复且不伪造可见轮次");
  const first = await server.rpc("turn/start", { threadId: thread.id, clientUserMessageId: "client-1", input: [{ type: "text", text: "first-new-message" }] });
  await server.waitTurn(first.turn.id);
  assert.match(JSON.stringify(requests[0].input), /imported-fact-17/);
  assert.match(JSON.stringify(requests[0].input), /prior_definition/);
  assert.match(JSON.stringify(requests[0]), /stable_artifact/);
  const read = await server.rpc("thread/read", { threadId: thread.id, includeTurns: true });
  const injectedHistoryQueryable = JSON.stringify(read.thread.turns).includes("imported-fact-17");
  const clientMessageQueryable = JSON.stringify(read.thread.turns).includes("client-1");
  assert.ok(clientMessageQueryable, "接收核对必须能读回客户端消息标识");
  await server.stop();
  toolRevision = 2;
  server = start();
  await server.initialize();
  const resumed = await server.rpc("thread/resume", { ...options, threadId: thread.id });
  assert.equal(resumed.thread.id, thread.id);
  const mcp = await server.rpc("mcpServerStatus/list", { threadId: thread.id, detail: "toolsAndAuthOnly", limit: 100 });
  assert.match(JSON.stringify(mcp), /query_v2/);
  assert.doesNotMatch(JSON.stringify(mcp), /query_v1/);
  const second = await server.rpc("turn/start", { threadId: thread.id, clientUserMessageId: "client-2", input: [{ type: "text", text: "second-new-message" }] });
  await server.waitTurn(second.turn.id);
  assert.match(JSON.stringify(requests[1].input), /first-new-message/);
  assert.match(JSON.stringify(requests[1].input), /imported-fact-17/);
  assert.match(JSON.stringify(requests[1].input), /prior_definition/);
  assert.match(JSON.stringify(requests[1]), /stable_artifact/);
  const forked = await server.rpc("thread/fork", { ...options, threadId: thread.id, beforeTurnId: second.turn.id });
  assert.notEqual(forked.thread.id, thread.id);
  const forkView = JSON.stringify(forked.thread.turns);
  assert.match(forkView, /first-new-message/);
  assert.doesNotMatch(forkView, /second-new-message/);
  for (let round = 3; round <= 20; round += 1) {
    await server.stop(); server = start(); await server.initialize();
    const model = round === 10 ? "gpt-5.6-terra" : options.model;
    const continued = await server.rpc("thread/resume", { ...options, model, threadId: thread.id });
    assert.equal(continued.thread.id, thread.id);
    const turn = await server.rpc("turn/start", { threadId: thread.id, clientUserMessageId: `client-${round}`, input: [{ type: "text", text: `message-${round}` }] });
    await server.waitTurn(turn.turn.id);
    assert.match(JSON.stringify(requests.at(-1)), /stable_artifact/);
    if (round < 10) assert.match(JSON.stringify(requests.at(-1).input), /imported-fact-17/);
    if (round === 10 || round === 18) {
      const offset = server.events.length;
      await server.rpc("thread/compact/start", { threadId: thread.id });
      const deadline = Date.now() + 20000;
      while (!server.events.slice(offset).some((event) => event.method === "turn/completed")) {
        if (Date.now() > deadline) throw new Error("本机压缩未完成");
        await new Promise((done) => setTimeout(done, 20));
      }
      const completed = server.events.slice(offset).find((event) => event.method === "turn/completed");
      assert.equal(completed.params.turn.status, "completed");
    }
  }
  await server.rpc("thread/delete", { threadId: forked.thread.id });
  await assert.rejects(server.rpc("thread/read", { threadId: forked.thread.id, includeTurns: true }));
  assert.equal((await server.rpc("thread/read", { threadId: thread.id, includeTurns: true })).thread.id, thread.id);
  console.log(JSON.stringify({ persistentResume: true, rounds: 20, compactions: 2, modelSwitchRoundTrip: true, isolatedDeletion: true, dynamicToolsRetained: true, toolSearchHistoryRetained: true, mcpRefreshed: true, forkBeforeTurn: true, clientMessageQueryable, injectedHistoryQueryable, localModelRequests: requests.length, paidModelRequests: 0 }));
} finally {
  if (server) await server.stop();
  backend.closeAllConnections();
  await new Promise((done) => backend.close(done));
  await rm(root, { recursive: true, force: true });
}
