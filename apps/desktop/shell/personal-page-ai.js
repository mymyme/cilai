const { randomUUID } = require("node:crypto");
const { actionTimeoutMs: ACTION_TIMEOUT_MS } = require("./personal-page-ai-timing.json");

const METHODS = new Set(["session.read", "context.publish", "context.revoke", "uiAction.register", "model.session.open", "model.session.read", "model.run.create", "model.run.get", "model.run.cancel", "conversation.handoff", "audit.list"]);
const PUBLIC_COMMANDS = new Set(["personal_page_ai_dispatch", "personal_page_ai_resolve"]);
const INVALID = "personal_page_request_invalid";
function object(value, keys) {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).some(key => !keys.includes(key))) throw new Error(INVALID);
}
function bounded(value) {
  if (Buffer.byteLength(JSON.stringify(value), "utf8") > 256 * 1024) throw new Error("personal_page_request_too_large");
}
function closedReceipt(value) {
  if (value?.retired !== true || value?.processesExited !== true || typeof value.cleanupPending !== "boolean" || typeof value.auditPending !== "boolean") throw new Error("personal_page_close_unconfirmed");
  return value;
}

function createPersonalPageAi({ getOwner, documents, service, actionTimeoutMs = ACTION_TIMEOUT_MS }) {
  const entries = new Map();
  let closing = false;
  let lifecycleError = null;
  const actualFrame = url => {
    const owner = getOwner();
    if (!owner || owner.isDestroyed()) throw new Error("personal_page_owner_unavailable");
    const matches = owner.mainFrame.frames.filter(frame => !frame.detached && frame.url === url);
    if (matches.length !== 1) throw new Error("personal_page_frame_unconfirmed");
    return { owner, frame: matches[0] };
  };
  const verify = entry => {
    if (entry.retired || !documents.binding(entry.url)) throw new Error("personal_page_document_closed");
    const { owner, frame } = actualFrame(entry.url);
    if (owner !== entry.owner || frame !== entry.frame || frame.processId !== entry.processId || frame.routingId !== entry.routingId) {
      throw new Error("personal_page_frame_changed");
    }
    return frame;
  };
  const retire = entry => {
    entry.retired = true;
    if (!entry.closed) entry.closed = (async () => {
      try { await entry.registration; } catch { /* 登记失败仍向运行时核对关闭。 */ }
      closedReceipt(await service.call("personal_page_ai_close_internal", { documentId: entry.documentId }));
    })();
    entry.closed.catch(() => { lifecycleError = "personal_page_close_unconfirmed"; });
    return entry.closed;
  };
  const find = async url => {
    if (closing || lifecycleError) throw new Error(lifecycleError ?? "personal_page_maintenance_active");
    if (typeof url !== "string") throw new Error(INVALID);
    let entry = entries.get(url);
    if (!entry) {
      const binding = documents.binding(url);
      if (!binding) throw new Error("personal_page_binding_missing");
      const { owner, frame } = actualFrame(url);
      entry = { url, owner, frame, frameTreeNodeId: frame.frameTreeNodeId, processId: frame.processId, routingId: frame.routingId, documentId: `document_${randomUUID().replaceAll("-", "")}`, retired: false, notificationRuns: new Map() };
      entries.set(url, entry);
      entry.registration = service.call("personal_page_ai_register_internal", { documentId: entry.documentId, binding });
      entry.registration.catch(() => { entry.retired = true; });
    }
    await entry.registration;
    verify(entry);
    return entry;
  };
  const sweep = setInterval(() => {
    for (const entry of entries.values()) {
      if (entry.retired) continue;
      try { verify(entry); } catch { void retire(entry); }
    }
  }, 250);
  sweep.unref?.();

  return {
    handles: command => PUBLIC_COMMANDS.has(command),
    async invoke(event, { command, args }) {
      documents.verify(event);
      if (command === "personal_page_ai_dispatch") {
        object(args, ["url", "request"]);
        object(args.request, ["method", "requestId", "params"]);
        if (!METHODS.has(args.request.method)) throw new Error("personal_page_method_not_allowed");
        if (typeof args.request.requestId !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(args.request.requestId)) throw new Error(INVALID);
        bounded(args.request);
        const entry = await find(args.url);
        const result = await service.call("personal_page_ai_dispatch_internal", { documentId: entry.documentId, request: args.request });
        if (args.request.method === "model.run.create" && typeof result?.runId === "string" && typeof result?.applicationId === "string" && typeof args.request.params?.modelSessionId === "string") {
          entry.notificationRuns.set(result.runId, { sessionId: args.request.params.modelSessionId, applicationId: result.applicationId });
        }
        return result;
      }
      if (command === "personal_page_ai_resolve") {
        object(args, ["url", "runId", "confirmationId", "decision", "arguments"]);
        if (typeof args.runId !== "string" || typeof args.confirmationId !== "string" || !["approved", "rejected"].includes(args.decision)) throw new Error(INVALID);
        bounded(args);
        const entry = await find(args.url);
        return service.call("personal_page_ai_resolve_internal", { documentId: entry.documentId, runId: args.runId, confirmationId: args.confirmationId, decision: args.decision, ...(args.arguments === undefined ? {} : { arguments: args.arguments }) });
      }
      throw new Error("personal_page_method_not_allowed");
    },
    async notificationLocation(target) {
      if (closing || lifecycleError || typeof target?.runId !== "string") throw new Error("personal_notification_target_missing");
      const matches = [...entries.values()].filter(entry => {
        const run = entry.notificationRuns.get(target.runId);
        return !entry.retired && run?.sessionId === target.sessionId && run?.applicationId === target.applicationId;
      });
      if (matches.length !== 1) throw new Error("personal_notification_target_missing");
      const entry = matches[0]; verify(entry);
      const run = await service.call("personal_page_ai_dispatch_internal", { documentId: entry.documentId, request: { method: "model.run.get", requestId: `notification_${randomUUID().replaceAll("-", "")}`, params: { runId: target.runId } } });
      if (run?.runId !== target.runId || run?.applicationId !== target.applicationId) throw new Error("personal_notification_target_missing");
      verify(entry);
      const binding = documents.binding(entry.url);
      if (!binding) throw new Error("personal_notification_target_missing");
      return { creationProjectId: binding.creationProjectId, documentUrl: entry.url };
    },
    async action(args) {
      object(args, ["documentId", "actionId", "arguments"]);
      const entry = [...entries.values()].find(item => item.documentId === args.documentId);
      if (!entry || typeof args.actionId !== "string") throw new Error("personal_page_document_closed");
      bounded(args);
      const frame = verify(entry);
      const data = JSON.stringify({ actionId: args.actionId, arguments: args.arguments });
      let timer;
      try {
        const result = await Promise.race([
          frame.executeJavaScript(`window.__cilaiPersonalAiExecute(${data})`),
          new Promise((_, reject) => { timer = setTimeout(() => reject(new Error("personal_page_action_outcome_unconfirmed")), actionTimeoutMs); }),
        ]);
        verify(entry);
        bounded(result);
        return result;
      } catch {
        void retire(entry);
        throw new Error("personal_page_action_outcome_unconfirmed");
      } finally { clearTimeout(timer); }
    },
    async release(url) {
      const entry = entries.get(url);
      if (entry) return retire(entry);
    },
    forget(url) { if (!documents.binding(url)) entries.delete(url); },
    invalidate() { for (const entry of entries.values()) void retire(entry); },
    navigation(owner, event) {
      for (const entry of entries.values()) {
        if (!entry.retired && entry.owner === owner && event.frame?.frameTreeNodeId === entry.frameTreeNodeId) void retire(entry);
      }
    },
    async revokeAll() {
      if (closing) throw new Error("personal_page_close_busy");
      closing = true;
      try {
        for (const entry of entries.values()) entry.retired = true;
        await Promise.allSettled([...entries.values()].map(entry => entry.registration));
        const receipt = closedReceipt(await service.call("personal_page_ai_revoke_all_internal", {}));
        lifecycleError = null;
        return receipt;
      } catch (error) {
        lifecycleError = "personal_page_close_unconfirmed";
        throw error;
      } finally { closing = false; }
    },
    stop() { clearInterval(sweep); },
  };
}

module.exports = { createPersonalPageAi, METHODS };
