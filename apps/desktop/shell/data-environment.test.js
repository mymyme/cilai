const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { loadDataEnvironment } = require("./data-environment");

function fixture(t) {
  const userData = fs.mkdtempSync(path.join(os.tmpdir(), "cilai-data-environment-"));
  t.after(() => fs.rmSync(userData, { recursive: true, force: true }));
  return { userData, file: path.join(userData, "data-environment.json") };
}

test("普通启动使用原构建环境，不自动创建数据绑定", t => {
  const { userData, file } = fixture(t);
  assert.deepEqual(loadDataEnvironment({ userData, args: [] }), {});
  assert.equal(fs.existsSync(file), false);
});

test('打包版本首次启动保留数据环境，跨通道与跨构建升级不能切换目录', t => {
  const { userData } = fixture(t);
  assert.deepEqual(loadDataEnvironment({ userData, args: [], defaultEnvironment: 'dev' }), { CILAI_DATA_ENVIRONMENT: 'dev' });
  assert.deepEqual(loadDataEnvironment({ userData, args: [], defaultEnvironment: 'prod' }), { CILAI_DATA_ENVIRONMENT: 'dev' });
  assert.throws(() => loadDataEnvironment({ userData, args: [], defaultEnvironment: 'invalid' }), /无效/);
});

test("显式接续原调试数据后，无参数重启及后续升级沿用同一环境", t => {
  const { userData, file } = fixture(t);
  assert.deepEqual(loadDataEnvironment({ userData, args: ["--adopt-data-environment=dev"] }), { CILAI_DATA_ENVIRONMENT: "dev" });
  const saved = fs.readFileSync(file, "utf8");
  assert.deepEqual(loadDataEnvironment({ userData, args: [] }), { CILAI_DATA_ENVIRONMENT: "dev" });
  assert.throws(() => loadDataEnvironment({ userData, args: ["--adopt-data-environment=prod"] }), /不一致/);
  assert.equal(fs.readFileSync(file, "utf8"), saved);
  assert.deepEqual(loadDataEnvironment({ userData, args: ["--adopt-data-environment=dev"] }), { CILAI_DATA_ENVIRONMENT: "dev" });
});

test("拒绝无效和重复的接续参数，不生成配置或切换目录", t => {
  const { userData, file } = fixture(t);
  for (const value of ["", "test", "../dev", "DEV"]) {
    assert.throws(() => loadDataEnvironment({ userData, args: [`--adopt-data-environment=${value}`] }), /无效/);
  }
  assert.throws(() => loadDataEnvironment({ userData, args: ["--adopt-data-environment=dev", "--adopt-data-environment=prod"] }), /重复/);
  assert.equal(fs.existsSync(file), false);
});

test("已有绑定损坏时明确失败，保留原文件并阻止回退到空数据目录", t => {
  const { userData, file } = fixture(t);
  for (const contents of ["{", "null", '{"schemaVersion":2,"environment":"dev"}', '{"schemaVersion":1,"environment":"../dev"}', " ".repeat(1025)]) {
    fs.writeFileSync(file, contents);
    assert.throws(() => loadDataEnvironment({ userData, args: ["--adopt-data-environment=dev"] }), /无法读取原有数据环境/);
    assert.equal(fs.readFileSync(file, "utf8"), contents);
  }
});
