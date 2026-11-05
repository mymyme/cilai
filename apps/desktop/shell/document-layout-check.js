const wordFixture = "UEsDBBQAAAAIAE9dZV33S4B1wgAAAHYBAAATAAAAW0NvbnRlbnRfVHlwZXNdLnhtbH2QuQ7CMAyGX6XKiqgRAwOiLMAKDLyAlbptRC7FLsfbk3INCBjt//gsLw7XSFxcnPVcqU4kzgFYd+SQyxDJZ6UJyaHkMbUQUR+xJZhOJjPQwQt5GcvQoZaLNTXYWyk2l7xmE3ylEllWxephHFiVwhit0ShZh5OvPyjjJ6HMybuHOxN5lA0KvhIG5TfgmdudKCVTU7HHJFt02QXnkGqog+5dTpb/a77cGZrGaHrnh7aYgiZm41tny7fi0PjX/XB/9/IGUEsDBBQAAAAIAE9dZV1hey9DiQAAAPIAAAALAAAAX3JlbHMvLnJlbHONzzsOAiEQBuCrEA6ws1pYGKCy2dZ4AQLDIy6PDBj19lJYrMbCcuaffH9GnHHVPZbcQqyNPdKam+Sh93oEaCZg0m0qFfNIXKGk+xjJQ9Xmqj3Cfp4PQFuDK7E12WIlp8XuOLs8K/5jF+eiwVMxt4S5/6j4uhiyJo9d8nshC/a9ngbLQQn4eFG9AFBLAwQUAAAACABPXWVdN3u7GocAAACkAAAAEQAAAHdvcmQvZG9jdW1lbnQueG1ssym3SslPLs1NzStRqMjNySu2KrdVyigpKbDS1y9OzkjNTSzWyy9IzQPKpeUX5SaWALlF6frl+UUpBUX5yanFxZl56bk5+kYGBmb6uYmZeUp2NuVWSfkplSC6AEQUgYgSu+fda56v7n7WN+l5Z8ezvkUvJ3XY6IPEQWQRmCwAkxC9+gh32QEAUEsBAhQDFAAAAAgAT11lXfdLgHXCAAAAdgEAABMAAAAAAAAAAAAAAIABAAAAAFtDb250ZW50X1R5cGVzXS54bWxQSwECFAMUAAAACABPXWVdYXsvQ4kAAADyAAAACwAAAAAAAAAAAAAAgAHzAAAAX3JlbHMvLnJlbHNQSwECFAMUAAAACABPXWVdN3u7GocAAACkAAAAEQAAAAAAAAAAAAAAgAGlAQAAd29yZC9kb2N1bWVudC54bWxQSwUGAAAAAAMAAwC5AAAAWwIAAAAA";

async function checkDocumentLayout(contents) {
  return contents.executeJavaScript(`(async () => {
    const prepare = (format, input) => window.__CILAI_HOST__.invoke("document_layout_start", {
      id: crypto.randomUUID(), format, input,
    });
    const markup = '<html><head><meta http-equiv="refresh" content="0;url=https://blocked.invalid"></head><body><a href="#body">读取</a><div id="body" class="reveal">网页探针</div></body></html>';
    const html = (await prepare("html", markup)).html;
    const bytes = Uint8Array.from(atob(${JSON.stringify(wordFixture)}), c => c.charCodeAt(0));
    const word = (await prepare("docx", bytes.buffer)).html;
    if (!html.includes("网页探针") || !html.includes("about:srcdoc#body")
      || !html.includes("Content-Security-Policy") || html.includes("blocked.invalid")
      || !word.includes("独立排版探针") || !word.includes("Content-Security-Policy")) throw new Error("文件排版探针失败");
    return { html: true, word: true };
  })()`);
}

module.exports = { checkDocumentLayout };
