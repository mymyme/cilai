const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const { preparePersonalPackageImport } = require("./personal-package-import");

const selectedPath = path.resolve("personal-test-package.zip");
const request = { target: { mode: "new" }, clientOperationId: "personal-import-test" };
function harness(result = { canceled: false, filePaths: [selectedPath] }) {
  const calls = [];
  return { calls, window: {}, args: { request }, dialog: { showOpenDialog: async (...args) => {
    calls.push(["dialog", ...args]); return result;
  } }, service: { call: async (...args) => { calls.push(["service", ...args]); return { importId: "test-id" }; } } };
}

test("安装包准备只使用原生选择的路径，不执行安装", async () => {
  const state = harness();
  assert.deepEqual(await preparePersonalPackageImport(state), { importId: "test-id" });
  assert.deepEqual(state.calls[1], ["service", "personal_package_import_prepare", { request: {
    ...request, archivePath: selectedPath,
  } }]);
  assert.equal(state.calls.length, 2);
});

test("取消文件选择不产生准备或安装操作", async () => {
  const state = harness({ canceled: true, filePaths: [] });
  assert.equal(await preparePersonalPackageImport(state), null);
  assert.equal(state.calls.length, 1);
});

test("前端不能提供原始文件路径或以包内身份指定覆盖目标", async () => {
  for (const invalid of [
    { ...request, archivePath: selectedPath },
    { ...request, target: { mode: "new", creationProjectId: "capability_" + "a".repeat(32) } },
    { ...request, target: { mode: "upgrade", creationProjectId: "../other", expectedRevision: 1 } },
    { ...request, target: { mode: "upgrade", creationProjectId: "capability_" + "a".repeat(32), expectedRevision: 0 } },
  ]) {
    const state = harness(); state.args = { request: invalid };
    await assert.rejects(preparePersonalPackageImport(state), /personal_package_import_(request|target)_invalid/);
    assert.equal(state.calls.length, 0);
  }
});

test("明确升级目标和版本原样绑定到准备操作", async () => {
  const state = harness();
  state.args = { request: { ...request, target: { mode: "upgrade", creationProjectId: "capability_" + "b".repeat(32), expectedRevision: 3 } } };
  await preparePersonalPackageImport(state);
  assert.deepEqual(state.calls[1][2].request.target, state.args.request.target);
});

test("无效原生选择被拒绝，宿主失败不自动重试", async () => {
  for (const filePaths of [[], ["relative.zip"], [path.resolve("package.json")], [selectedPath, selectedPath]]) {
    const state = harness({ canceled: false, filePaths });
    await assert.rejects(preparePersonalPackageImport(state), /personal_package_import_archive_invalid/);
    assert.equal(state.calls.length, 1);
  }
  const state = harness(); let calls = 0;
  state.service.call = async () => { calls++; throw new Error("prepare_failed"); };
  await assert.rejects(preparePersonalPackageImport(state), /prepare_failed/);
  assert.equal(calls, 1);
});
