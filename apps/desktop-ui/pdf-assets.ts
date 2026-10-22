import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import type { Plugin } from "vite";

export function pdfAssets(root: string): Plugin {
  const assets = new Map<string, Buffer>();
  for (const group of ["cmaps", "standard_fonts", "wasm"]) {
    const collect = (directory: string) => {
      for (const entry of readdirSync(directory, { withFileTypes: true })) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) { collect(path); continue; }
        if (!entry.isFile()) throw new Error(`PDF 资源不是普通文件：${path}`);
        if (group === "wasm" && !entry.name.endsWith("_nowasm_fallback.js")) continue;
        const name = `pdfjs/${group}/${entry.name}`;
        if (assets.has(name)) throw new Error(`PDF 资源名称重复：${name}`);
        assets.set(name, readFileSync(path));
      }
    };
    collect(join(root, group));
  }
  return {
    name: "cilai-pdf-assets",
    generateBundle() {
      for (const [fileName, source] of assets) this.emitFile({ type: "asset", fileName, source });
    },
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const path = request.url?.split("?")[0] ?? "";
        if (!path.startsWith("/pdfjs/")) { next(); return; }
        const bytes = assets.get(path.slice(1));
        if (!bytes) { response.statusCode = 404; response.end(); return; }
        if (request.method !== "GET" && request.method !== "HEAD") {
          response.statusCode = 405; response.setHeader("Allow", "GET, HEAD"); response.end(); return;
        }
        response.setHeader("Content-Type", path.endsWith(".js") ? "text/javascript; charset=utf-8" : "application/octet-stream");
        response.setHeader("Content-Length", bytes.length);
        response.setHeader("X-Content-Type-Options", "nosniff");
        response.end(request.method === "HEAD" ? undefined : bytes);
      });
    },
  };
}
