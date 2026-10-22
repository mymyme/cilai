import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { basename, dirname, relative, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const platformBoundaryRoots = new Set([
  "crates/local-transport",
  "crates/platform-adapter",
]);

function rustSourcesBelow(relativeDirectory) {
  const absoluteDirectory = resolve(projectRoot, relativeDirectory);
  if (!existsSync(absoluteDirectory)) return [];
  return readdirSync(absoluteDirectory, { withFileTypes: true })
    .flatMap((entry) => {
      const relativePath = `${relativeDirectory}/${entry.name}`;
      if (entry.isDirectory()) return rustSourcesBelow(relativePath);
      return entry.isFile() && entry.name.endsWith(".rs") ? [relativePath] : [];
    });
}

function productionRustSources() {
  const appSources = readdirSync(resolve(projectRoot, "apps"), { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .flatMap((entry) => [
      ...rustSourcesBelow(`apps/${entry.name}/src`),
      ...rustSourcesBelow(`apps/${entry.name}/host-service/src`),
    ]);
  const crateSources = readdirSync(resolve(projectRoot, "crates"), { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .filter((entry) => !platformBoundaryRoots.has(`crates/${entry.name}`))
    .flatMap((entry) => rustSourcesBelow(`crates/${entry.name}/src`));
  const files = [...appSources, ...crateSources].sort();
  const testOnly = testOnlyExternalModuleFiles(files, (file) => readFileSync(resolve(projectRoot, file), "utf8"));
  return files.filter((file) => !testOnly.has(file));
}

const productionFiles = productionRustSources();

function isTestOnlyAttribute(text) {
  const match = /^#\s*\[\s*cfg\s*\(([\s\S]*)\)\s*\]$/.exec(text);
  return match !== null && requiresTestPredicate(match[1]);
}

function requiresTestPredicate(expression) {
  const text = expression.trim();
  if (text === "test") return true;
  const group = /^(all|any)\s*\(([\s\S]*)\)$/.exec(text);
  if (group === null) return false;
  const argumentsText = group[2];
  const argumentsList = [];
  let start = 0;
  let depth = 0;
  let quoted = false;
  let escaped = false;
  for (let index = 0; index < argumentsText.length; index += 1) {
    const character = argumentsText[index];
    if (quoted) {
      if (escaped) escaped = false;
      else if (character === "\\") escaped = true;
      else if (character === '"') quoted = false;
    } else if (character === '"') quoted = true;
    else if (character === "(") depth += 1;
    else if (character === ")") depth -= 1;
    else if (character === "," && depth === 0) {
      argumentsList.push(argumentsText.slice(start, index));
      start = index + 1;
    }
    if (depth < 0) return false;
  }
  if (quoted || depth !== 0) return false;
  argumentsList.push(argumentsText.slice(start));
  const predicates = argumentsList.filter((item) => item.trim()).map(requiresTestPredicate);
  return group[1] === "all" ? predicates.some(Boolean) : predicates.length > 0 && predicates.every(Boolean);
}

function testOnlyExternalModuleFiles(files, readSource) {
  const known = new Set(files);
  const tests = new Set();
  const production = new Set();
  for (const parent of files) {
    const source = readSource(parent);
    const attributes = [...source.matchAll(/#\s*\[/g)].map((match) => {
      const bracket = source.indexOf("[", match.index);
      const end = findBalancedEnd(source, bracket, "[", "]");
      return { start: match.index, end, text: source.slice(match.index, end) };
    });
    for (const declaration of source.matchAll(/\b(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+(\w+)\s*;/g)) {
      let start = declaration.index;
      const attached = [];
      for (const attribute of attributes.slice().reverse()) {
        if (attribute.end > start) continue;
        if (source.slice(attribute.end, start).trim()) break;
        attached.push(attribute.text);
        start = attribute.start;
      }
      const path = attached.map((text) => /^#\s*\[\s*path\s*=\s*"([^"]+)"\s*\]$/.exec(text)).find(Boolean)?.[1];
      const stem = basename(parent, ".rs");
      const directory = ["lib", "main", "mod"].includes(stem) ? dirname(parent) : `${dirname(parent)}/${stem}`;
      const candidates = path ? [resolve(projectRoot, dirname(parent), path)] : [resolve(projectRoot, directory, `${declaration[1]}.rs`), resolve(projectRoot, directory, declaration[1], "mod.rs")];
      for (const candidate of candidates) {
        const file = relative(projectRoot, candidate).replaceAll("\\", "/");
        if (known.has(file)) (attached.some(isTestOnlyAttribute) ? tests : production).add(file);
      }
    }
  }
  return new Set([...tests].filter((file) => !production.has(file)));
}

const forbidden = [
  ["平台条件编译", countPlatformCfgAttributes],
  ["编译期平台分支", countPlatformCfgMacros],
  ["平台专属标准库", (source) => countMatches(source, /std::os::(?:windows|unix)/g)],
  ["系统命令绝对路径", (source) => countMatches(source, /["']\/(?:usr\/)?bin\//g)],
  ["Windows 文件管理器进程", (source) => countMatches(source, /explorer\.exe/g)],
  ["Homebrew 安装目录", (source) => countMatches(source, /Caskroom/g)],
  ["直接读取 HOME", (source) => countMatches(source, /var(?:_os)?\(\s*"HOME"\s*\)/g)],
];

const temporaryAllowances = new Map([
  // 唯一的具名过渡例外：PDF 渲染器是 macOS 原生实现，预览统一前两端走不同路径。
  // 前一代宿主壳里另有一份同类分支，随该壳退役已一并删除，这里是它现在唯一的落点。
  ["crates/workbench-runtime/src/workspace.rs", new Map([
    ["平台条件编译", 5],
  ])],
]);

function countMatches(source, pattern) {
  return [...source.matchAll(pattern)].length;
}

function findBalancedEnd(source, openingIndex, opening, closing) {
  let depth = 0;
  let quote = null;
  let escaped = false;
  for (let index = openingIndex; index < source.length; index += 1) {
    const character = source[index];
    if (quote !== null) {
      if (escaped) escaped = false;
      else if (character === "\\") escaped = true;
      else if (character === quote) quote = null;
      continue;
    }
    if (character === '"' || character === "'") {
      quote = character;
    } else if (character === opening) {
      depth += 1;
    } else if (character === closing) {
      depth -= 1;
      if (depth === 0) return index + 1;
    }
  }
  return source.length;
}

function rustCfgAttributes(source) {
  const attributes = [];
  const startPattern = /#\s*\[\s*cfg(?:_attr)?\s*\(/g;
  for (const match of source.matchAll(startPattern)) {
    const bracket = source.indexOf("[", match.index);
    const end = findBalancedEnd(source, bracket, "[", "]");
    attributes.push({ start: match.index, end, text: source.slice(match.index, end) });
  }
  return attributes;
}

function rustCfgMacros(source) {
  const macros = [];
  const startPattern = /\bcfg!\s*\(/g;
  for (const match of source.matchAll(startPattern)) {
    const parenthesis = source.indexOf("(", match.index);
    const end = findBalancedEnd(source, parenthesis, "(", ")");
    macros.push(source.slice(match.index, end));
  }
  return macros;
}

function containsPlatformPredicate(fragment) {
  return /\b(?:windows|unix|target_os|target_family)\b/.test(fragment);
}

function countPlatformCfgAttributes(source) {
  return rustCfgAttributes(source).filter(({ text }) => containsPlatformPredicate(text)).length;
}

function countPlatformCfgMacros(source) {
  return rustCfgMacros(source).filter(containsPlatformPredicate).length;
}

function productionSource(relativePath) {
  const source = readFileSync(resolve(projectRoot, relativePath), "utf8");
  // 只排除显式的整文件测试模块，不根据文件名豁免平台边界。
  if (/^\s*#!\s*\[\s*cfg\(\s*test\s*\)\s*\]/.test(source)) return "";
  const testModule = rustCfgAttributes(source).find(({ end, text }) =>
    isTestOnlyAttribute(text) && /^\s*mod\s+tests\s*\{/.test(source.slice(end)),
  );
  return testModule === undefined ? source : source.slice(0, testModule.start);
}

test("业务层平台差异必须留在平台适配边界", () => {
  assert.ok(productionFiles.length > 20, "业务 Rust 源码扫描范围异常为空或过小");
  const violations = [];
  for (const relativePath of productionFiles) {
    const source = productionSource(relativePath);
    for (const [label, countViolations] of forbidden) {
      const count = countViolations(source);
      const allowed = temporaryAllowances.get(relativePath)?.get(label) ?? 0;
      if (count !== allowed) {
        violations.push(`${relativePath}: ${label} ${count} 处，允许 ${allowed} 处`);
      }
    }
  }
  assert.deepEqual(violations, [], violations.join("\n"));
});

test("扫描器识别多行平台属性与一般 cfg! 平台谓词", () => {
  const source = `
#[cfg_attr(
  feature = "desktop",
  cfg(target_os = "windows")
)]
fn attributed() {}

const IS_MACOS: bool = cfg!(
  all(feature = "desktop", target_os = "macos")
);
`;
  assert.equal(countPlatformCfgAttributes(source), 1);
  assert.equal(countPlatformCfgMacros(source), 1);
});

test("扫描器在多行 test 属性前截断测试模块", () => {
  const source = `
fn production() {}
#[cfg(
  test
)]
mod tests {
  const WINDOWS: bool = cfg!(target_os = "windows");
}
`;
  const testModule = rustCfgAttributes(source).find(({ end, text }) =>
    isTestOnlyAttribute(text) && /^\s*mod\s+tests\s*\{/.test(source.slice(end)),
  );
  assert.equal(source.slice(0, testModule.start).includes("WINDOWS"), false);
  assert.equal(isTestOnlyAttribute("#[cfg(any(test, feature = \"shipping\"))]"), false);
  assert.equal(isTestOnlyAttribute("#[cfg(not(test))]"), false);
  assert.equal(isTestOnlyAttribute("#[cfg(all(test, unix))]"), true);
  assert.equal(isTestOnlyAttribute("#[cfg(any(all(test, windows), all(test, unix)))]"), true);
  assert.equal(isTestOnlyAttribute("#[cfg(all(not(test), unix))]"), false);
});

test("独立测试模块必须有明确测试条件，生产引用及文件名不能被误排除", () => {
  const files = ["example/src/lib.rs", "example/src/only.rs", "example/src/shared.rs", "example/src/mixed.rs", "example/src/looks_like_tests.rs"];
  const sources = new Map(files.map((file) => [file, "fn ordinary_source() {}"]));
  sources.set(files[0], `
#[cfg(test)]
#[path = "only.rs"]
mod external_tests;
#[cfg(test)]
#[path = "shared.rs"]
mod shared_tests;
#[path = "shared.rs"]
mod shared_production;
#[cfg(any(test, feature = "shipping"))]
#[path = "mixed.rs"]
mod mixed;
`);
  assert.deepEqual([...testOnlyExternalModuleFiles(files, (file) => sources.get(file))], [files[1]]);
  sources.set(files[0], `${sources.get(files[0])}\nmod only;`);
  assert.deepEqual([...testOnlyExternalModuleFiles(files, (file) => sources.get(file))], []);
});

test("平台分支只限于 PDF 渲染", () => {
  // 例外按用途成立而不是按数量成立：允许表里的数字只说明有几处，说不出是哪几处。
  // 这里确认它们确实都在 PDF 预览上，而不是借这个名额混进了别的平台差异。
  const source = productionSource("crates/workbench-runtime/src/workspace.rs");
  const branches = [...source.matchAll(/#\[cfg\((?:not\()?target_os = "macos"\)?\)\]\r?\n\s*(\S+)/g)];
  assert.equal(branches.length, 5);
  for (const [, next] of branches) {
    assert.ok(/^(mod|pub|fn|use)/.test(next), `平台分支后应为 PDF 预览的定义，实际为 ${next}`);
  }
  assert.match(source, /pub mod pdf \{/);
});

test("工作区文件系统不保留重复源码占位文件", () => {
  assert.equal(existsSync(resolve(projectRoot, "crates/workspace-fs/src/lib 2.rs")), false);
});
