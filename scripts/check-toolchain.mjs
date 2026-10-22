import { realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";

export const EXPECTED_NODE_VERSION = "24.18.0";
export const SUPPORTED_BUILD_HOSTS = Object.freeze([
  "darwin/arm64",
  "darwin/x64",
  "win32/x64",
]);

export function validateToolchain({ nodeVersion, platform, arch }) {
  if (nodeVersion !== EXPECTED_NODE_VERSION) {
    throw new Error(`Node 版本不匹配：要求 ${EXPECTED_NODE_VERSION}，当前 ${nodeVersion}`);
  }

  const currentHost = `${platform}/${arch}`;
  if (!SUPPORTED_BUILD_HOSTS.includes(currentHost)) {
    throw new Error(
      `构建环境不匹配：仅支持 ${SUPPORTED_BUILD_HOSTS.join("、")}，当前 ${currentHost}`,
    );
  }

  return `工具链环境符合构建基线：Node ${nodeVersion}，${currentHost}`;
}

const invokedPath = process.argv[1] ? realpathSync.native(process.argv[1]) : null;
const modulePath = realpathSync.native(fileURLToPath(import.meta.url));
const isDirectInvocation = invokedPath === modulePath;

if (isDirectInvocation) {
  try {
    console.log(
      validateToolchain({
        nodeVersion: process.versions.node,
        platform: process.platform,
        arch: process.arch,
      }),
    );
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
