const { readdirSync } = require("node:fs");
const path = require("node:path");

async function renderPdfProbe(contents, frontendDirectory, bytes) {
  // 调用随包生产分块中的同一渲染器，覆盖真实 Worker、字体资源和 canvas，不能以 mock 替代。
  const chunks = readdirSync(path.join(frontendDirectory, "assets"))
    .filter((name) => /^pdfjs-renderer-[\w-]+\.js$/.test(name));
  if (chunks.length !== 1) throw new Error("Windows PDF 生产分块缺失或不唯一");
  return contents.executeJavaScript(`(async () => {
    const { openPdfDocument } = await import(${JSON.stringify(`/assets/${chunks[0]}`)});
    const bytes = Uint8Array.from(atob(${JSON.stringify(bytes.toString("base64"))}), (value) => value.charCodeAt(0));
    const task = openPdfDocument(bytes.buffer);
    try {
      const document = await task.promise;
      const canvas = window.document.createElement("canvas");
      await document.renderPage(canvas, 1, 640).promise;
      const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
      let inkPixels = 0;
      for (let offset = 0; offset < pixels.length; offset += 4) {
        if (pixels[offset + 3] && (pixels[offset] < 240 || pixels[offset + 1] < 240 || pixels[offset + 2] < 240)) inkPixels++;
      }
      return { pageCount: document.pageCount, width: canvas.width, height: canvas.height, inkPixels };
    } finally {
      await task.destroy();
    }
  })()`);
}

module.exports = { renderPdfProbe };
