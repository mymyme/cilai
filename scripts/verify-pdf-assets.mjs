import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readdir, readFile, stat } from "node:fs/promises";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pdfJsRoot = join(projectRoot, "node_modules", "pdfjs-dist");
const distRoot = join(projectRoot, "apps", "desktop-ui", "dist");

const cMapCount = await verifyFlatAssets(
  "PDF.js CMap",
  join(pdfJsRoot, "cmaps"),
  join(distRoot, "pdfjs", "cmaps"),
);
const fontCount = await verifyFlatAssets(
  "PDF.js 标准字体",
  join(pdfJsRoot, "standard_fonts"),
  join(distRoot, "pdfjs", "standard_fonts"),
);
const noWasmDecoderCount = await verifyFlatAssets(
  "PDF.js no-WASM 解码模块",
  join(pdfJsRoot, "wasm"),
  join(distRoot, "pdfjs", "wasm"),
  (file) => file.endsWith("_nowasm_fallback.js"),
);
assert.equal(noWasmDecoderCount, 2, "PDF.js no-WASM 解码模块数量必须为 2");
await verifyWorker();
assert.deepEqual(
  (await listFiles(distRoot)).filter((file) => file.endsWith(".wasm")),
  [],
  "PDF.js 禁用 WASM 时构建产物不得包含 WASM 二进制",
);

console.log(`PDF.js 本地资产校验通过：1 个 Worker、${cMapCount} 个 CMap、${fontCount} 个标准字体文件、${noWasmDecoderCount} 个 no-WASM 解码模块`);

async function verifyFlatAssets(label, sourceRoot, outputRoot, include = () => true) {
  const sourceFiles = (await listFiles(sourceRoot)).filter(include);
  assert.ok(sourceFiles.length > 0, `${label} 源目录为空`);

  const expectedNames = sourceFiles.map((file) => basename(file));
  assert.equal(new Set(expectedNames).size, expectedNames.length, `${label} 存在重名文件，不能扁平复制`);

  const outputFiles = await listFiles(outputRoot);
  assert.deepEqual(outputFiles.toSorted(), expectedNames.toSorted(), `${label} 输出文件集合不完整或目录层级错误`);

  for (const sourceFile of sourceFiles) {
    const outputFile = basename(sourceFile);
    const sourcePath = join(sourceRoot, sourceFile);
    const outputPath = join(outputRoot, outputFile);
    const [sourceStat, outputStat] = await Promise.all([stat(sourcePath), stat(outputPath)]);
    assert.ok(sourceStat.isFile() && sourceStat.size > 0, `${label} 源文件无效：${sourceFile}`);
    assert.ok(outputStat.isFile() && outputStat.size > 0, `${label} 输出文件无效：${outputFile}`);
    assert.equal(await sha256(outputPath), await sha256(sourcePath), `${label} 文件内容不一致：${outputFile}`);
  }

  return sourceFiles.length;
}

async function verifyWorker() {
  const workerFiles = (await listFiles(distRoot)).filter((file) =>
    /^pdf\.worker(?:\.min)?(?:[-.][^/]*)?\.m?js$/.test(basename(file)),
  );
  assert.equal(workerFiles.length, 1, "PDF.js Worker 输出数量必须为 1");
  assert.match(workerFiles[0], /^assets\/pdf\.worker\.min-[A-Za-z0-9_-]+\.mjs$/, "PDF.js Worker 输出位置或名称无效");

  const source = join(pdfJsRoot, "build", "pdf.worker.min.mjs");
  const output = join(distRoot, workerFiles[0]);
  const outputStat = await stat(output);
  assert.ok(outputStat.size > 0, "PDF.js Worker 输出为空");
  assert.equal(await sha256(output), await sha256(source), "PDF.js Worker 与锁定依赖内容不一致");
}

async function listFiles(root) {
  const files = [];
  await visit(root, root, files);
  return files;
}

async function visit(root, directory, files) {
  const entries = await readdir(directory, { withFileTypes: true });
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      await visit(root, path, files);
    } else if (entry.isFile()) {
      files.push(relative(root, path).split(sep).join("/"));
    }
  }
}

async function sha256(path) {
  return createHash("sha256").update(await readFile(path)).digest("hex");
}
