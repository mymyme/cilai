import assert from "node:assert/strict";
import test from "node:test";

import {
  EXPECTED_NODE_VERSION,
  SUPPORTED_BUILD_HOSTS,
  validateToolchain,
} from "./check-toolchain.mjs";

for (const host of SUPPORTED_BUILD_HOSTS) {
  test(`允许构建主机 ${host}`, () => {
    const [platform, arch] = host.split("/");

    assert.equal(
      validateToolchain({
        nodeVersion: EXPECTED_NODE_VERSION,
        platform,
        arch,
      }),
      `工具链环境符合构建基线：Node ${EXPECTED_NODE_VERSION}，${host}`,
    );
  });
}

test("拒绝非基线 Node 版本", () => {
  assert.throws(
    () =>
      validateToolchain({
        nodeVersion: "24.17.0",
        platform: "win32",
        arch: "x64",
      }),
    /Node 版本不匹配：要求 24\.18\.0，当前 24\.17\.0/,
  );
});

for (const host of ["darwin/ia32", "win32/arm64", "linux/x64"]) {
  test(`拒绝未列入基线的构建主机 ${host}`, () => {
    const [platform, arch] = host.split("/");

    assert.throws(
      () =>
        validateToolchain({
          nodeVersion: EXPECTED_NODE_VERSION,
          platform,
          arch,
        }),
      new RegExp(`构建环境不匹配.*当前 ${host}`),
    );
  });
}
