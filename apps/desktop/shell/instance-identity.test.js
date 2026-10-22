const assert = require("node:assert/strict");
const test = require("node:test");
const { differentInstancePath } = require("./instance-identity");

test("另一份工作台测试包启动时识别其可执行文件路径", () => {
  const current = "/build/desktop/词来工作台.app/Contents/MacOS/词来工作台";
  const requested = "/build/desktop/file-status/词来工作台.app/Contents/MacOS/词来工作台";
  assert.equal(differentInstancePath(current, { executablePath: requested }), requested);
  assert.equal(differentInstancePath(current, { executablePath: current }), null);
  assert.equal(differentInstancePath(current, {}), null);
});
