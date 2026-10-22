const assert = require("node:assert/strict");
const test = require("node:test");
const { BrowserMcpServer } = require("./browser-mcp");

/** 运行时的替身：只记录每个工具收到了什么参数。 */
function recordingRuntime() {
  const calls = [];
  return {
    calls,
    proxy: new Proxy({}, {
      get: (_target, name) => {
        // current() 是取"这次对话当前那个标签"，审批摘要要用它，不能当成被记录的工具调用
        if (name === "current") {
          return () => ({ contents: { isDestroyed: () => false, getURL: () => "https://example.com/x" } });
        }
        return (...args) => {
          calls.push({ name, args });
          return { ok: true };
        };
      },
    }),
  };
}

const alwaysApprove = { request: async () => ({ approved: true }) };

const callTool = (server, name, args, conversationId) => server.handle({
  jsonrpc: "2.0",
  id: 1,
  method: "tools/call",
  params: { name, arguments: args },
}, conversationId);

/*
 * 对话标识一路传到每个工具，是浏览器"归属发起它的那次对话"的全部依据。
 *
 * 它断过一次：标识只是 handle 的参数，而真正调用工具的 invoke 与 invokeWrite 从来没收到它，
 * 于是每个工具调用都抛 `conversationId is not defined`。表现极具迷惑性——工具全部失败，
 * 模型只拿到一句错误文本，照样回了"百度首页已经打开了"，而员工看到的浏览器是空白的。
 * 这类"失败被模型描述成成功"的组合，只有在调用层钉住才拦得下来。
 */
test("每个读工具都把对话标识交给运行时", async () => {
  const runtime = recordingRuntime();
  const server = new BrowserMcpServer(runtime.proxy, alwaysApprove);
  const tools = [
    ["browser_navigate", { url: "https://example.com/" }],
    ["browser_tabs", {}],
    ["browser_snapshot", {}],
    ["browser_extract", {}],
    ["browser_screenshot", {}],
    ["browser_scroll", {}],
    ["browser_wait_for", { expression: "1" }],
  ];
  for (const [name, args] of tools) await callTool(server, name, args, "conv-7");
  assert.equal(runtime.calls.length, tools.length, "每个工具都该落到运行时上");
  for (const call of runtime.calls) {
    assert.ok(call.args.includes("conv-7"), `${call.name} 没有把对话标识传下去`);
  }
});

test("写工具经过审批之后同样带着对话标识", async () => {
  const runtime = recordingRuntime();
  const server = new BrowserMcpServer(runtime.proxy, alwaysApprove);
  await callTool(server, "browser_click", { x: 1, y: 2, purpose: "试" }, "conv-9");
  await callTool(server, "browser_type", { text: "你好", purpose: "试" }, "conv-9");
  assert.deepEqual(runtime.calls.map((call) => call.name), ["click", "type"]);
  for (const call of runtime.calls) assert.ok(call.args.includes("conv-9"));
});

test("工具失败时错误如实回给模型，不伪装成成功", async () => {
  const runtime = {
    navigate: async () => { throw new Error("站点打不开"); },
  };
  const server = new BrowserMcpServer(runtime, alwaysApprove);
  const reply = await callTool(server, "browser_navigate", { url: "https://example.com/" }, "conv-1");
  const text = JSON.stringify(reply);
  assert.ok(text.includes("站点打不开"), text);
});

/*
 * 注解决定 harness 在"帮我批准"下问不问。没有注解会被按"有破坏性"处理，
 * 读页面也要员工点一次，与员工选的执行方式对不上。
 */
test("每个工具都声明了读写性质", async () => {
  const runtime = recordingRuntime();
  const server = new BrowserMcpServer(runtime.proxy, alwaysApprove);
  const listed = await server.handle({ jsonrpc: "2.0", id: 1, method: "tools/list", params: {} }, "conv-1");
  const tools = listed.result.tools;
  assert.ok(tools.length >= 9, `工具数量异常：${tools.length}`);
  for (const tool of tools) {
    assert.ok(tool.annotations, `${tool.name} 没有声明注解`);
    assert.equal(typeof tool.annotations.readOnlyHint, "boolean", tool.name);
    assert.equal(typeof tool.annotations.destructiveHint, "boolean", tool.name);
  }
  const readOnly = tools.filter((tool) => tool.annotations.readOnlyHint).map((tool) => tool.name);
  assert.ok(readOnly.includes("browser_snapshot"), "读页面结构应当是只读的");
  assert.ok(readOnly.includes("browser_extract"), "取文应当是只读的");
  // 留下痕迹的那两个任何时候都要员工确认
  const writing = tools.filter((tool) => tool.annotations.destructiveHint).map((tool) => tool.name);
  // browser_batch 里可以装写操作，注解必须按最宽的那一项声明，否则写操作会绕过"帮我批准"
  assert.deepEqual(writing.sort(),
    ["browser_batch", "browser_click", "browser_set_value", "browser_type"]);
});

/*
 * 业界这两年收敛出来的做法：快照发句柄，操作按句柄定位，而不是按坐标。
 *
 * 此前快照返回的是可访问性树的内部节点号，而点击只收坐标——那个号没有任何工具能用，
 * AI 只能靠截图目测像素去点。而截图在标签没呈现时根本取不到，等于常见情况下点不准。
 */
test("点击优先按句柄定位，句柄缺失时才用坐标", async () => {
  const seen = [];
  const runtime = {
    click: async (target) => { seen.push(target); return { ok: true }; },
    current: () => ({ contents: { isDestroyed: () => false, getURL: () => "https://example.com/" } }),
  };
  const server = new BrowserMcpServer(runtime, alwaysApprove);
  await callTool(server, "browser_click", { ref: "ref_12", purpose: "试" }, "c1");
  await callTool(server, "browser_click", { x: 10, y: 20, purpose: "试" }, "c1");
  assert.equal(seen[0].ref, "ref_12");
  assert.equal(seen[1].x, 10);
});

test("表单设值按句柄下发，值原样传给运行时", async () => {
  const seen = [];
  const runtime = {
    setValue: async (ref, value) => { seen.push({ ref, value }); return { ok: true }; },
    current: () => ({ contents: { isDestroyed: () => false, getURL: () => "https://example.com/" } }),
  };
  const server = new BrowserMcpServer(runtime, alwaysApprove);
  await callTool(server, "browser_set_value",
    { ref: "ref_3", value: "一般工业固体废物", purpose: "试" }, "c1");
  assert.deepEqual(seen, [{ ref: "ref_3", value: "一般工业固体废物" }]);
});

test("批量按顺序执行，遇到第一个失败就停下并说明停在哪一步", async () => {
  const runtime = recordingRuntime();
  let calls = 0;
  const server = new BrowserMcpServer(new Proxy({}, {
    get: (_t, name) => {
      if (name === "current") return () => ({ contents: { isDestroyed: () => false, getURL: () => "https://example.com/" } });
      return async () => {
        calls += 1;
        if (calls === 2) throw new Error("第二步失败");
        return { ok: true };
      };
    },
  }), alwaysApprove);
  const reply = await callTool(server, "browser_batch", {
    actions: [
      { name: "browser_navigate", arguments: { url: "https://example.com/" } },
      { name: "browser_snapshot", arguments: {} },
      { name: "browser_extract", arguments: {} },
    ],
  }, "c1");
  // 工具结果是一段 JSON 文本，套在 MCP 的内容项里，取出来再解析
  const payload = JSON.parse(reply.result.content[0].text);
  assert.equal(payload.stoppedAt, 2);
  assert.equal(payload.completed, 1);
  assert.equal(payload.results[1].error, "第二步失败");
  // 第三步不执行：后面的步骤依赖前面的结果，在错误的页面状态上继续叠加操作没有意义
  assert.equal(calls, 2);
  assert.ok(runtime.calls.length === 0);
});

test("批量不能嵌套自身", async () => {
  const runtime = recordingRuntime();
  const server = new BrowserMcpServer(runtime.proxy, alwaysApprove);
  const reply = await callTool(server, "browser_batch",
    { actions: [{ name: "browser_batch", arguments: { actions: [] } }] }, "c1");
  assert.ok(JSON.stringify(reply).includes("不能嵌套"));
});

test("控制台与网络记录可读，让 AI 能自证这一步成没成", async () => {
  const seen = [];
  const runtime = {
    consoleMessages: async (options) => { seen.push(["console", options]); return { entries: [] }; },
    networkRequests: async (options) => { seen.push(["network", options]); return { entries: [] }; },
    current: () => ({ contents: { isDestroyed: () => false, getURL: () => "https://example.com/" } }),
  };
  const server = new BrowserMcpServer(runtime, alwaysApprove);
  await callTool(server, "browser_console", { level: "error" }, "c1");
  await callTool(server, "browser_network", { failedOnly: true }, "c1");
  assert.equal(seen[0][1].level, "error");
  assert.equal(seen[1][1].failedOnly, true);
});


test("应用采集只允许授权来源和只读工具，拒绝跨域标签与重定向", async()=>{
 const scope="application:com.cilai.environmental.policy:job",origins=["https://example.com"];
 const runtime={current:()=>({contents:{getURL:()=>"https://example.com/a"}}),listTabs:()=>({tabs:[{id:"foreign",url:"https://other.example/a"}]}),extract:async()=>({text:"政策"}),navigate:async()=>({url:"https://other.example/redirect"})};
 const server=new BrowserMcpServer(runtime,alwaysApprove);
 const invoke=(name,args={})=>server.handle({jsonrpc:"2.0",id:1,method:"tools/call",params:{name,arguments:args}},scope,origins);
 assert.equal((await invoke("browser_extract")).result.isError,false);
 assert.equal((await invoke("browser_extract",{tabId:"foreign"})).result.isError,true);
 assert.equal((await invoke("browser_click",{ref:"x"})).result.isError,true);
 assert.equal((await invoke("browser_navigate",{url:"https://outside.example/a"})).result.isError,true);
 assert.equal((await invoke("browser_navigate",{url:"https://example.com/a"})).result.isError,true);
 const listed=await server.handle({jsonrpc:"2.0",id:2,method:"tools/list"},scope,origins);
 assert.equal(listed.result.tools.some(t=>t.name==="browser_click"),false);
});


test("应用采集声明来源内的导航可只读执行，普通对话保持导航审批语义", async () => {
  const runtime = recordingRuntime();
  const server = new BrowserMcpServer(runtime.proxy, alwaysApprove);
  const message = {jsonrpc:"2.0",id:1,method:"tools/list",params:{}};
  const application = await server.handle(message, "application:com.cilai.test:run", ["https://example.com"]);
  const normal = await server.handle(message, "conversation");
  assert.equal(application.result.tools.find(t=>t.name === "browser_navigate").annotations.readOnlyHint, true);
  assert.equal(normal.result.tools.find(t=>t.name === "browser_navigate").annotations.readOnlyHint, false);
  assert.ok(application.result.tools.every(t=>t.annotations.readOnlyHint));
  const denied = await server.handle({jsonrpc:"2.0",id:2,method:"tools/call",params:{name:"browser_navigate",arguments:{url:"https://other.example/"}}}, "application:com.cilai.test:run", ["https://example.com"]);
  assert.equal(denied.result.isError, true);
  assert.equal(runtime.calls.length, 0);
});

test("独立应用任务只能导航并读取本次来源，拒绝写入和越界", async () => {
  const scope = "application-task:app_run_synthetic_collection_plan";
  const origins = ["https://example.com"];
  const calls = [];
  const runtime = {
    current: () => ({ contents: { getURL: () => "https://example.com/policy" } }),
    listTabs: () => ({ tabs: [{ id: "foreign", url: "https://other.example/policy" }] }),
    navigate: async (url, conversationId) => {
      calls.push({ name: "navigate", conversationId });
      return { url };
    },
    extract: async (selector, tabId, conversationId) => {
      calls.push({ name: "extract", conversationId });
      return { text: "合成政策正文" };
    },
  };
  const server = new BrowserMcpServer(runtime, alwaysApprove);
  const message = { jsonrpc: "2.0", id: 1, method: "tools/list" };
  const listed = (await server.handle(message, scope, origins)).result.tools;
  assert.deepEqual(listed.map(tool => tool.name).sort(), ["browser_attachment_capture", "browser_collect", "browser_extract", "browser_navigate", "browser_screenshot", "browser_snapshot", "browser_tabs"]);
  assert.ok(listed.every(tool => tool.annotations.readOnlyHint));
  const invoke = (name, args = {}, permitted = origins) => server.handle({ jsonrpc: "2.0", id: 2, method: "tools/call", params: { name, arguments: args } }, scope, permitted);
  assert.equal((await invoke("browser_navigate", { url: "https://example.com/policy" })).result.isError, false);
  assert.equal((await invoke("browser_extract")).result.isError, false);
  assert.deepEqual(calls.map(call => call.name), ["navigate", "extract"]);
  assert.ok(calls.every(call => call.conversationId === scope));
  assert.equal((await invoke("browser_navigate", { url: "https://example.com/policy" }, [])).result.isError, true);
  assert.equal((await invoke("browser_navigate", { url: "https://other.example/policy" })).result.isError, true);
  assert.equal((await invoke("browser_extract", { tabId: "foreign" })).result.isError, true);
  assert.equal((await invoke("browser_click", { ref: "submit" })).result.isError, true);
  assert.equal((await invoke("browser_batch", { actions: [] })).result.isError, true);
  assert.equal(calls.length, 2);
  runtime.navigate = async () => ({ url: "https://other.example/redirect" });
  assert.equal((await invoke("browser_navigate", { url: "https://example.com/policy" })).result.isError, true);
  const normal = (await server.handle(message, "conversation")).result.tools;
  assert.equal(normal.find(tool => tool.name === "browser_navigate").annotations.readOnlyHint, false);
  assert.ok(normal.some(tool => tool.name === "browser_click"));
});
