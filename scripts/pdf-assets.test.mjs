import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { pdfAssets } from "../apps/desktop-ui/pdf-assets.ts";

test("PDF 资源以固定本地路径完整输出，只包含无 WASM 解码器", () => {
  const root = mkdtempSync(join(tmpdir(), "cilai-pdf-assets-"));
  try {
    for (const group of ["cmaps", "standard_fonts", "wasm"]) mkdirSync(join(root, group));
    writeFileSync(join(root, "cmaps", "test.bcmap"), "cmap");
    writeFileSync(join(root, "standard_fonts", "test.pfb"), "font");
    writeFileSync(join(root, "wasm", "decoder_nowasm_fallback.js"), "decoder");
    writeFileSync(join(root, "wasm", "decoder.wasm"), "excluded");
    const plugin = pdfAssets(root);
    const emitted = [];
    plugin.generateBundle.call({ emitFile: (value) => emitted.push(value) });
    assert.deepEqual(emitted.map((value) => value.fileName).sort(), ["pdfjs/cmaps/test.bcmap", "pdfjs/standard_fonts/test.pfb", "pdfjs/wasm/decoder_nowasm_fallback.js"]);
    assert.equal(emitted[0].source.toString(), "cmap");
    let middleware;
    plugin.configureServer({ middlewares: { use: (handler) => { middleware = handler; } } });
    function request(url, method = "GET") {
      const response = { statusCode: 200, headers: {}, setHeader(name, value) { this.headers[name] = value; }, end(bytes) { this.bytes = bytes; } };
      middleware({ url, method }, response, () => { response.next = true; });
      return response;
    }
    assert.equal(request("/pdfjs/cmaps/test.bcmap?version=1").bytes.toString(), "cmap");
    assert.equal(request("/pdfjs/cmaps/test.bcmap", "HEAD").bytes, undefined);
    assert.equal(request("/pdfjs/cmaps/test.bcmap", "POST").statusCode, 405);
    assert.equal(request("/pdfjs/../../package.json").statusCode, 404);
    assert.equal(request("/pdfjs/cmaps/%2e%2e/package.json").statusCode, 404);
    assert.equal(request("/pdfjs/wasm/decoder.wasm").statusCode, 404);
    assert.equal(request("/src/main.tsx").next, true);
    assert.equal(request("/pdfjs/wasm/decoder_nowasm_fallback.js").headers["Content-Type"], "text/javascript; charset=utf-8");
  } finally { rmSync(root, { recursive: true }); }
});
