const fs = require("node:fs");
const path = require("node:path");
const { randomUUID, createHash } = require("node:crypto");
const { formatErrorReport } = require("./diagnostic-report");

function createDiagnosticExport({ diagnostics, collect, dialog, now = Date.now }) {
  const previews = new Map();
  const expire = () => {
    for (const [owner, preview] of previews) if (preview.expiresAt <= now()) previews.delete(owner);
  };
  return {
    async preview(owner) {
      expire();
      if (!diagnostics()) throw new Error("诊断日志无法初始化，请检查应用数据目录权限。");
      let timer;
      const runtime = await Promise.race([
        Promise.resolve().then(collect).catch(() => ({ status: "unavailable" })),
        new Promise(resolve => { timer = setTimeout(() => resolve({ status: "timedOut" }), 5000); }),
      ]).finally(() => clearTimeout(timer));
      const text = formatErrorReport(JSON.parse(diagnostics().exportText(runtime)));
      const bytes = Buffer.byteLength(text, "utf8");
      if (bytes > 2 * 1024 * 1024) throw new Error("诊断内容超过导出上限，请缩小记录范围。");
      const preview = { previewId: randomUUID(), digest: createHash("sha256").update(text).digest("hex"),
        expiresAt: now() + 10 * 60 * 1000, bytes, text };
      if (previews.size >= 16 && !previews.has(owner)) throw new Error("诊断预览过多，请关闭其他窗口后重试。");
      previews.set(owner, preview);
      return { ...preview };
    },
    async save(owner, window, args) {
      expire();
      const preview = previews.get(owner);
      if (!args || Object.keys(args).some(key => !["previewId", "digest"].includes(key))
        || !preview || args.previewId !== preview.previewId || args.digest !== preview.digest) {
        throw new Error("诊断预览已失效，请重新预览。");
      }
      const result = await dialog.showSaveDialog(window, { title: "导出诊断", defaultPath: `词来个人工作台诊断-${new Date(now()).toISOString().slice(0, 10)}.txt`,
        filters: [{ name: "诊断日志", extensions: ["txt"] }] });
      if (result.canceled || !result.filePath) return false;
      if (preview.expiresAt <= now()) throw new Error("诊断预览已失效，请重新预览。");
      const temporary = path.join(path.dirname(result.filePath), `.cilai-diagnostic-${randomUUID()}.tmp`);
      let created = false;
      try {
        const file = fs.openSync(temporary, "wx", 0o600);
        created = true;
        try { fs.writeFileSync(file, preview.text, "utf8"); fs.fsyncSync(file); }
        finally { fs.closeSync(file); }
        fs.renameSync(temporary, result.filePath);
        created = false;
      } catch {
        throw new Error("诊断文件未能保存，请检查目标位置后重新选择。");
      } finally {
        if (created) fs.unlinkSync(temporary);
      }
      return true;
    },
  };
}

module.exports = { createDiagnosticExport };
