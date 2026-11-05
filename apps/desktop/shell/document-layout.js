const path = require('node:path');
const fs = require('node:fs/promises');

const SCHEME = 'cilai-document-layout';
const PAGE = `${SCHEME}://preview/document-layout.html`;
const POLICY = "default-src 'none'; script-src 'self'; connect-src 'none'; img-src data: blob:; font-src data: blob:; style-src 'unsafe-inline'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
const MAX_BYTES = 32 * 1024 * 1024;
const MAX_MEMORY = 768 * 1024;

function assetPath(url) {
  const parsed = new URL(url);
  if (parsed.protocol !== `${SCHEME}:` || parsed.host !== 'preview' || parsed.search || parsed.hash || parsed.username || parsed.password) return null;
  const file = parsed.pathname.slice(1);
  return file === 'document-layout.html' || /^assets\/[a-zA-Z0-9_.-]+\.js$/.test(file) ? file : null;
}

function createDocumentLayout({ contentRoot, sessionOf, createView, metrics, getOwner, ipc }) {
  let sessionReady;
  let active;
  let stopping = false;
  const live = new Set();
  function getSession() {
    if (!sessionReady) sessionReady = Promise.resolve().then(() => {
      const session = sessionOf('cilai-document-layout');
      session.setPermissionRequestHandler((_contents, _permission, reply) => reply(false));
      session.setPermissionCheckHandler(() => false);
      session.on('will-download', event => event.preventDefault());
      session.webRequest.onBeforeRequest({ urls: ['<all_urls>'] }, (request, reply) => {
        let allowed = false;
        try { allowed = assetPath(request.url) !== null; } catch {}
        reply({ cancel: !allowed });
      });
      session.protocol.handle(SCHEME, async request => {
        const file = assetPath(request.url);
        if (!file) return new Response('forbidden', { status: 403 });
        try {
          return new Response(await fs.readFile(path.join(contentRoot, file)), { headers: {
            'Content-Type': file.endsWith('.html') ? 'text/html; charset=utf-8' : 'text/javascript; charset=utf-8',
            'Content-Security-Policy': POLICY, 'X-Content-Type-Options': 'nosniff', 'Cache-Control': 'no-store',
          }});
        } catch { return new Response('missing', { status: 404 }); }
      });
      return session;
    });
    return sessionReady;
  }
  function mainOwner(event) {
    const owner = getOwner();
    if (!owner || owner !== event.sender || event.senderFrame !== owner.mainFrame || owner.getURL() !== 'app://localhost/') {
      throw new Error('preview_layout_owner_invalid');
    }
    return owner;
  }
  function finish(job, error, html) {
    if (active !== job) return;
    active = null;
    job.input = null;
    clearTimeout(job.timer);
    clearInterval(job.memoryTimer);
    job.owner.removeListener('destroyed', job.closedOwner);
    const contents = job.contents;
    if (error) {
      if (contents && !contents.isDestroyed()) contents.close({ waitForBeforeUnload: false });
      job.reject(new Error(error));
      return;
    }
    if (!contents || contents.isDestroyed()) { job.reject(new Error('preview_layout_process_gone')); return; }
    const exitTimer = setTimeout(() => job.reject(new Error('preview_layout_close_unconfirmed')), 3_000);
    contents.once('destroyed', () => { clearTimeout(exitTimer); job.resolve({ html }); });
    contents.close({ waitForBeforeUnload: false });
  }
  function start(event, { id, format, input } = {}) {
    const owner = mainOwner(event);
    if (stopping) throw new Error('preview_layout_closed');
    if (typeof id !== 'string' || !/^[a-f0-9-]{36}$/.test(id) || !['docx', 'html'].includes(format)
      || (format === 'html' ? typeof input !== 'string' : !(input instanceof ArrayBuffer))) throw new Error('preview_layout_invalid');
    const bytes = format === 'html' ? Buffer.byteLength(input) : input.byteLength;
    if (bytes > MAX_BYTES) throw new Error('preview_file_too_large');
    if (active || live.size) throw new Error('preview_layout_capacity');
    return new Promise((resolve, reject) => {
      const job = { id, format, input, owner, resolve, reject, contents: null, described: false };
      job.closedOwner = () => finish(job, 'preview_layout_cancelled');
      owner.once('destroyed', job.closedOwner);
      job.timer = setTimeout(() => finish(job, 'preview_layout_timeout'), 30_000);
      active = job;
      void getSession().then(session => {
        if (active !== job) return;
        const view = createView({
          session, sandbox: true, contextIsolation: true, nodeIntegration: false, webSecurity: true,
          preload: path.join(__dirname, 'document-layout-preload.js'),
        });
        const contents = view.webContents;
        job.contents = contents;
        live.add(contents);
        contents.setWindowOpenHandler(() => ({ action: 'deny' }));
        contents.on('will-navigate', event => event.preventDefault());
        contents.on('will-redirect', event => event.preventDefault());
        contents.on('will-frame-navigate', event => { if (!event.isMainFrame || event.url !== PAGE) event.preventDefault(); });
        contents.once('destroyed', () => { live.delete(contents); finish(job, 'preview_layout_process_gone'); });
        contents.once('render-process-gone', () => finish(job, 'preview_layout_process_gone'));
        job.memoryTimer = setInterval(() => {
          if (active !== job || contents.isDestroyed()) return;
          const pid = contents.getOSProcessId();
          const usage = pid > 0 ? metrics().find(metric => metric.pid === pid) : null;
          if (usage?.memory?.workingSetSize > MAX_MEMORY) finish(job, 'preview_layout_memory_limit');
        }, 500);
        void contents.loadURL(PAGE).catch(() => finish(job, 'preview_layout_assets_unavailable'));
      }).catch(() => finish(job, 'preview_layout_assets_unavailable'));
    });
  }
  function worker(event) {
    if (!active || event.sender !== active.contents || event.senderFrame !== event.sender.mainFrame || event.sender.getURL() !== PAGE) {
      throw new Error('preview_layout_worker_invalid');
    }
    return active;
  }
  ipc.handle('document-layout:take', event => {
    const job = worker(event);
    if (job.described) throw new Error('preview_layout_worker_invalid');
    const input = job.input;
    job.input = null;
    job.described = true;
    return { format: job.format, input };
  });
  ipc.handle('document-layout:complete', (event, html) => {
    const job = worker(event);
    if (!job.described || typeof html !== 'string') return finish(job, 'preview_layout_invalid');
    if (Buffer.byteLength(html) > MAX_BYTES * 2) return finish(job, 'preview_layout_output_limit');
    finish(job, null, html);
  });
  ipc.handle('document-layout:failed', event => finish(worker(event), 'preview_layout_failed'));
  return {
    handles: command => command === 'document_layout_start' || command === 'document_layout_cancel',
    invoke: (event, { command, args }) => {
      if (command === 'document_layout_start') return start(event, args);
      const owner = mainOwner(event);
      if (active?.owner !== owner || active.id !== args?.id) return false;
      finish(active, 'preview_layout_cancelled');
      return true;
    },
    stop: () => { stopping = true; if (active) finish(active, 'preview_layout_cancelled'); },
  };
}

module.exports = { createDocumentLayout, SCHEME };
