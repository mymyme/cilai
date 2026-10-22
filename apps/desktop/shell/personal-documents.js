const { randomUUID } = require("node:crypto");

const ORIGIN = "app://personal-capability";
const POLICY = "default-src 'none'; connect-src 'none'; img-src data: blob:; media-src data: blob:; font-src data:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; object-src 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors app://localhost; sandbox allow-scripts";
const MAX_DOCUMENT_BYTES = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES = 16 * 1024 * 1024;
const MAX_DOCUMENTS = 32;

function createPersonalDocuments(getOwner) {
  const documents = new Map();
  const trackedFrames = new WeakMap();
  let totalBytes = 0;
  const frameIds = owner => {
    if (!trackedFrames.has(owner)) trackedFrames.set(owner, new Set());
    return trackedFrames.get(owner);
  };
  const personalOrigin = value => {
    try { const url = new URL(value); return url.protocol === "app:" && url.hostname === "personal-capability"; } catch { return false; }
  };
  const verify = (event) => {
    const owner = getOwner();
    if (!owner || owner.isDestroyed() || event.sender !== owner || event.senderFrame !== owner.mainFrame) {
      throw new Error("应用页面请求来源无效");
    }
    const url = new URL(event.senderFrame.url);
    if (url.protocol !== "app:" || url.host !== "localhost") throw new Error("应用页面请求来源无效");
  };
  return {
    handles: (command) => command === "personal_document_create" || command === "personal_document_release",
    invoke(event, { command, args }) {
      verify(event);
      if (command === "personal_document_create") {
        if (Object.keys(args ?? {}).some(key => !["html", "pageBinding"].includes(key))) throw new Error("应用页面参数无效");
        const binding = args?.pageBinding;
        if (binding !== undefined && (!binding || typeof binding !== "object" || Array.isArray(binding)
            || Object.keys(binding).sort().join(",") !== "creationProjectId,expectedPackageDigest"
            || typeof binding.creationProjectId !== "string" || !binding.creationProjectId.length || binding.creationProjectId.length > 200
            || typeof binding.expectedPackageDigest !== "string" || !/^sha256:[a-f0-9]{64}$/.test(binding.expectedPackageDigest))) {
          throw new Error("personal_page_binding_invalid");
        }
        if (typeof args?.html !== "string" || !args.html.length) throw new Error("应用页面内容无效");
        const bytes = Buffer.byteLength(args.html, "utf8");
        if (bytes > MAX_DOCUMENT_BYTES || totalBytes + bytes > MAX_TOTAL_BYTES || documents.size >= MAX_DOCUMENTS) {
          throw new Error("应用页面超过加载限制，请关闭其他应用后重试");
        }
        const url = `${ORIGIN}/${randomUUID()}`;
        documents.set(url, { html: args.html, bytes, pageBinding: binding ? Object.freeze({ ...binding }) : undefined });
        totalBytes += bytes;
        return url;
      }
      if (command === "personal_document_release") {
        const document = documents.get(args?.url);
        if (document) { totalBytes -= document.bytes; documents.delete(args.url); }
        return;
      }
      throw new Error("不支持的应用页面请求");
    },
    verify,
    binding(url) { return documents.get(url)?.pageBinding; },
    trackNavigation(owner, event) {
      if (owner !== getOwner() || !event.frame || event.isMainFrame) return;
      if (personalOrigin(event.frame.url) || documents.has(event.url)) frameIds(owner).add(event.frame.frameTreeNodeId);
    },
    frames(owner) {
      const ids = frameIds(owner);
      const frames = owner.mainFrame.framesInSubtree.filter(frame => !frame.detached);
      for (const frame of frames) if (personalOrigin(frame.url)) ids.add(frame.frameTreeNodeId);
      const existing = new Set(frames.map(frame => frame.frameTreeNodeId));
      for (const id of ids) if (!existing.has(id)) ids.delete(id);
      return frames.filter(frame => ids.has(frame.frameTreeNodeId));
    },
    response(request) {
      const document = request.method === "GET" ? documents.get(request.url) : undefined;
      return new Response(document?.html ?? "应用页面已关闭，请重新打开", {
        status: document ? 200 : 404,
        headers: { "Content-Type": "text/html; charset=utf-8", "Content-Security-Policy": POLICY,
          "Cache-Control": "no-store", "X-Content-Type-Options": "nosniff" },
      });
    },
    clear() { documents.clear(); totalBytes = 0; },
  };
}

module.exports = { createPersonalDocuments, ORIGIN, POLICY, MAX_DOCUMENT_BYTES, MAX_TOTAL_BYTES, MAX_DOCUMENTS };
