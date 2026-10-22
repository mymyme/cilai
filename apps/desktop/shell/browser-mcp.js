const http = require("node:http");
const crypto = require("node:crypto");

/**
 * AI 浏览器的本地 MCP 服务。
 *
 * 工具以本地 MCP 形式向 Harness 注册，与企业应用的 MCP 并列——这样 AI 调用浏览器与调用
 * 业务应用走同一套机制，不必在对话运行时里另开一条特殊通路。
 *
 * 只监听回环地址，端口由系统分配，并要求逐次携带令牌：随机端口本身不是保护，本机上的
 * 任何进程都能扫到它。令牌只经环境变量交给 Harness 进程，不落盘。
 *
 * 当前只注册读工具。点击与输入具有外部副作用，须经审批链路逐次确认，链路接入之前不在
 * 这里暴露——暴露了就等于放行。
 */

/**
 * 工具调用所属的对话。
 *
 * 由 Harness 以请求头带来：浏览器是某一次对话里开出来的，切到别的对话就不该看到它。
 * 缺头时按"不属于任何对话"处理，而不是猜一个——猜错会把标签挂到别人的对话上。
 */
function conversationOf(request) {
  const value = request.headers["x-cilai-conversation-id"];
  const text = Array.isArray(value) ? value[0] : value;
  return typeof text === "string" && text.trim() ? text.trim() : null;
}

const PROTOCOL_VERSION = "2025-06-18";

/** 凭据字段一律不由 AI 填写，遇到即转人工接管。判定放在工具入口，早于任何输入动作。 */
const CREDENTIAL_HINTS = [
  "password", "passwd", "密码", "验证码", "captcha", "otp", "cvv", "身份证", "银行卡", "支付",
];

/** 写工具：会在外部站点留下真实痕迹，逐次经员工确认后才执行。 */
function writeTools() {
  return [
    {
      name: "browser_click",
      // 会在页面上留下痕迹，任何审批方式下都要员工确认
      annotations: { readOnlyHint: false, destructiveHint: true, openWorldHint: true },
      description: "点击页面元素。优先用 browser_snapshot 给出的 ref 句柄定位，句柄绑在元素上，页面滚动或窗口改变都不受影响；只有快照里没有的元素（如画布内图形）才退到坐标。会留下痕迹，每次都需要员工确认。",
      inputSchema: {
        type: "object",
        properties: {
          ref: { type: "string", description: "browser_snapshot 给出的元素句柄，如 ref_12。优先使用" },
          x: { type: "number", description: "页面内横坐标。仅在没有句柄时使用" },
          y: { type: "number", description: "页面内纵坐标。仅在没有句柄时使用" },
          purpose: { type: "string", description: "这次点击要达成什么，供员工判断是否批准" },
        },
        required: ["purpose"],
      },
    },
    {
      name: "browser_set_value",
      // 直接改表单值，同样留下痕迹
      annotations: { readOnlyHint: false, destructiveHint: true, openWorldHint: true },
      description: "给表单元素直接设值：下拉框选项、复选框勾选、输入框文本都用它，比点开再点选项可靠得多，并会派发页面能听到的 input 与 change 事件。需要员工确认。密码、验证码、支付信息与证件号一律不由 AI 填写。",
      inputSchema: {
        type: "object",
        properties: {
          ref: { type: "string", description: "browser_snapshot 给出的元素句柄" },
          value: { description: "要设置的值。下拉框传选项文本或值，复选框传 true/false" },
          fieldHint: { type: "string", description: "目标字段的名称或含义" },
          purpose: { type: "string", description: "这次设值要达成什么，供员工判断是否批准" },
        },
        required: ["ref", "value", "purpose"],
      },
    },
    {
      name: "browser_type",
      // 会在页面上留下痕迹，任何审批方式下都要员工确认
      annotations: { readOnlyHint: false, destructiveHint: true, openWorldHint: true },
      description: "在当前焦点处输入文本。每次都需要员工确认。密码、验证码、支付信息与证件号一律不由 AI 填写，遇到这类字段应转人工接管。",
      inputSchema: {
        type: "object",
        properties: {
          text: { type: "string", description: "要输入的文本" },
          fieldHint: { type: "string", description: "目标字段的名称或含义" },
          purpose: { type: "string", description: "这次输入要达成什么，供员工判断是否批准" },
        },
        required: ["text", "purpose"],
      },
    },
  ];
}

/** 读工具：不改变页面状态，因此不需要逐次审批，但仍受站点策略约束。 */
const applicationReadTools = new Set(["browser_navigate","browser_tabs","browser_snapshot","browser_extract","browser_screenshot","browser_collect","browser_attachment_capture"]);
function readTools() {
  return [
    { name: "browser_attachment_capture", annotations: {readOnlyHint:true,destructiveHint:false,openWorldHint:true}, description:"下载本轮实际页面声明的附件原件。仅接受本次授权来源，跳转逐次核验，返回字节与摘要，不写本地文件。", inputSchema:{type:"object",additionalProperties:false,properties:{url:{type:"string",minLength:1,maxLength:2000},name:{type:"string",minLength:1,maxLength:240},tabId:{type:"string"}},required:["url","name"]}},
    { name: "browser_collect", annotations: {readOnlyHint:true,destructiveHint:false,openWorldHint:true}, description:"按CSS选择规则读取完整正文和链接，超限明确失败。", inputSchema:{type:"object",additionalProperties:false,properties:{selector:{type:"string",minLength:1,maxLength:300},linkSelector:{type:"string",maxLength:300},attachmentCapture:{type:"boolean"},tabId:{type:"string"}},required:["selector","linkSelector"]}},
    {
      name: "browser_navigate",
      // 会让页面动起来，但不写入任何业务数据，也不可能造成破坏
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: true },
      description: "在受控浏览器中打开指定地址。地址须落在已声明的可访问站点内，否则拒绝并说明原因。",
      inputSchema: {
        type: "object",
        properties: { url: { type: "string", description: "要打开的完整地址" } },
        required: ["url"],
      },
    },
    {
      name: "browser_tabs",
      // 只读：不改变页面，也不留下痕迹。审批方式选"帮我批准"时自动放行
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
      description: "列出浏览器里当前打开的全部标签，含员工自己打开的。要读员工正在看的页面时先用它拿到标签标识。",
      inputSchema: { type: "object", properties: {} },
    },
    {
      name: "browser_snapshot",
      // 只读：不改变页面，也不留下痕迹。审批方式选"帮我批准"时自动放行
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
      description: "返回当前页面的可访问性树：元素的角色与名称。理解页面结构应优先用它，它比截图更准确也更省上下文。",
      inputSchema: {
        type: "object",
        properties: {
          maxNodes: { type: "number", description: "最多返回的节点数" },
        tabId: { type: "string", description: "要读取的标签，省略则读当前标签" },
        },
      },
    },
    {
      name: "browser_extract",
      // 只读：不改变页面，也不留下痕迹。审批方式选"帮我批准"时自动放行
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
      description: "提取页面文本。给出选择器时只取匹配的元素，否则取整页正文。",
      inputSchema: {
        type: "object",
        properties: {
          selector: { type: "string", description: "CSS 选择器，可省略" },
        tabId: { type: "string", description: "要读取的标签，省略则读当前标签" },
        },
      },
    },
    {
      name: "browser_screenshot",
      // 只读：不改变页面，也不留下痕迹。审批方式选"帮我批准"时自动放行
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
      description: "返回页面截图。可访问性树不足以判断的视觉问题才用它。",
      inputSchema: {
        type: "object",
        properties: {
        tabId: { type: "string", description: "要读取的标签，省略则读当前标签" },
        },
      },
    },
    {
      name: "browser_scroll",
      // 会让页面动起来，但不写入任何业务数据，也不可能造成破坏
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: true },
      description: "滚动页面。正值向下，负值向上。",
      inputSchema: {
        type: "object",
        properties: { deltaY: { type: "number", description: "滚动量，默认向下一屏的一部分" } },
      },
    },
    {
      name: "browser_batch",
      /*
       * 批量本身不改变页面，但里面可以装写工具。注解按最宽的那一项声明：
       * 声明成只读会让写操作绕过"帮我批准"，而批内的每一步仍各自走自己的审批。
       */
      annotations: { readOnlyHint: false, destructiveHint: true, openWorldHint: true },
      description: "一次往返里按顺序执行多个浏览器动作，遇到第一个失败就停下并返回已完成的部分。导航后接读取、或连续多步操作时用它，能省掉每一步之间的模型往返。批内的写操作仍各自需要员工确认。",
      inputSchema: {
        type: "object",
        properties: {
          actions: {
            type: "array",
            description: "按顺序执行的动作，每项形如 { name, arguments }",
            items: {
              type: "object",
              properties: {
                name: { type: "string", description: "工具名，不能是 browser_batch 自身" },
                arguments: { type: "object", description: "该工具的参数" },
              },
              required: ["name"],
            },
          },
        },
        required: ["actions"],
      },
    },
    {
      name: "browser_console",
      // 只读：读已经发生的记录，不改变页面
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
      description: "读页面近期的控制台记录。操作完成后用它确认这一步到底成没成——工具返回成功但页面报错的情况，只有这里看得见。传 level=\"error\" 只看报错。",
      inputSchema: {
        type: "object",
        properties: {
          level: { type: "string", description: "只看某一级别，如 error、warning" },
          limit: { type: "number", description: "最多返回多少条，默认 50" },
          tabId: { type: "string", description: "要读哪个标签，缺省读当前标签" },
        },
      },
    },
    {
      name: "browser_network",
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
      description: "读页面近期的网络请求记录。页面空白、内容加载不出来时用它定位原因；传 failedOnly=true 只看失败和 4xx、5xx 的请求。",
      inputSchema: {
        type: "object",
        properties: {
          failedOnly: { type: "boolean", description: "只看失败与 4xx、5xx" },
          limit: { type: "number", description: "最多返回多少条，默认 50" },
          tabId: { type: "string", description: "要读哪个标签，缺省读当前标签" },
        },
      },
    },
    {
      name: "browser_wait_for",
      // 只读：不改变页面，也不留下痕迹。审批方式选"帮我批准"时自动放行
      annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: true },
      description: "等待页面上的条件成立，避免以固定延时代替确定性等待。",
      inputSchema: {
        type: "object",
        properties: {
          expression: { type: "string", description: "在页面中求值的表达式，为真即视为成立" },
          timeoutMs: { type: "number", description: "最长等待毫秒数" },
        },
        required: ["expression"],
      },
    },
  ];
}

class BrowserMcpServer {
  constructor(runtime, approvals) {
    this.runtime = runtime;
    this.approvals = approvals;
    this.token = crypto.randomBytes(32).toString("hex");
    this.server = null;
    this.endpoint = null;
  }

  /** 逐次校验令牌。缺失或不符一律拒绝，不因为来自回环就放行。 */
  authorized(request) {
    const header = request.headers.authorization ?? "";
    const value = header.startsWith("Bearer ") ? header.slice(7) : "";
    const expected = Buffer.from(this.token);
    const actual = Buffer.from(value);
    return actual.length === expected.length && crypto.timingSafeEqual(actual, expected);
  }

  async handle(message, conversationId, origins = []) {
    const application = typeof conversationId === "string" && (conversationId.startsWith("application:") || conversationId.startsWith("application-task:"));
    const { id, method, params } = message;
    if (method === "initialize") {
      return {
        jsonrpc: "2.0",
        id,
        result: {
          protocolVersion: PROTOCOL_VERSION,
          capabilities: { tools: {} },
          serverInfo: { name: "cilai-browser", version: "0.1.0" },
        },
      };
    }
    if (method === "tools/list") {
      // 没有审批通道就不暴露写工具：暴露了却无人确认，等于要么卡死要么放行
      const tools = application
        ? readTools().filter(tool => applicationReadTools.has(tool.name)).map(tool =>
            tool.name === "browser_navigate" ? { ...tool, annotations: { ...tool.annotations, readOnlyHint: true } } : tool)
        : this.approvals ? [...readTools().filter(tool => tool.name !== "browser_attachment_capture"), ...writeTools()] : readTools().filter(tool => tool.name !== "browser_attachment_capture");
      return { jsonrpc: "2.0", id, result: { tools } };
    }
    if (method === "tools/call") {
      const args = params?.arguments ?? {};
      try {
        if (application) {
          if (!applicationReadTools.has(params?.name)) throw new Error("应用采集仅允许读取页面");
          if (!Array.isArray(origins) || !origins.length || origins.length > 50 || origins.some(v => typeof v !== "string" || new URL(v).origin !== v)) throw new Error("采集来源未绑定");
          const targetTab = args.tabId ? this.runtime.listTabs(conversationId).tabs.find(tab => tab.id === args.tabId) : null;
          if (args.tabId && !targetTab) throw new Error("标签页不属于本次采集");
          const target = params?.name === "browser_navigate" ? String(args.url ?? "") : args.tabId ? targetTab.url : this.runtime.current?.(conversationId)?.contents?.getURL?.();
          if (params?.name !== "browser_tabs" && (!target || !origins.includes(new URL(target).origin))) throw new Error("页面超出本次采集来源");
        }
        const result = await this.invoke(params?.name, args, conversationId, origins);
        if (application && params?.name === "browser_navigate" && !origins.includes(new URL(result.url).origin)) throw new Error("跳转页面超出本次采集来源");
        return {
          jsonrpc: "2.0",
          id,
          result: { content: [{ type: "text", text: JSON.stringify(result) }], isError: false },
        };
      } catch (error) {
        // 失败以工具结果回给模型，让它看到原因并改变做法，而不是当作协议错误中断整轮
        return {
          jsonrpc: "2.0",
          id,
          result: {
            content: [{ type: "text", text: String(error?.message ?? error) }],
            isError: true,
          },
        };
      }
    }
    return { jsonrpc: "2.0", id, error: { code: -32601, message: `未提供该方法：${method}` } };
  }

  /** 当前页面地址，用于让员工看清这次操作发生在哪个站点。 */
  /*
   * 审批卡片上写的"在哪个站点"。取的是这次对话当前那个标签的地址——早先读的是运行时的
   * 单一窗口，而运行时改成按标签组织之后那个属性就没了，于是卡片一直写着"未打开页面"，
   * 员工看不出 AI 要在哪里点。
   */
  currentSite(conversationId) {
    const tab = this.runtime.current?.(conversationId);
    const contents = tab?.contents;
    const url = contents && !contents.isDestroyed() ? contents.getURL() : "";
    try {
      return new URL(url).origin;
    } catch {
      return "未打开页面";
    }
  }

  /** 写操作：先确认凭据红线，再逐次请员工批准，两关都过才执行。 */
  async invokeWrite(name, args, conversationId) {
    if (name === "browser_type" || name === "browser_set_value") {
      const hint = `${args.fieldHint ?? ""}`.toLowerCase();
      if (CREDENTIAL_HINTS.some((word) => hint.includes(word))) {
        throw new Error("凭据字段不由 AI 填写，请转人工接管");
      }
    }
    const site = this.currentSite(conversationId);
    const purpose = args.purpose ?? "未说明用途";
    // 目标写清楚：句柄就报句柄，坐标就报坐标——员工要看得出 AI 到底要动哪个元素
    const target = args.ref ? `元素 ${args.ref}` : `(${args.x}, ${args.y})`;
    const summary = name === "browser_click" ? `在 ${site} 点击${target}：${purpose}`
      : name === "browser_set_value" ? `在 ${site} 把${target}设为「${args.value}」：${purpose}`
      : `在 ${site} 输入文本：${purpose}`;
    /*
     * 凭据类字段永远单问，不受站点授权覆盖：这类风险不随"这个站点我信任"而降低。
     * 上面的红线拦的是明确命中关键词的那些，这里拦的是所有涉及写入文本的操作——
     * 员工授权"在这个站点点击"不等于授权"在这个站点输入任何内容"。
     */
    const alwaysAsk = name === "browser_type" || name === "browser_set_value";
    const decision = await this.approvals.request({
      tool: name,
      site,
      scope: conversationId ?? null,
      alwaysAsk,
      summary,
      arguments: name === "browser_click"
        ? { ref: args.ref, x: args.x, y: args.y, purpose: args.purpose }
        : name === "browser_set_value"
          // 待写入的值原样呈现：判断该不该写，必须看见写什么
          ? { ref: args.ref, value: args.value, fieldHint: args.fieldHint, purpose: args.purpose }
          : { text: args.text, fieldHint: args.fieldHint, purpose: args.purpose },
    });
    if (!decision.approved) throw new Error(decision.reason);
    if (name === "browser_click") {
      return this.runtime.click({ ref: args.ref, x: args.x, y: args.y }, conversationId);
    }
    if (name === "browser_set_value") {
      return this.runtime.setValue(String(args.ref ?? ""), args.value, conversationId);
    }
    return this.runtime.type(String(args.text ?? ""), conversationId);
  }

  /*
   * 对话标识必须一路传到每个工具：它决定这次操作落在哪个对话的标签上。漏传不会静默
   * 退化成"用当前标签"，而是直接抛引用错误——工具全部失败，而模型只看到一句错误文本，
   * 很可能照样编一段"已经打开了"，员工看到的是浏览器空白但 AI 说成功。
   */
  /**
   * 按顺序执行多个动作，遇到第一个失败就停。
   *
   * 不吞错也不跳过：后面的步骤通常依赖前面的结果，前一步没成还继续做，只会在错误的页面
   * 状态上叠加操作。已完成的部分照常返回，AI 据此知道走到哪里断的。
   */
  async invokeBatch(args, conversationId) {
    const actions = Array.isArray(args.actions) ? args.actions : [];
    if (!actions.length) throw new Error("browser_batch 需要至少一个动作");
    const results = [];
    for (const [index, action] of actions.entries()) {
      const name = String(action?.name ?? "");
      if (name === "browser_batch") throw new Error("browser_batch 不能嵌套");
      try {
        results.push({ step: index + 1, name, ok: true,
          result: await this.invoke(name, action?.arguments ?? {}, conversationId) });
      } catch (error) {
        results.push({ step: index + 1, name, ok: false, error: String(error?.message ?? error) });
        return { completed: results.length - 1, total: actions.length, stoppedAt: index + 1, results };
      }
    }
    return { completed: results.length, total: actions.length, stoppedAt: null, results };
  }

  invoke(name, args, conversationId, origins = []) {
    if (name === "browser_batch") return this.invokeBatch(args, conversationId);
    if (name === "browser_click" || name === "browser_type" || name === "browser_set_value") {
      if (!this.approvals) throw new Error("审批通道不可用，写操作不予执行");
      return this.invokeWrite(name, args, conversationId);
    }
    switch (name) {
      case "browser_navigate":
        return this.runtime.navigate(String(args.url ?? ""), conversationId);
      case "browser_tabs":
        return this.runtime.listTabs(conversationId);
      case "browser_snapshot":
        return this.runtime.snapshot(args.maxNodes, args.tabId, conversationId);
      case "browser_attachment_capture":
        return this.runtime.captureAttachment(args, conversationId, origins);
      case "browser_collect":
        return this.runtime.collect(args.selector, args.linkSelector, args.tabId, conversationId, args.attachmentCapture === true);
      case "browser_extract":
        return this.runtime.extract(args.selector, args.tabId, conversationId);
      case "browser_screenshot":
        return this.runtime.screenshot(args.tabId, conversationId);
      case "browser_scroll":
        return this.runtime.scroll(args.deltaY ?? -240, conversationId);
      case "browser_console":
        return this.runtime.consoleMessages(
          { level: args.level, limit: args.limit }, args.tabId, conversationId);
      case "browser_network":
        return this.runtime.networkRequests(
          { failedOnly: args.failedOnly, limit: args.limit }, args.tabId, conversationId);
      case "browser_wait_for":
        return this.runtime.waitFor(String(args.expression ?? ""), args.timeoutMs, conversationId);
      default:
        throw new Error(`未提供该工具：${name}`);
    }
  }

  /** 起服务并返回接入信息。端口由系统分配，避免与本机其他服务相撞。 */
  async start() {
    if (this.endpoint) return { endpoint: this.endpoint, token: this.token };
    this.server = http.createServer((request, response) => {
      if (request.method !== "POST") {
        response.writeHead(405).end();
        return;
      }
      if (!this.authorized(request)) {
        response.writeHead(401).end();
        return;
      }
      let body = "";
      request.on("data", (chunk) => {
        body += chunk;
        // 请求体是 JSON-RPC 消息，正常不会大；设上限避免本机进程以大包拖垮服务
        if (body.length > 1_000_000) request.destroy();
      });
      request.on("end", () => {
        void (async () => {
          let message, origins;
          try {
            message = JSON.parse(body);
            origins = JSON.parse(String(request.headers["x-cilai-browser-origins"] ?? "[]"));
          } catch {
            response.writeHead(400).end();
            return;
          }
          // 通知没有编号，也不该有响应
          if (message.id === undefined) {
            response.writeHead(202).end();
            return;
          }
          const reply = await this.handle(message, conversationOf(request), origins);
          response.writeHead(200, { "content-type": "application/json" });
          response.end(JSON.stringify(reply));
        })();
      });
    });
    await new Promise((resolve, reject) => {
      this.server.once("error", reject);
      this.server.listen(0, "127.0.0.1", resolve);
    });
    const { port } = this.server.address();
    this.endpoint = `http://127.0.0.1:${port}/mcp`;
    return { endpoint: this.endpoint, token: this.token };
  }

  stop() {
    this.server?.close();
    this.server = null;
    this.endpoint = null;
  }
}

module.exports = { BrowserMcpServer, readTools, writeTools, CREDENTIAL_HINTS };
