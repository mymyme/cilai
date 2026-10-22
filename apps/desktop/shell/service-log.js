const fs = require("node:fs");
const path = require("node:path");

const DEFAULT_MAX_BYTES = 5 * 1024 * 1024;

/**
 * 业务服务标准错误输出的落盘记录。
 *
 * 发送链路各阶段耗时、保存耗时和启动失败原因都只从标准错误输出给出；只进控制台时，
 * 打包后的应用里事后无从查起。按行加时间戳写入应用日志目录，超过上限时把当前文件
 * 轮换为 `.1` 再重新开始，只保留一份旧文件。
 */
function createServiceLog(filePath, maxBytes = DEFAULT_MAX_BYTES) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  const currentSize = () => {
    try {
      const metadata = fs.lstatSync(filePath);
      if (!metadata.isFile() || metadata.isSymbolicLink() || !Number.isSafeInteger(metadata.size)) {
        throw new Error("personal_namespace_log_file_invalid");
      }
      return metadata.size;
    } catch (error) {
      if (error.code === "ENOENT") return 0;
      throw error;
    }
  };
  let size = currentSize();
  let pending = "";
  let suspension = null;
  const rotate = () => {
    fs.renameSync(filePath, `${filePath}.1`);
    size = 0;
  };
  return {
    filePath,
    suspendForStorage() {
      if (suspension) throw new Error("personal_namespace_log_already_paused");
      const token = {};
      suspension = token;
      let resumed = false;
      const check = () => {
        if (resumed || suspension !== token) throw new Error("personal_namespace_log_pause_expired");
      };
      return {
        sealPending() {
          check();
          if (!pending.trim()) return;
          const text = `${new Date().toISOString()} ${pending.trimEnd()}\n`;
          const bytes = Buffer.byteLength(text);
          if (size > 0 && size + bytes > maxBytes) rotate();
          fs.appendFileSync(filePath, text, { mode: 0o600 });
          size += bytes;
          pending = "";
        },
        rebase() {
          check();
          size = currentSize();
          pending = "";
        },
        resume() {
          check();
          suspension = null;
          resumed = true;
        },
      };
    },
    write(chunk) {
      if (suspension) return false;
      pending += String(chunk);
      const lines = pending.split("\n");
      pending = lines.pop() ?? "";
      const text = lines
        .filter((line) => line.trim())
        .map((line) => `${new Date().toISOString()} ${line.trimEnd()}\n`)
        .join("");
      if (!text) return false;
      const bytes = Buffer.byteLength(text);
      if (size > 0 && size + bytes > maxBytes) rotate();
      fs.appendFileSync(filePath, text, { mode: 0o600 });
      size += bytes;
      return true;
    },
  };
}

module.exports = { createServiceLog };
