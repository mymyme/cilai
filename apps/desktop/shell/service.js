const { spawn } = require("node:child_process");
const path = require("node:path");
const readline = require("node:readline");
const { gitRuntimeEnvironment } = require("./git-runtime");
const { isUpdateRead } = require('./update-install-guard');

const { personalServiceEnvironment, personalBuildRoot } = require('./personal-isolation');

function resolveServiceEnvironment(source = {}, explicit = {}) {
  return personalServiceEnvironment(source, explicit);
}

/**
 * 业务服务进程的客户端。
 *
 * 服务以标准输入输出按行交换 JSON。进程退出时所有在途请求立即以失败结束，而不是悬挂等待——
 * 悬挂会让界面停在加载态且无从判断原因，失败则能立刻暴露服务不可用。
 */
class HostService {
  /** `prepareEnvironment` 默认配置随包 Git 运行时；仅测试以替身宿主验证进程生命周期时替换。 */
  constructor(binaryPath, prepareEnvironment = gitRuntimeEnvironment) {
    this.binaryPath = binaryPath;
    this.prepareEnvironment = prepareEnvironment;
    this.pending = new Map();
    this.nextId = 0;
    this.child = null;
    /** 反向调用的处理器，由壳注入。 */
    this.onHostRequest = null;
    /** 标准错误输出的落盘记录，由壳在知道日志目录后注入。 */
    this.diagnosticsLog = null;
    this.testDiagnostics = null;
  }

  start(env, arguments_ = []) {
    let runtimeEnvironment;
    this.startError = null;
    try {
      runtimeEnvironment = this.prepareEnvironment(this.binaryPath, resolveServiceEnvironment({ ...process.env, ...(env ?? {}) }, env ?? {}));
    } catch (error) {
      this.startError = `业务服务启动失败：${error.message}`;
      return this;
    }
    const child = spawn(this.binaryPath, arguments_, {
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
      // 额外环境变量用于把壳提供的能力地址交给业务进程，凭据只经此传递、不落盘
      env: runtimeEnvironment,
    });
    this.child = child;
    this.testDiagnostics?.record("service_start");
    const fail = (reason) => {
      if (this.child !== child) return;
      console.error("[service]", reason);
      this.startError = reason;
      this.testDiagnostics?.record("service_error");
      for (const [, entry] of this.pending) entry.reject(new Error(reason));
      this.pending.clear();
      this.child = null;
    };
    readline.createInterface({ input: child.stdout }).on("line", (line) => {
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        console.warn("[service] 无法解析的响应:", line.slice(0, 200));
        return;
      }
      // 业务侧反向请求宿主能力：两个方向共用同一对管道，靠字段区分
      if (typeof message.hostRequest === "number") {
        void this.serveHostRequest(message);
        return;
      }
      const entry = this.pending.get(message.id);
      if (!entry) return;
      this.pending.delete(message.id);
      this.testDiagnostics?.response(entry.command, message.result, message.error, Date.now() - entry.startedAt);
      if (message.error) entry.reject(new Error(message.error));
      else entry.resolve(message.result);
    });
    child.stderr.on("data", (chunk) => {
      this.testDiagnostics?.stderr(chunk);
      console.warn("[service]", String(chunk).trim());
      if (!this.diagnosticsLog || (this.testDiagnostics && !this.testDiagnostics.status().enabled)) return;
      try {
        this.diagnosticsLog.write(chunk);
      } catch (error) {
        // 日志写不进去不影响业务服务本身，但要让开发者在控制台看到原因
        console.error("[service] 诊断日志写入失败:", error);
        this.diagnosticsLog = null;
      }
    });
    child.on("error", (error) => fail(`业务服务启动失败：${error.message}`));
    child.on("exit", (code, signal) => {
      this.testDiagnostics?.record("service_exit", { exitCode: code });
      const reason = `业务服务已退出（code ${code}${signal ? ", signal " + signal : ""}）`;
      fail(reason);
    });
    return this;
  }

  /**
   * 处理业务侧的反向调用。
   *
   * 未注册处理器时明确拒绝而不是静默丢弃：丢弃会让业务侧一直等到超时，错误信息里
   * 只剩"未在限期内回执"，看不出是宿主没有这项能力。
   */
  async serveHostRequest(message) {
    const reply = (payload) => {
      if (!this.child) return;
      this.child.stdin.write(`${JSON.stringify({ hostResponse: message.hostRequest, ...payload })}\n`);
    };
    try {
      if (!this.onHostRequest) throw new Error(`宿主未提供该能力：${message.command}`);
      reply({ result: await this.onHostRequest(message.command, message.args ?? {}) });
    } catch (error) {
      reply({ error: String(error?.message ?? error) });
    }
  }

  call(command, args) {
    if (this.updateHold && !isUpdateRead(command, args?.method)) return Promise.reject(new Error('正在准备重启更新，请稍后再操作。'));
    if (!this.child) return Promise.reject(new Error(this.startError ?? "业务服务未运行"));
    this.nextId += 1;
    const id = this.nextId;
    const line = JSON.stringify({ id, command, args: args ?? {} });
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, command, method: args?.method, startedAt: Date.now() });
      this.child.stdin.write(line + "\n", (error) => {
        if (!error) return;
        this.pending.delete(id);
        reject(error);
      });
    });
  }

  stop() {
    if (!this.child) return;
    const child = this.child;
    this.child = null;
    for (const [, entry] of this.pending) entry.reject(new Error("业务服务已停止"));
    this.pending.clear();
    child.stdin.end();
  }
}

function resolveBinary({
  platform = process.platform,
  packaged = require("electron").app?.isPackaged,
  resourcesPath = process.resourcesPath,
  buildRoot = process.env.CILAI_BUILD_ROOT,
} = {}) {
  // 打包后业务服务随包置于资源目录；开发期指向统一构建目录的产物
  if (platform !== "darwin" && platform !== "win32") {
    throw new Error(`桌面宿主暂不支持当前平台：${platform}`);
  }
  const paths = platform === "win32" ? path.win32 : path.posix;
  const name = platform === "win32" ? "cilai-host-service.exe" : "cilai-host-service";
  if (packaged) {
    if (!resourcesPath) throw new Error("无法定位随包资源目录");
    return paths.join(resourcesPath, "sidecar", name);
  }
  const root = personalBuildRoot(buildRoot);
  return paths.join(root, "target", ...(platform === "win32" ? ["x86_64-pc-windows-msvc"] : []), "debug", name);
}

module.exports = { HostService, resolveBinary, resolveServiceEnvironment };
