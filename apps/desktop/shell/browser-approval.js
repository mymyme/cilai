const crypto = require("node:crypto");

/**
 * 浏览器写操作的审批。
 *
 * 点击与输入会在外部站点留下真实痕迹——提交表单、下单、发消息都由它们构成，因此交由
 * 员工确认。确认信息必须包含目标站点与关键参数：只说"AI 想点击"而不说在哪、点什么，
 * 员工无从判断。
 *
 * 粒度按站点，而不是逐次。一个流程往往由十几次点击和输入构成，逐次弹窗会让员工在
 * 疲劳中一路点"允许"，那样的审批只是形式。员工可以选择"本站点本次会话都允许"，授权
 * 限于这一个站点、这一次对话、本进程存续期间，不落盘、不跨对话、不跨重启。
 *
 * 但凭据类操作不在授权范围内：密码、验证码、支付信息、证件号一律每次单问，无论此前
 * 授权过什么。这类字段的风险不随"这个站点我信任"而降低。
 *
 * 无人应答时按拒绝处理。默认放行等于把审批变成摆设，而超时本身通常意味着没有人在看。
 */

/** 员工要在这个时限内作出决定，超时按拒绝。 */
const DECISION_TIMEOUT_MS = 120_000;

class BrowserApprovals {
  constructor(notify) {
    this.notify = notify;
    this.pending = new Map();
    /** 已授权的「对话+站点」。值是授权时间，仅用于说明来由。 */
    this.grants = new Map();
  }

  /** 授权只在同一对话、同一站点内成立，两者缺一就不算数。 */
  static grantKey(scope, site) {
    return `${scope ?? ""}|${site ?? ""}`;
  }

  /** 这次操作是否已被此前的站点授权覆盖。凭据类永远不被覆盖。 */
  grantedFor(request) {
    if (request.alwaysAsk) return false;
    if (!request.site || request.site === "未打开页面") return false;
    return this.grants.has(BrowserApprovals.grantKey(request.scope, request.site));
  }

  /** 收回某个对话的全部授权。员工接管、对话结束或站点变化时用得上。 */
  revokeScope(scope) {
    const prefix = `${scope ?? ""}|`;
    for (const key of [...this.grants.keys()]) {
      if (key.startsWith(prefix)) this.grants.delete(key);
    }
  }

  /**
   * 发起一次审批并等待结果。
   *
   * @param {{tool: string, site: string, summary: string, arguments: object}} request
   */
  request(request) {
    // 已授权的站点直接放行，并说明是凭哪次授权放行的——不能让操作看起来像没经过审批
    if (this.grantedFor(request)) {
      return Promise.resolve({ approved: true, viaGrant: true, site: request.site });
    }
    const confirmationId = crypto.randomUUID();
    const payload = {
      confirmationId,
      tool: request.tool,
      site: request.site,
      summary: request.summary,
      arguments: request.arguments ?? {},
      // 写操作一律按高风险呈现：它们的共同点是会在外部留下痕迹，且难以撤回
      riskLevel: "high",
      /*
       * 能不能给"本站点都允许"这个选项，由发起方说了算：凭据类操作不给，
       * 站点取不到时也不给——授权的范围必须是员工看得见的那个站点。
       */
      allowSiteGrant: !request.alwaysAsk
        && Boolean(request.site) && request.site !== "未打开页面",
      requestedAt: Date.now(),
    };
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.pending.delete(confirmationId);
        this.notify("browser-approval-closed", { confirmationId, decision: "timeout" });
        resolve({ approved: false, reason: "员工未在限期内确认，操作已按拒绝处理" });
      }, DECISION_TIMEOUT_MS);
      this.pending.set(confirmationId, { resolve, timer, payload, scope: request.scope ?? null });
      this.notify("browser-approval-request", payload);
    });
  }

  /** 员工的决定。未知确认号一律忽略——它要么已超时，要么根本不存在。 */
  /**
   * 员工的决定。未知确认号一律忽略——它要么已超时，要么根本不存在。
   *
   * @param {boolean} approved 是否允许
   * @param {boolean} grantSite 是否同时授权本站点在本次对话内都允许
   */
  resolve(confirmationId, approved, grantSite = false) {
    const entry = this.pending.get(confirmationId);
    if (!entry) return { accepted: false };
    clearTimeout(entry.timer);
    this.pending.delete(confirmationId);
    // 只有本来就允许给授权选项的请求才能建立授权，界面传什么都不能绕过这一条
    const granted = approved && grantSite && entry.payload.allowSiteGrant;
    if (granted) {
      this.grants.set(
        BrowserApprovals.grantKey(entry.scope, entry.payload.site), Date.now());
    }
    this.notify("browser-approval-closed", {
      confirmationId,
      decision: approved ? "approved" : "rejected",
      grantedSite: granted ? entry.payload.site : null,
    });
    entry.resolve(
      approved
        ? { approved: true, grantedSite: granted ? entry.payload.site : null }
        : { approved: false, reason: "员工拒绝了本次操作" },
    );
    return { accepted: true, grantedSite: granted ? entry.payload.site : null };
  }

  /** 进程收尾时把在途审批一并拒绝，避免调用方悬着。 */
  rejectAll(reason) {
    for (const [, entry] of this.pending) {
      clearTimeout(entry.timer);
      entry.resolve({ approved: false, reason });
    }
    this.pending.clear();
  }
}

module.exports = { BrowserApprovals, DECISION_TIMEOUT_MS };
