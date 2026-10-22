async function checkPersonalDocument(contents) {
  return contents.executeJavaScript(`(async () => {
    const api = window.__CILAI_HOST__;
    const channel = crypto.randomUUID();
    const html = '<p id="count">0</p><button id="add">加一</button><button id="reset">清零</button><script>(' + ((channel) => {
      let count = 0;
      const display = document.getElementById('count');
      document.getElementById('add').onclick = () => { display.textContent = String(++count); };
      document.getElementById('reset').onclick = () => { count = 0; display.textContent = '0'; };
      document.getElementById('add').click();
      document.getElementById('add').click();
      document.getElementById('add').click();
      const added = display.textContent === '3';
      document.getElementById('reset').click();
      const reset = display.textContent === '0';
      let parentBlocked = false;
      try { void parent.document.body; } catch { parentBlocked = true; }
      fetch('https://personal-document-probe.invalid/').then(
        () => parent.postMessage({ channel, networkBlocked: false }, '*'),
        () => parent.postMessage({ channel, networkBlocked: true, parentBlocked,
          hostAbsent: typeof window.__CILAI_HOST__ === 'undefined' && typeof require === 'undefined' && typeof window.cilaiPersonal === 'undefined',
          inlineRan: true, added, reset }, '*')
      );
    }).toString() + ')(' + JSON.stringify(channel) + ')</script>';
    const url = await api.invoke('personal_document_create', { html });
    const frame = document.createElement('iframe');
    frame.setAttribute('sandbox', 'allow-scripts');
    frame.hidden = true;
    frame.src = url;
    try {
      return await new Promise((resolve, reject) => {
        const done = () => { clearTimeout(timer); window.removeEventListener('message', receive); };
        const receive = event => {
          if (event.source !== frame.contentWindow || event.data?.channel !== channel) return;
          done(); resolve({ ...event.data, opaqueOrigin: event.origin === 'null' });
        };
        const timer = setTimeout(() => { done(); reject(new Error('应用页面脚本未响应')); }, 8000);
        window.addEventListener('message', receive);
        document.body.append(frame);
      });
    } finally {
      frame.remove();
      await api.invoke('personal_document_release', { url });
    }
  })()`);
}

module.exports = { checkPersonalDocument };
