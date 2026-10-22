import { createHash } from "node:crypto";
import {
  access,
  chmod,
  mkdir,
  symlink,
  writeFile,
} from "node:fs/promises";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);

if (args.length !== 2 || args[0] !== "--output" || !args[1]) {
  console.error("用法：npm run fixtures:generate -- --output <源码目录之外的新目录>");
  process.exit(2);
}

const outputRoot = resolve(args[1]);
const outputRelativeToProject = relative(projectRoot, outputRoot);

if (
  outputRelativeToProject === "" ||
  (!outputRelativeToProject.startsWith("..") && !isAbsolute(outputRelativeToProject))
) {
  console.error(`Fixture 输出目录不得位于源码树内：${outputRoot}`);
  process.exit(2);
}

try {
  await access(outputRoot);
  console.error(`Fixture 输出目录必须尚不存在：${outputRoot}`);
  process.exit(2);
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}

const writeJson = async (path, value) => {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
};

const writeText = async (path, value) => {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, value, "utf8");
};

const writeBinary = async (path, value) => {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, value);
};

const sha256 = (value) => createHash("sha256").update(value).digest("hex");

await mkdir(outputRoot, { recursive: false });

const smallRoot = resolve(outputRoot, "small");
const smallFiles = Array.from({ length: 100 }, (_, index) => {
  const number = String(index + 1).padStart(3, "0");
  return writeText(
    resolve(smallRoot, "documents", `document-${number}.md`),
    `# 小型样本 ${number}\n\n这是用于功能测试的确定性文件。\n`,
  );
});
await Promise.all(smallFiles);
await writeJson(resolve(smallRoot, "_fixture.json"), {
  fixtureSchemaVersion: 1,
  kind: "small",
  businessFileCount: 100,
  purpose: "功能、扫描和状态测试",
});

const mediumRoot = resolve(outputRoot, "medium");
for (let batchStart = 0; batchStart < 10_000; batchStart += 200) {
  const writes = [];
  for (let index = batchStart; index < batchStart + 200; index += 1) {
    const number = String(index + 1).padStart(5, "0");
    const bucket = String(Math.floor(index / 250)).padStart(2, "0");
    writes.push(
      writeText(
        resolve(mediumRoot, `group-${bucket}`, `file-${number}.txt`),
        `ContextHub medium fixture ${number}\n`,
      ),
    );
  }
  await Promise.all(writes);
}
await writeJson(resolve(mediumRoot, "_fixture.json"), {
  fixtureSchemaVersion: 1,
  kind: "medium",
  businessFileCount: 10_000,
  purpose: "扫描、监听和性能基准",
});

const abnormalRoot = resolve(outputRoot, "abnormal");
await writeBinary(resolve(abnormalRoot, "encoding", "unknown-encoding.bin"), Buffer.from([0xff, 0xfe, 0x00, 0x81, 0x82]));
await writeBinary(resolve(abnormalRoot, "large", "2mb.bin"), Buffer.alloc(2 * 1024 * 1024, 0x5a));
await writeText(resolve(abnormalRoot, "permissions", "unreadable.txt"), "permission fixture\n");
await chmod(resolve(abnormalRoot, "permissions", "unreadable.txt"), 0o000);
await mkdir(resolve(abnormalRoot, "links"), { recursive: true });
await symlink("../..", resolve(abnormalRoot, "links", "outside-workspace"));
const longSegments = Array.from({ length: 12 }, (_, index) => `segment-${String(index).padStart(2, "0")}-abcdefghijklmnop`);
await writeText(resolve(abnormalRoot, "long-path", ...longSegments, "document.md"), "long path fixture\n");
await writeJson(resolve(abnormalRoot, "_fixture.json"), {
  fixtureSchemaVersion: 1,
  kind: "abnormal",
  scenarios: ["unknown-encoding", "large-file", "unreadable-file", "symlink-escape", "long-path"],
  purpose: "错误边界和明确失败状态测试",
});

const conflictRoot = resolve(outputRoot, "conflict");
const baseline = "# 方案\n\n基线内容。\n";
const local = "# 方案\n\n本地修改后的内容。\n";
const platform = "# 方案\n\n平台修改后的内容。\n";
await writeText(resolve(conflictRoot, "baseline", "document.md"), baseline);
await writeText(resolve(conflictRoot, "local", "document.md"), local);
await writeText(resolve(conflictRoot, "platform", "document.md"), platform);
await writeJson(resolve(conflictRoot, "_fixture.json"), {
  fixtureSchemaVersion: 1,
  kind: "conflict",
  documentPath: "document.md",
  baselineSha256: sha256(baseline),
  localSha256: sha256(local),
  platformSha256: sha256(platform),
  expectedState: "conflict",
  purpose: "本地与平台从同一基线分叉的冲突测试",
});

await writeJson(resolve(outputRoot, "fixture-index.json"), {
  fixtureSchemaVersion: 1,
  deterministicSeed: "contexthub-m0-v1",
  fixtureKinds: ["small", "medium", "abnormal", "conflict"],
  productionDataAllowed: false,
});

console.log(`Fixture 已生成：${outputRoot}`);
console.log("small=100, medium=10000, abnormal=5 scenarios, conflict=1 document");
