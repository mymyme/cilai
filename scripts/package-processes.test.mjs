import test from "node:test";
import assert from "node:assert/strict";
import { assertPackageNotInUse, waitForPackageExit } from "./package-processes.mjs";

const bundle = "/tmp/词来工作台.app";
test("打包同时拒绝主程序及退出后留下的随包服务", () => {
  for (const executable of ["MacOS/词来工作台", "Resources/sidecar/cilai-app-server", "Frameworks/词来工作台 Helper.app/Contents/MacOS/词来工作台 Helper"]) {
    assert.throws(() => assertPackageNotInUse(bundle, () => ({status:0,stdout:`  42 ${bundle}/Contents/${executable}\n`})), /仍有 1 个运行进程/);
  }
});
test("进程检查只认实际可执行路径，其他目录和检查命令不会阻止打包", () => {
  assert.deepEqual(assertPackageNotInUse(bundle, () => ({status:0,stdout:"12 /usr/bin/python3\n13 /tmp/词来工作台.app.backup/Contents/MacOS/词来工作台\n"})), []);
});
test("进程检查被拒绝时明确停止，不把失败当作无运行进程", () => {
  assert.throws(() => assertPackageNotInUse(bundle, () => ({status:1,stdout:""})), /无法检查/);
  assert.throws(() => assertPackageNotInUse(bundle, () => ({status:null,error:new Error("EPERM"),stdout:""})), /无法检查/);
});
test("自检退出等待确认全部进程归还", async () => {
  const states = [`42 ${bundle}/Contents/Resources/sidecar/cilai-app-server\n`, ""];
  await waitForPackageExit(bundle, 1000, () => ({status:0,stdout:states.shift()}));
  assert.equal(states.length, 0);
});
test("自检的残留进程及检查失败均明确拒绝，无终止进程动作", async () => {
  await assert.rejects(waitForPackageExit(bundle, 0, () => ({status:0,stdout:`42 ${bundle}/Contents/Resources/sidecar/cilai-app-server\n`})), /未完成收尾/);
  await assert.rejects(waitForPackageExit(bundle, 0, () => ({status:1,stdout:""})), /无法检查/);
});
