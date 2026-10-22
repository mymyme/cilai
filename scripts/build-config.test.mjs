// 构建配置的约束检查。
//
// 本文件原先覆盖前一代宿主壳的 Windows 打包配置：内置浏览器组件的引导安装、离线安装包、
// 两种 flavor 的发布顺序。渲染底座统一后引擎随包分发，那一整套连同它所约束的配置一并
// 不存在了，相关断言随之删除——断言的对象没了还留着，只会在下一个人手里变成噪声。
//
// 保留的是仍有对象的部分：Cargo 构建配置、质量入口的 Node 锁定、Windows 脚本编码，以及
// 界面构建与 PDF 预览的本地资产约束。随包组件的清单检查已由 apps/desktop 的暂存脚本
// 测试覆盖，不在此重复。
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const projectRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const uiRoot = join(projectRoot, "apps", "desktop-ui");

const prepareSidecarScript = await readFile(
  join(projectRoot, "script", "prepare_sidecar.ps1"),
  "utf8",
);
const macosQualityScript = await readFile(join(projectRoot, "script", "quality.sh"), "utf8");
const macosPrepareSidecarScript = await readFile(
  join(projectRoot, "script", "prepare_sidecar.sh"),
  "utf8",
);
const rootCargoManifest = await readFile(join(projectRoot, "Cargo.toml"), "utf8");
const uiPackage = JSON.parse(await readFile(join(uiRoot, "package.json"), "utf8"));
const viteConfig = await readFile(join(uiRoot, "vite.config.ts"), "utf8");
const pdfJsRenderer = await readFile(join(uiRoot, "src", "pdfjs-renderer.ts"), "utf8");
const shellMain = await readFile(join(projectRoot, "apps", "desktop", "shell", "main.js"), "utf8");

test("开发构建限制调试信息且 Codex Harness 不保留增量对象", () => {
  assert.match(
    rootCargoManifest,
    /\[profile\.dev\]\s+debug = "limited"\s+incremental = false/,
  );
  assert.match(
    rootCargoManifest,
    /\[profile\.dev\.package\."\*"\]\s+debug = "none"/,
  );
  assert.match(
    rootCargoManifest,
    /\[profile\.test\]\s+debug = "limited"\s+incremental = false/,
  );
  assert.match(macosPrepareSidecarScript, /prepare-codex-runtime\.mjs/);
  assert.match(prepareSidecarScript, /prepare-codex-runtime\.mjs/);
  assert.doesNotMatch(macosPrepareSidecarScript, /CODEX_BUILD_ARGS/);
  assert.doesNotMatch(prepareSidecarScript, /\$CodexArgs/);
});

test("macOS 质量入口在暂存前锁定 Node 24.18.0", () => {
  assert.match(macosQualityScript, /REQUIRED_NODE_VERSION="v24\.18\.0"/);
  assert.match(macosQualityScript, /source "\$NVM_SCRIPT"/);
  assert.match(macosQualityScript, /nvm use "\$\{REQUIRED_NODE_VERSION#v\}"/);
  assert.match(macosQualityScript, /node --version/);

  const versionGate = macosQualityScript.indexOf('REQUIRED_NODE_VERSION="v24.18.0"');
  const staging = macosQualityScript.indexOf("node \"$PROJECT_ROOT/scripts/stage-build.mjs\"");
  assert.ok(versionGate >= 0 && versionGate < staging);
});

test("Windows PowerShell 入口使用 Windows PowerShell 5.1 可识别的 UTF-8 BOM", async () => {
  for (const scriptName of ["prepare_sidecar.ps1", "quality.ps1", "package.ps1", "windows-build-environment.ps1"]) {
    const bytes = await readFile(join(projectRoot, "script", scriptName));
    assert.deepEqual([...bytes.subarray(0, 3)], [0xef, 0xbb, 0xbf], scriptName);
  }
});

test("Vite 只保留 TypeScript 配置并打包 PDF.js 本地资产", () => {
  assert.equal(existsSync(join(uiRoot, "vite.config.ts")), true);
  assert.equal(existsSync(join(uiRoot, "vite.config.js")), false);
  assert.equal(existsSync(join(uiRoot, "vite.config.d.ts")), false);
  assert.equal(uiPackage.dependencies["pdfjs-dist"], "6.3.289");
  assert.equal(uiPackage.scripts.postbuild, "node ../../scripts/verify-pdf-assets.mjs");
  assert.match(viteConfig, /pdfAssets\(pdfJsRoot\)/);
  assert.match(pdfJsRenderer, /pdf\.worker\.min\.mjs\?url/);
  assert.match(pdfJsRenderer, /PDFWorker\.create\(\{ port: browserWorker \}\)/);
  assert.match(pdfJsRenderer, /isEvalSupported: false/);
  assert.match(pdfJsRenderer, /useWorkerFetch: false/);
  assert.match(pdfJsRenderer, /useWasm: false/);
  assert.match(pdfJsRenderer, /wasmUrl: localAssetUrl\("pdfjs\/wasm\/"\)/);
  assert.doesNotMatch(`${viteConfig}\n${pdfJsRenderer}`, /https?:\/\//);
  assert.doesNotMatch(pdfJsRenderer, /GlobalWorkerOptions/);
});

test("PDF Worker 的 CSP 只增加本地与 blob 来源", () => {
  // 策略此前由前一代宿主的打包配置声明，现在由宿主壳在响应头上下发，约束不变。
  const csp = shellMain.slice(shellMain.indexOf("const CSP = ["), shellMain.indexOf("].join"));
  assert.match(csp, /worker-src 'self' blob:/);
  assert.doesNotMatch(csp, /https?:|unsafe-eval|wasm-unsafe-eval/);
});
