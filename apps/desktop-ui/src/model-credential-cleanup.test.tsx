import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ModelCredentialCleanup } from "./model-credential-cleanup";
import type { ModelConnection, ModelCredentialCleanupCandidate, ModelCredentialCleanupPlan } from "./generated/protocol";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
const connection: ModelConnection = {
  profileId: "personal-profile", connectionId: "personal-connection", configRevision: 2, authGeneration: 2, authScopes: [],
  config: { name: "个人连接", kind: "openai_key", baseUrl: null, localRuntime: null, authMode: "api_key", enabled: true,
    network: { mode: "direct", proxyUrl: null, proxySecretRef: null, proxyAuth: "none", serviceAuth: "none", headerNames: [] } }, secretRef: null, deleted: false, createdAt: 1, updatedAt: 2,
};
function candidate(overrides: Partial<ModelCredentialCleanupCandidate> = {}): ModelCredentialCleanupCandidate {
  return { cleanupId: "cleanup-one", profileId: connection.profileId, connectionId: connection.connectionId, authGeneration: 1,
    credentialRef: { service: "cn.cilai.personal.workbench.vault.model", account: "synthetic-opaque-reference" }, sourceOperationIds: ["old-operation"],
    deletedConnection: false, blockedReferences: [], activeLeaseIds: [], canPrepare: true, referenceDigest: "reference-digest", state: "unconfirmed", plan: null, ...overrides };
}
function plan(item: ModelCredentialCleanupCandidate, state: ModelCredentialCleanupPlan["state"]): ModelCredentialCleanupPlan {
  return { cleanupId: item.cleanupId, profileId: item.profileId, connectionId: item.connectionId, authGeneration: item.authGeneration,
    credentialRef: item.credentialRef, referenceDigest: item.referenceDigest, state, preparedAt: 3, confirmedAt: state === "confirmed" ? 4 : null };
}
let candidates: ModelCredentialCleanupCandidate[];
beforeEach(() => {
  candidates = [candidate()];
  invoke.mockReset();
  invoke.mockImplementation(async (command: string) => {
    if (command === "model_credential_cleanup_list") return structuredClone(candidates);
    throw new Error(`unexpected command ${command}`);
  });
});
afterEach(cleanup);
async function openConfirm(label = "清理") {
  const button = await screen.findByRole("button", { name: label });
  await waitFor(() => expect(button).toHaveProperty("disabled", false));
  fireEvent.click(button);
  return within(await screen.findByRole("dialog", { name: "清理历史凭据？" }));
}
const executions = () => invoke.mock.calls.filter(([command]) => command === "model_credential_cleanup_execute");

it("挂载及刷新只读本地引用，不显示凭据引用值或发起保存、登录及推理", async () => {
  candidates.push(candidate({ cleanupId: "cleanup-deleted", connectionId: "abcdefgh-missing", authGeneration: 5, deletedConnection: true }));
  render(<ModelCredentialCleanup connections={[connection]} disabled={false}/>);
  await screen.findByRole("group", { name: "个人连接 · 授权代次 1" });
  expect(screen.getByRole("group", { name: "abcdefgh · 授权代次 5" })).toBeTruthy();
  expect(screen.getByText("授权代次 5 · 连接已删除")).toBeTruthy();
  expect(document.body.textContent).not.toContain("cn.cilai.personal.workbench.vault.model");
  expect(document.body.textContent).not.toContain("synthetic-opaque-reference");
  expect(invoke.mock.calls).toEqual([["model_credential_cleanup_list", { request: {} }]]);
  fireEvent.click(screen.getByRole("button", { name: "刷新" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
  expect(invoke.mock.calls.every(([command]) => command === "model_credential_cleanup_list")).toBe(true);
});

it("活任务、活引用及不能准备的候选均拒绝清理并说明原因", async () => {
  candidates = [
    candidate({ activeLeaseIds: ["lease-one"] }),
    candidate({ cleanupId: "referenced", authGeneration: 2, blockedReferences: [{ kind: "current_credential", id: "connection" }] }),
    candidate({ cleanupId: "not-ready", authGeneration: 3, canPrepare: false }),
    candidate({ cleanupId: "current", authGeneration: 4, canPrepare: false, state: "not_required" }),
  ];
  render(<ModelCredentialCleanup connections={[connection]} disabled={false}/>);
  await screen.findByText("仍有 1 个任务占用");
  expect(screen.getByText("仍有 1 项引用")).toBeTruthy();
  expect(screen.getByText("尚不能清理，请刷新核对")).toBeTruthy();
  expect(screen.getByText("当前连接仍引用此凭据")).toBeTruthy();
  for (const button of screen.getAllByRole("button", { name: "清理" })) {
    expect(button).toHaveProperty("disabled", true); fireEvent.click(button);
  }
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(executions()).toHaveLength(0);
});

it("清理前说明保留范围，取消不执行", async () => {
  render(<ModelCredentialCleanup connections={[connection]} disabled={false}/>);
  const dialog = await openConfirm();
  expect(dialog.getByText(/仅清理此历史引用对应的工作台本地加密凭据，保留项目、会话和当前连接/)).toBeTruthy();
  expect(dialog.getByText(/是否已删除，以清理回执为准/)).toBeTruthy();
  fireEvent.click(dialog.getByRole("button", { name: "取消" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(executions()).toHaveLength(0);
  expect(invoke).toHaveBeenCalledTimes(1);
});

it("确认只提交清理标识与引用摘要，Host确认后刷新账本和父列表", async () => {
  const onChanged = vi.fn(async () => {});
  invoke.mockImplementation(async (command: string) => {
    if (command === "model_credential_cleanup_list") return structuredClone(candidates);
    if (command === "model_credential_cleanup_execute") {
      const completed = plan(candidates[0]!, "confirmed");
      candidates = [{ ...candidates[0]!, state: "confirmed", canPrepare: false, plan: completed }];
      return completed;
    }
    throw new Error(command);
  });
  render(<ModelCredentialCleanup connections={[connection]} disabled={false} onChanged={onChanged}/>);
  const dialog = await openConfirm(); fireEvent.click(dialog.getByRole("button", { name: "清理" }));
  await screen.findByText("已确认清理");
  await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
  expect(executions()).toEqual([["model_credential_cleanup_execute", { request: { cleanupId: "cleanup-one", expectedReferenceDigest: "reference-digest" } }]]);
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["model_credential_cleanup_list", "model_credential_cleanup_execute", "model_credential_cleanup_list"]);
  expect(screen.queryByRole("button", { name: /清理/ })).toBeNull();
});

it("失败后保留待清理记录，仅用户再次确认才继续，不保存连接或重发推理", async () => {
  let attempts = 0;
  const onChanged = vi.fn(async () => {});
  invoke.mockImplementation(async (command: string) => {
    if (command === "model_credential_cleanup_list") return structuredClone(candidates);
    if (command === "model_credential_cleanup_execute") {
      ++attempts;
      if (attempts === 1) {
        candidates = [{ ...candidates[0]!, state: "pending", plan: plan(candidates[0]!, "pending") }];
        throw new Error("model_credential_delete_failed");
      }
      const completed = plan(candidates[0]!, "confirmed");
      candidates = [{ ...candidates[0]!, state: "confirmed", canPrepare: false, plan: completed }];
      return completed;
    }
    throw new Error(command);
  });
  render(<ModelCredentialCleanup connections={[connection]} disabled={false} onChanged={onChanged}/>);
  const first = await openConfirm(); fireEvent.click(first.getByRole("button", { name: "清理" }));
  await screen.findByText("本地凭据未能删除，操作保持未完成。");
  await waitFor(() => expect(screen.getByRole("button", { name: "继续清理" })).toHaveProperty("disabled", false));
  expect(screen.getByText("待清理")).toBeTruthy();
  expect(onChanged).not.toHaveBeenCalled();
  expect(executions()).toHaveLength(1);
  const cancel = await openConfirm("继续清理"); fireEvent.click(cancel.getByRole("button", { name: "取消" }));
  expect(executions()).toHaveLength(1);
  const retry = await openConfirm("继续清理"); fireEvent.click(retry.getByRole("button", { name: "继续清理" }));
  await screen.findByText("已确认清理");
  await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
  expect(executions()).toHaveLength(2);
  expect(invoke.mock.calls.every(([command]) => ["model_credential_cleanup_list", "model_credential_cleanup_execute"].includes(command))).toBe(true);
});

it("Host仅返回pending时保留待清理状态，不能显示成功或自动续做", async () => {
  const onChanged = vi.fn(async () => {});
  invoke.mockImplementation(async (command: string) => {
    if (command === "model_credential_cleanup_list") return structuredClone(candidates);
    if (command === "model_credential_cleanup_execute") {
      const pending = plan(candidates[0]!, "pending");
      candidates = [{ ...candidates[0]!, state: "pending", plan: pending }];
      return pending;
    }
    throw new Error(command);
  });
  render(<ModelCredentialCleanup connections={[connection]} disabled={false} onChanged={onChanged}/>);
  const dialog = await openConfirm(); fireEvent.click(dialog.getByRole("button", { name: "清理" }));
  await screen.findByText("清理尚未确认，请核对后继续清理。");
  await waitFor(() => expect(screen.getByRole("button", { name: "继续清理" })).toHaveProperty("disabled", false));
  expect(executions()).toHaveLength(1); expect(onChanged).not.toHaveBeenCalled();
  expect(screen.queryByText("已确认清理")).toBeNull();
});

it("资料忙碌时保持只读，打开确认框后变为忙碌也不能执行", async () => {
  const view = render(<ModelCredentialCleanup connections={[connection]} disabled/>);
  await screen.findByText("待清理");
  expect(screen.getByRole("button", { name: "清理" })).toHaveProperty("disabled", true);
  view.rerender(<ModelCredentialCleanup connections={[connection]} disabled={false}/>);
  const dialog = await openConfirm();
  view.rerender(<ModelCredentialCleanup connections={[connection]} disabled/>);
  expect(dialog.getByRole("button", { name: "清理" })).toHaveProperty("disabled", true);
  fireEvent.click(dialog.getByRole("button", { name: "清理" }));
  expect(executions()).toHaveLength(0);
  expect(invoke).toHaveBeenCalledTimes(1);
});

it("刷新失败保留原列表，未确认完成时不伪造清理结果", async () => {
  render(<ModelCredentialCleanup connections={[connection]} disabled={false}/>);
  await screen.findByText("待清理");
  invoke.mockRejectedValueOnce(new Error("本地引用账本暂不可读。"));
  fireEvent.click(screen.getByRole("button", { name: "刷新" }));
  await screen.findByText("本地引用账本暂不可读。");
  expect(screen.getByRole("group", { name: "个人连接 · 授权代次 1" })).toBeTruthy();
  expect(screen.getByText("待清理")).toBeTruthy();
  expect(screen.queryByText("已确认清理")).toBeNull();
  expect(executions()).toHaveLength(0);
  expect(invoke).toHaveBeenCalledTimes(2);
});

it("返回的计划归属不匹配时明确拒绝成功状态", async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === "model_credential_cleanup_list") return structuredClone(candidates);
    if (command === "model_credential_cleanup_execute") return { ...plan(candidates[0]!, "confirmed"), connectionId: "other-connection" };
    throw new Error(command);
  });
  const onChanged = vi.fn(async () => {});
  render(<ModelCredentialCleanup connections={[connection]} disabled={false} onChanged={onChanged}/>);
  const dialog = await openConfirm(); fireEvent.click(dialog.getByRole("button", { name: "清理" }));
  await screen.findByText("凭据归属不匹配，操作已停止。");
  expect(screen.queryByText("已确认清理")).toBeNull();
  expect(onChanged).not.toHaveBeenCalled();
  expect(executions()).toHaveLength(1);
});


it.each(["cn.cilai.personal.workbench.model", "cn.cilai.personal.workbench.proxy", "cn.cilai.personal.workbench.vault.model.extra"])("旧或相似引用%s仅提示手工清理，不把历史确认当新库已删除", async (service) => {
  const item = candidate({ credentialRef: { service, account: "private-reference-do-not-render" } });
  candidates = [{ ...item, state: "confirmed", plan: plan(item, "confirmed") }];
  render(<ModelCredentialCleanup connections={[connection]} disabled={false}/>);
  await screen.findByText("旧系统凭据需手工清理，工作台未确认删除。");
  expect(screen.getByText("需手工核对")).toBeTruthy();
  expect(screen.queryByText("已确认清理")).toBeNull();
  expect(screen.queryByRole("button", { name: /清理/ })).toBeNull();
  expect(document.body.textContent).not.toContain(service);
  expect(document.body.textContent).not.toContain("private-reference-do-not-render");
  expect(executions()).toHaveLength(0);
});

it("本地引用收到旧系统清理未完成回执时不显示已删除，也不自动重试", async () => {
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === "model_credential_cleanup_execute") throw new Error("model_credential_legacy_cleanup_required");
    return original(command, args);
  });
  const onChanged = vi.fn(async () => {});
  render(<ModelCredentialCleanup connections={[connection]} disabled={false} onChanged={onChanged}/>);
  const dialog = await openConfirm(); fireEvent.click(dialog.getByRole("button", { name: "清理" }));
  await screen.findByText("旧系统凭据需手工清理，工作台未确认删除。");
  expect(screen.queryByText("已确认清理")).toBeNull();
  expect(executions()).toHaveLength(1);
  expect(onChanged).not.toHaveBeenCalled();
});
