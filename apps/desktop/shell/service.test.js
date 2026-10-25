const test = require("node:test");
const assert = require("node:assert/strict");
const { mkdtempSync, writeFileSync, existsSync, readFileSync, rmSync } = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { once } = require("node:events");

const { HostService, resolveBinary, resolveServiceEnvironment } = require("./service");
const { loadDataEnvironment } = require("./data-environment");

test("正常停止向真实宿主交付输入结束并等待自身收尾，不发送终止信号", async () => {
  const root = mkdtempSync(path.join(os.tmpdir(), "cilai-host-eof-"));
  const ready = path.join(root, "ready");
  const result = path.join(root, "result");
  const preload = path.join(root, "probe.cjs");
  writeFileSync(preload, `const fs=require('node:fs');
    process.on('SIGTERM',()=>{fs.writeFileSync(${JSON.stringify(result)},'signal');process.exit(2)});
    process.stdin.on('end',()=>{fs.writeFileSync(${JSON.stringify(result)},'eof');process.exit(0)});
    fs.writeFileSync(${JSON.stringify(ready)},'ready');`);
  // 以 Node 充当宿主只验证停止流程；随包 Git 运行时由 git-runtime.test.js 单独覆盖。
  const service = new HostService(process.execPath, (_binary, env) => env);
  let child;
  try {
    service.start({ NODE_OPTIONS: `--require=${JSON.stringify(preload)}` });
    child = service.child;
    assert.ok(child, service.startError);
    for (let i = 0; i < 200 && !existsSync(ready); i++) await new Promise(resolve => setTimeout(resolve, 10));
    assert.ok(existsSync(ready), "宿主探针未就绪");
    const exited = once(child, "exit");
    const pending = assert.rejects(service.call("probe", {}), /业务服务已停止/);
    service.stop();
    service.stop();
    await pending;
    const [code, signal] = await Promise.race([exited, new Promise((_, reject) => setTimeout(() => reject(new Error("宿主未完成收尾")), 2000))]);
    assert.equal(code, 0);
    assert.equal(signal, null);
    assert.equal(readFileSync(result, "utf8"), "eof");
    assert.equal(service.child, null);
  } finally {
    if (child && child.exitCode === null) child.kill();
    rmSync(root, { recursive: true, force: true });
  }
});

test("宿主服务程序缺失时明确结束在途调用", async () => {
  const service = new HostService("/nonexistent/cilai-host-service");
  service.start();
  await assert.rejects(service.call("host_platform", {}), /业务服务启动失败|ENOENT/);
  assert.equal(service.child, null);
});

test("个人宿主清除继承的企业、模型环境，保留已授权浏览器地址", () => {
  const env = resolveServiceEnvironment({ PATH: '/usr/bin', CILAI_ENTERPRISE_ORIGIN: 'https://enterprise.invalid',
    CILAI_LSP_ORIGIN: 'https://lsp.invalid', CILAI_COLLABORATION_ORIGIN: 'https://space.invalid',
    CILAI_WORKBENCH_ACCESS_TOKEN: 'test-only', OPENAI_API_KEY: 'test-only', CODEX_HOME: '/forbidden',
    cilai_lsp_member_key: 'test-only', openai_api_key: 'test-only', codex_home: '/forbidden',
    CILAI_BROWSER_MCP_URL: 'http://127.0.0.1:1234', CILAI_BROWSER_MCP_TOKEN: 'scoped-test' });
  assert.deepEqual(env, { PATH: '/usr/bin', CILAI_BROWSER_MCP_URL: 'http://127.0.0.1:1234', CILAI_BROWSER_MCP_TOKEN: 'scoped-test' });
  assert.deepEqual(resolveServiceEnvironment({}), {});
});

test("数据环境只接收可信显式选择，不继承外部进程的同名变量", () => {
  const inherited = { PATH: '/usr/bin', CILAI_DATA_ENVIRONMENT: 'prod', cilai_data_environment: 'dev' };
  assert.deepEqual(resolveServiceEnvironment(inherited), { PATH: '/usr/bin' });
  for (const selected of ['dev', 'prod']) {
    assert.deepEqual(resolveServiceEnvironment(inherited, { CILAI_DATA_ENVIRONMENT: selected }), {
      PATH: '/usr/bin', CILAI_DATA_ENVIRONMENT: selected,
    });
  }
  for (const selected of ['', '../dev', 'DEV', undefined, null]) {
    assert.throws(() => resolveServiceEnvironment(inherited, { CILAI_DATA_ENVIRONMENT: selected }), /数据环境配置无效/);
  }
  assert.deepEqual(resolveServiceEnvironment(inherited, { CILAI_ENTERPRISE_ORIGIN: 'https://enterprise.invalid', OPENAI_API_KEY: 'test-only' }), { PATH: '/usr/bin' });
});

test("保存的数据环境实际传入宿主子进程且不会放开继承的企业配置", async () => {
  const root = mkdtempSync(path.join(os.tmpdir(), 'cilai-host-environment-'));
  const result = path.join(root, 'environment.json');
  const probe = path.join(root, 'probe.cjs');
  writeFileSync(probe, `require('node:fs').writeFileSync(${JSON.stringify(result)},JSON.stringify({environment:process.env.CILAI_DATA_ENVIRONMENT,enterprise:process.env.CILAI_ENTERPRISE_ORIGIN}));process.stdin.resume();`);
  const inheritedEnvironment = process.env.CILAI_DATA_ENVIRONMENT;
  const service = new HostService(process.execPath, (_binary, env) => env);
  let child;
  try {
    const selected = loadDataEnvironment({ userData: root, args: ['--adopt-data-environment=dev'] });
    process.env.CILAI_DATA_ENVIRONMENT = 'prod';
    service.start({ ...selected, CILAI_ENTERPRISE_ORIGIN: 'https://enterprise.invalid', NODE_OPTIONS: `--require=${JSON.stringify(probe)}` });
    child = service.child;
    assert.ok(child, service.startError);
    for (let i = 0; i < 200 && !existsSync(result); i++) await new Promise(resolve => setTimeout(resolve, 10));
    assert.ok(existsSync(result), '宿主环境探针未就绪');
    assert.deepEqual(JSON.parse(readFileSync(result, 'utf8')), { environment: 'dev' });
    const exited = once(child, 'exit');
    service.stop();
    await exited;
  } finally {
    if (inheritedEnvironment === undefined) delete process.env.CILAI_DATA_ENVIRONMENT;
    else process.env.CILAI_DATA_ENVIRONMENT = inheritedEnvironment;
    if (child && child.exitCode === null) child.kill();
    rmSync(root, { recursive: true, force: true });
  }
});

test("开发宿主拒绝企业构建目录并使用个人目录", () => {
  assert.throws(() => resolveBinary({platform:'darwin', packaged:false, buildRoot:'/forbidden/desktop'}), /个人版构建目录/);
  assert.match(resolveBinary({platform:'darwin',packaged:false,buildRoot:undefined}), /\.cilai-personal-builds\/desktop\/target\/debug\/cilai-host-service$/);
  assert.equal(resolveBinary({platform:'win32',packaged:true,resourcesPath:'C:\\Personal\\resources'}), 'C:\\Personal\\resources\\sidecar\\cilai-host-service.exe');
});
