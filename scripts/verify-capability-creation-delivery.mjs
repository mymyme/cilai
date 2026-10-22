import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import {
  access,
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  realpath,
  rm,
  stat,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { verificationEnvironment } from "./verification-environment.mjs";
import { strFromU8, unzipSync } from "fflate";

const [executable, windowsHostExecutable, environment = "dev"] = process.argv.slice(2);
if (!executable) throw new Error("请传入待验收的 contexthub-app-server 可执行文件路径");
if (!["dev", "prod"].includes(environment)) throw new Error("验收环境仅支持 dev 或 prod");

const platform = { darwin: "macos", win32: "windows" }[process.platform];
const arch = { arm64: "arm64", x64: "x86_64" }[process.arch];
if (!platform || !arch) throw new Error(`不支持的验收平台：${process.platform}/${process.arch}`);

const temporaryRoot = await realpath(await mkdtemp(join(tmpdir(), "cilai-capability-delivery-")));
const childEnvironment = verificationEnvironment(temporaryRoot, { CILAI_DATA_ENVIRONMENT: environment });
const firstBusinessRoot = join(temporaryRoot, "first", "cn.cilai.personal.workbench", "business");
let firstProfile = join(temporaryRoot, "first", environment);
const secondProfile = join(temporaryRoot, "second", environment);
const webHtml = `<!doctype html>
<html lang="zh-CN">
<head><meta charset="utf-8"><title>会议行动项</title></head>
<body><main><h1>会议行动项</h1><p>负责人：王明</p><p>截止：周五</p></main></body>
</html>`;
const mcpSource = `import readline from "node:readline";
const input = readline.createInterface({ input: process.stdin });
input.on("line", (line) => process.stdout.write(line + "\\n"));`;

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function installFingerprint(project) {
  return sha256(JSON.stringify({
    creationProjectId: project.creationProjectId,
    expectedRevision: project.stateRevision,
    packageDigest: project.packageDigest,
  }));
}

function delivery(title, businessGoal, targetType, requestedPermissions, acceptanceExamples, sourceFiles) {
  return `方案如下。\n\n\`\`\`cilai-capability-project\n${JSON.stringify({
    title,
    businessGoal,
    targetType,
    requestedPermissions,
    acceptanceExamples,
    sourceFiles,
  })}\n\`\`\``;
}

async function prepareProfile(root) {
  await mkdir(join(root, "run"), { recursive: true });
}

async function start(root) {
  await prepareProfile(root);
  const child = spawn(resolve(executable), [
    "serve",
    "--transport",
    "stdio",
    "--enable-local-ipc",
    "--environment",
    environment,
    "--data-dir",
    root,
    "--runtime-dir",
    join(root, "run"),
  ], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true, env: childEnvironment });
  let requestId = 0;
  let stderr = "";
  const pending = new Map();

  child.stderr.on("data", (chunk) => {
    stderr = `${stderr}${chunk}`.slice(-8000);
  });
  createInterface({ input: child.stdout }).on("line", (line) => {
    const message = JSON.parse(line);
    const pendingRequest = pending.get(message.id);
    if (!pendingRequest) return;
    clearTimeout(pendingRequest.timer);
    pending.delete(message.id);
    if (message.error) pendingRequest.reject(Object.assign(
      new Error(JSON.stringify(message.error)),
      { rpcError: message.error },
    ));
    else pendingRequest.resolve(message.result);
  });
  child.on("exit", (code) => {
    for (const pendingRequest of pending.values()) {
      clearTimeout(pendingRequest.timer);
      pendingRequest.reject(new Error(`服务退出 ${code}: ${stderr}`));
    }
    pending.clear();
  });

  const rpc = (method, params) => new Promise((resolveRequest, reject) => {
    const id = ++requestId;
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`请求超时 ${method}: ${stderr}`));
    }, 15000);
    pending.set(id, { resolve: resolveRequest, reject, timer });
    child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  });
  const stop = () => new Promise((resolveStop) => {
    if (child.exitCode !== null) return resolveStop();
    child.once("exit", resolveStop);
    child.stdin.end();
  });

  await rpc("initialize", {
    client: { name: "contexthub-desktop", version: "1", platform, arch },
    protocol: { minVersion: "1.0", maxVersion: "1.0" },
  });
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method: "initialized", params: {} })}\n`);
  return { rpc, stop };
}

async function expectRpcReason(action, reason) {
  await assert.rejects(action, (error) => {
    const serialized = JSON.stringify(error.rpcError ?? error.message);
    return serialized.includes(reason);
  });
}

async function windowsHostRequest(command, args) {
  assert.equal(platform, "windows");
  const child = spawn(resolve(windowsHostExecutable), ["--data-dir", firstBusinessRoot], {
    stdio: ["pipe", "pipe", "pipe"], windowsHide: true,
    env: childEnvironment,
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => { stderr = `${stderr}${chunk}`.slice(-4000); });
  try {
    const response = await new Promise((resolveResponse, reject) => {
      const timer = setTimeout(() => reject(new Error(`沙箱就绪检查超时：${stderr}`)), 30000);
      const finish = (error, value) => {
        clearTimeout(timer);
        if (error) reject(error); else resolveResponse(value);
      };
      child.once("error", (error) => finish(error));
      child.once("exit", (code) => finish(new Error(`宿主提前退出 ${code}: ${stderr}`)));
      createInterface({ input: child.stdout }).on("line", (line) => {
        const message = JSON.parse(line);
        if (message.id === 1) finish(null, message);
      });
      child.stdin.write(`${JSON.stringify({ id: 1, command, args })}\n`);
    });
    return response;
  } finally {
    if (child.exitCode === null) {
      const stopped = new Promise((done) => child.once("exit", done));
      child.kill();
      await stopped;
    }
  }
}

async function verifyWindowsSetupGate(creationProjectId) {
  const response = await windowsHostRequest("personal_capability_tools", { creation_project_id: creationProjectId });
  assert.match(response.error ?? "", /^Windows 安全沙箱(?:尚未初始化|需要更新)，请先在模型与智能体设置中完成(?:初始化|更新)。$/);
  assert.equal(response.result, undefined);
  return response.error;
}

async function waitForProjects(server, count) {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    const result = await server.rpc("capabilityCreation/list", {});
    if (result.projects.length === count) return result.projects;
    await new Promise((resolveWait) => setTimeout(resolveWait, 250));
  }
  throw new Error(`等待能力项目超时，预期 ${count} 个`);
}

let firstServer;
let secondServer;
try {
  if (windowsHostExecutable) {
    const profile = await windowsHostRequest("local_profile_status", {});
    assert.equal(profile.error, undefined);
    assert.equal(profile.result?.ready, true);
    assert.match(profile.result?.profileId ?? "", /^[0-9a-f]{32}$/);
    firstProfile = join(firstBusinessRoot, "profiles", profile.result.profileId);
  }
  firstServer = await start(firstProfile);
  const before = await firstServer.rpc("aiState/get", {});
  const state = {
    conversations: [
      { conversationId: "conversation-web" },
      { conversationId: "conversation-mcp" },
    ],
    runs: [
      {
        runId: "run-web",
        conversationId: "conversation-web",
        status: "completed",
        superseded: false,
        skillIds: ["system.capability-creator"],
        artifacts: [],
      },
      {
        runId: "run-mcp",
        conversationId: "conversation-mcp",
        status: "completed",
        superseded: false,
        skillIds: ["system.capability-creator"],
        artifacts: [],
      },
    ],
    messages: [
      {
        messageId: "reply-web",
        role: "assistant",
        runId: "run-web",
        content: delivery(
          "会议行动项",
          "把会议结论整理为可直接查看的行动项页面",
          "web_application",
          [],
          [{ input: "打开个人应用", expected: "显示负责人和截止时间" }],
          [
            { path: "web/index.html", content: webHtml },
            { path: "README.md", content: "# 会议行动项\n\n用于查看会议行动项。\n" },
          ],
        ),
      },
      {
        messageId: "reply-mcp",
        role: "assistant",
        runId: "run-mcp",
        content: delivery(
          "本地文本检查",
          "读取用户选择的文本并返回检查结果",
          "local_mcp",
          ["personal_file.read"],
          [{ input: "选择文本文件", expected: "返回检查结果且不修改文件" }],
          [{ path: "mcp/server.mjs", content: mcpSource }],
        ),
      },
    ],
  };
  await firstServer.rpc("aiState/replace", { expectedRevision: before.stateRevision, state });

  const projects = await waitForProjects(firstServer, 2);
  const webProject = projects.find((project) => project.title === "会议行动项");
  const mcpProject = projects.find((project) => project.title === "本地文本检查");
  assert.ok(webProject);
  assert.ok(mcpProject);
  assert.equal(webProject.status, "packaged");
  assert.equal(webProject.targetType, "web_application");
  assert.equal(webProject.conversationId, "conversation-web");
  assert.equal(webProject.sourceFileCount, 2);
  assert.match(webProject.sourceDigest, /^sha256:[0-9a-f]{64}$/);
  assert.match(webProject.packageDigest, /^sha256:[0-9a-f]{64}$/);
  assert.equal(mcpProject.status, "packaged");

  const preview = await firstServer.rpc("capabilityCreation/preview", {
    creationProjectId: webProject.creationProjectId,
  });
  assert.equal(preview.html, webHtml);
  assert.equal(preview.contentDigest, `sha256:${sha256(webHtml)}`);

  const installParams = {
    creationProjectId: webProject.creationProjectId,
    expectedRevision: webProject.stateRevision,
    packageDigest: webProject.packageDigest,
    clientOperationId: "verification-install-web",
    requestFingerprint: installFingerprint(webProject),
  };
  const installed = await firstServer.rpc("capabilityCreation/install", installParams);
  assert.equal(installed.replayed, false);
  assert.equal(installed.project.status, "installed");
  const replayed = await firstServer.rpc("capabilityCreation/install", installParams);
  assert.equal(replayed.replayed, true);

  const opened = await firstServer.rpc("personalApplication/open", {
    creationProjectId: webProject.creationProjectId,
  });
  assert.equal(opened.html, webHtml);
  assert.equal(opened.contentDigest, preview.contentDigest);
  assert.equal(opened.installationRevision, 1);

  const exported = await firstServer.rpc("personalCapability/export", {
    creationProjectId: webProject.creationProjectId,
  });
  assert.equal(exported.format, "cilai-personal-package");
  assert.equal(exported.formatVersion, 1);
  const exportedBytes = new Uint8Array(exported.bytes);
  assert.equal(exported.packageDigest, `sha256:${sha256(exportedBytes)}`);
  const exportedFiles = unzipSync(exportedBytes);
  const exportedNames = Object.keys(exportedFiles).sort();
  assert.deepEqual(exportedNames, [
    "generated/personal-application.json",
    "generated/plugin-manifest.json",
    "source/README.md",
    "source/web/index.html",
  ]);
  assert.equal(strFromU8(exportedFiles["source/web/index.html"]), webHtml);
  const exportedManifest = JSON.parse(strFromU8(exportedFiles["generated/plugin-manifest.json"]));
  assert.equal(exportedManifest.distributionScope, "personal");
  for (const field of ["tenantId", "membershipId", "enterpriseBinding", "enterpriseReview", "reviewState", "publicationState", "credentials"]) {
    assert.equal(exportedManifest[field], undefined);
  }

  const installedMcp = await firstServer.rpc("capabilityCreation/install", {
      creationProjectId: mcpProject.creationProjectId,
      expectedRevision: mcpProject.stateRevision,
      packageDigest: mcpProject.packageDigest,
      clientOperationId: "verification-install-mcp",
      requestFingerprint: installFingerprint(mcpProject),
    });
  assert.equal(installedMcp.project.status, "installed");
  const mcpParams = { creationProjectId: mcpProject.creationProjectId };
  const runtime = await firstServer.rpc("personalCapability/runtime", mcpParams);
  assert.equal(runtime.packageDigest, mcpProject.packageDigest);
  assert.equal(await readFile(join(runtime.packageRoot, "source/mcp/server.mjs"), "utf8"), mcpSource);
  const disabled = await firstServer.rpc("personalCapability/manage", {
    ...mcpParams, packageDigest: mcpProject.packageDigest,
    expectedRevision: installedMcp.project.stateRevision,
    action: "disable", clientOperationId: "verification-disable-mcp",
  });
  assert.equal(disabled.enabled, false);
  await expectRpcReason(firstServer.rpc("personalCapability/runtime", mcpParams), "personal_capability_not_ready");

  const projectRoot = join(firstProfile, webProject.managedRelativePath);
  const installRoot = join(
    firstProfile,
    "personal-capabilities",
    webProject.creationProjectId,
    webProject.packageDigest.slice("sha256:".length),
  );
  const artifactNames = await readdir(join(projectRoot, "artifacts"));
  assert.deepEqual(artifactNames, [`${webProject.packageDigest.slice("sha256:".length)}.zip`]);
  assert.equal((await stat(join(projectRoot, "source", "web", "index.html"))).isFile(), true);
  assert.equal((await stat(join(installRoot, "source", "web", "index.html"))).isFile(), true);
  assert.equal((await stat(join(installRoot, "generated", "plugin-manifest.json"))).isFile(), true);
  assert.equal(await readFile(join(installRoot, "source", "web", "index.html"), "utf8"), webHtml);

  await firstServer.stop();
  firstServer = await start(firstProfile);
  const restartedProjects = await firstServer.rpc("capabilityCreation/list", {});
  assert.equal(restartedProjects.projects.length, 2);
  const reopened = await firstServer.rpc("personalApplication/open", {
    creationProjectId: webProject.creationProjectId,
  });
  assert.equal(reopened.contentDigest, preview.contentDigest);
  const restartedMcp = await firstServer.rpc("personalCapability/status", mcpParams);
  assert.equal(restartedMcp.enabled, false);
  await expectRpcReason(firstServer.rpc("personalCapability/runtime", mcpParams), "personal_capability_not_ready");
  const enabled = await firstServer.rpc("personalCapability/manage", {
    ...mcpParams, packageDigest: mcpProject.packageDigest,
    expectedRevision: restartedMcp.project.stateRevision,
    action: "enable", clientOperationId: "verification-enable-mcp",
  });
  assert.equal(enabled.enabled, true);
  assert.equal((await firstServer.rpc("personalCapability/runtime", mcpParams)).packageDigest, runtime.packageDigest);
  if (windowsHostExecutable) {
    // The delivery fixture deliberately uses minimal conversation records. Restore the original
    // empty runtime state before the actual Host deserializes it; installed capabilities persist.
    const current = await firstServer.rpc("aiState/get", {});
    await firstServer.rpc("aiState/replace", { expectedRevision: current.stateRevision, state: before.state });
  }
  await firstServer.stop();
  firstServer = null;

  const windowsSetupGate = windowsHostExecutable
    ? await verifyWindowsSetupGate(mcpProject.creationProjectId) : null;

  secondServer = await start(secondProfile);
  assert.deepEqual((await secondServer.rpc("capabilityCreation/list", {})).projects, []);
  await expectRpcReason(
    secondServer.rpc("personalApplication/open", {
      creationProjectId: webProject.creationProjectId,
    }),
    "capability_creation_project_not_found",
  );

  await access(join(firstProfile, "contexthub.db"));
  const report = {
    status: "passed",
    executable: resolve(executable),
    executableName: basename(executable),
    platform,
    arch,
    conversationDeliveryCreatedProjects: 2,
    webProjectStatus: installed.project.status,
    previewDigest: preview.contentDigest,
    packageDigest: webProject.packageDigest,
    installationRevision: opened.installationRevision,
    installReplayVerified: replayed.replayed,
    mcpInstalled: true,
    mcpDisabledAcrossRestart: true,
    mcpReenabled: true,
    windowsSetupGateVerified: Boolean(windowsHostExecutable),
    windowsSetupGateMessage: windowsSetupGate,
    restartPreserved: true,
    profileIsolationVerified: true,
    managedProjectFilesVerified: true,
    installedFilesVerified: true,
    personalPackageExportVerified: true,
    exportedFileNames: exportedNames,
    evidenceScope: "本地交付与持久化集成；模型回复为协议夹具，未验证真实模型生成",
  };
  console.log(JSON.stringify(report, null, 2));
} finally {
  await firstServer?.stop();
  await secondServer?.stop();
  await rm(temporaryRoot, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
}
