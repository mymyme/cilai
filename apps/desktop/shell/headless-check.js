const { readdirSync, statSync, readFileSync } = require("node:fs");
const path = require("node:path");
const HEADLESS_SCOPE_MARKER = '启动自检范围：个人资料与本地启动能力';
const AUTHENTICATED_CHECK_MARKER = '真实模型与工具验收：未执行';

function runtimeStartupReady(status) {
  return status?.phase === "running" && !status.runtimeInitializationError;
}

function localProfileReady(profile) {
  return profile?.ready === true && profile.schemaVersion === 2
    && typeof profile.profileId === "string" && /^[a-f0-9]{32}$/.test(profile.profileId);
}

function pdfSelfCheckMode(platform) {
  if (platform === "darwin") return "native-renderer";
  if (platform === "win32") return "pdfjs";
  throw new Error(`PDF 启动自检不支持当前平台：${platform}`);
}

function runtimePdfJsAssets(frontendRoot) {
  const assetsRoot = path.join(frontendRoot, "assets");
  const workerFiles = readdirSync(assetsRoot).filter((name) =>
    /^pdf\.worker\.min-[A-Za-z0-9_-]+\.mjs$/.test(name)
      && statSync(path.join(assetsRoot, name)).size > 0);
  if (workerFiles.length !== 1) {
    throw new Error(`PDF.js Worker 随包数量无效：${workerFiles.length}`);
  }

  const countFiles = (relativeDirectory, include = () => true) => {
    const directory = path.join(frontendRoot, relativeDirectory);
    return readdirSync(directory).filter((name) =>
      include(name) && statSync(path.join(directory, name)).isFile()
        && statSync(path.join(directory, name)).size > 0).length;
  };
  const cMapCount = countFiles(path.join("pdfjs", "cmaps"));
  const fontCount = countFiles(path.join("pdfjs", "standard_fonts"));
  const decoderCount = countFiles(
    path.join("pdfjs", "wasm"),
    (name) => name.endsWith("_nowasm_fallback.js"),
  );
  if (cMapCount === 0 || fontCount === 0 || decoderCount !== 2) {
    throw new Error(
      `PDF.js 随包资产不完整：CMap ${cMapCount}，字体 ${fontCount}，解码模块 ${decoderCount}`,
    );
  }
  return { workerCount: 1, cMapCount, fontCount, decoderCount };
}

function assertHeadlessCheckCompleted(output) {
  const lines = output.split(/\r?\n/);
  if (lines.filter(line => line === "启动自检：通过").length !== 1 || lines.includes("启动自检：未通过")) {
    throw new Error("启动自检没有完整通过标记，不能确认构建成功");
  }
  if (lines.filter(line => line === HEADLESS_SCOPE_MARKER).length !== 1
    || lines.filter(line => line === AUTHENTICATED_CHECK_MARKER).length !== 1
    || lines.includes('真实模型与工具验收：通过')) {
    throw new Error('启动自检没有完整个人资料隔离范围标记，不能确认验收范围');
  }
}

module.exports = { pdfSelfCheckMode, runtimePdfJsAssets, runtimeStartupReady, localProfileReady, assertHeadlessCheckCompleted,
  HEADLESS_SCOPE_MARKER, AUTHENTICATED_CHECK_MARKER };

if (require.main === module) {
  if (process.argv[2] !== "--validate-log" || !process.argv[3]) throw new Error("缺少启动自检日志");
  assertHeadlessCheckCompleted(readFileSync(process.argv[3], "utf8"));
}
