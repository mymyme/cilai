const test = require("node:test");
const assert = require("node:assert/strict");
const { mkdtempSync, mkdirSync, rmSync, writeFileSync } = require("node:fs");
const { tmpdir } = require("node:os");
const path = require("node:path");

const { pdfSelfCheckMode, runtimePdfJsAssets, runtimeStartupReady, localProfileReady, assertHeadlessCheckCompleted,
  HEADLESS_SCOPE_MARKER, AUTHENTICATED_CHECK_MARKER } = require("./headless-check");

test("启动自检完整标记同时支持两端换行", () => {
  for (const newline of ["\n", "\r\n"]) assert.doesNotThrow(() => assertHeadlessCheckCompleted([
    HEADLESS_SCOPE_MARKER, AUTHENTICATED_CHECK_MARKER, "启动自检：通过", "",
  ].join(newline)));
});

test('启动自检必须声明个人资料隔离范围，真实模型验收单独记录', () => {
  for (const lines of [
    ['启动自检：通过'],
    [HEADLESS_SCOPE_MARKER, '启动自检：通过'],
    [AUTHENTICATED_CHECK_MARKER, '启动自检：通过'],
    [HEADLESS_SCOPE_MARKER, AUTHENTICATED_CHECK_MARKER, '真实模型与工具验收：通过', '启动自检：通过'],
  ]) assert.throws(() => assertHeadlessCheckCompleted(lines.join('\n')), /隔离范围标记/);
});
test("缺少终态或相互矛盾的自检记录不能由成功退出码替代", () => {
  for (const output of ["界面已加载\n", "启动自检：未通过\n", "启动自检：通过\n启动自检：未通过\n", "启动自检：通过\n启动自检：通过\n"]) {
    assert.throws(() => assertHeadlessCheckCompleted(output), /没有完整通过标记/);
  }
});

test("业务进程运行但历史初始化失败时不能确认新版启动成功", () => {
  assert.equal(runtimeStartupReady({ phase: "running", runtimeInitializationError: "读取历史会话失败" }), false);
  assert.equal(runtimeStartupReady({ phase: "failed", runtimeInitializationError: null }), false);
  assert.equal(runtimeStartupReady({ phase: "running", runtimeInitializationError: null }), true);
  assert.equal(runtimeStartupReady({ phase: "running" }), true);
});

test("资料未就绪、结构损坏或版本未知时不能确认启动通过", () => {
  const profile = { profileId: "0123456789abcdef0123456789abcdef", schemaVersion: 2, ready: true };
  assert.equal(localProfileReady(profile), true);
  for (const invalid of [null, {}, { ...profile, ready: false }, { ...profile, ready: "true" },
    { ...profile, schemaVersion: 1 }, { ...profile, schemaVersion: 3 }, { ...profile, profileId: "tenant-id" }]) {
    assert.equal(localProfileReady(invalid), false);
  }
});

test("PDF 启动自检按宿主平台选择已交付的渲染链路", () => {
  assert.equal(pdfSelfCheckMode("darwin"), "native-renderer");
  assert.equal(pdfSelfCheckMode("win32"), "pdfjs");
  assert.throws(() => pdfSelfCheckMode("linux"), /不支持当前平台/);
});

test("Windows 启动自检核对随包 PDF.js 资产", () => {
  const root = mkdtempSync(path.join(tmpdir(), "cilai-pdfjs-runtime-"));
  try {
    const write = (relative, content = "asset") => {
      const file = path.join(root, relative);
      mkdirSync(path.dirname(file), { recursive: true });
      writeFileSync(file, content);
    };
    write("assets/pdf.worker.min-locked.mjs");
    write("pdfjs/cmaps/Adobe-CNS1.bcmap");
    write("pdfjs/standard_fonts/FoxitSans.pfb");
    write("pdfjs/wasm/jbig2_nowasm_fallback.js");
    write("pdfjs/wasm/openjpeg_nowasm_fallback.js");
    assert.deepEqual(runtimePdfJsAssets(root), {
      workerCount: 1,
      cMapCount: 1,
      fontCount: 1,
      decoderCount: 2,
    });
    write("assets/pdf.worker.min-extra.mjs");
    assert.throws(() => runtimePdfJsAssets(root), /Worker 随包数量无效/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
