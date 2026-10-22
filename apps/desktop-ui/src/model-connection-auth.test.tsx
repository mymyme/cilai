import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ModelConnectionAuth } from "./model-connection-auth";
import type { ModelRuntimeKind, RootAuthChange } from "./generated/protocol";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
let changes: RootAuthChange[];
let accounts: Record<ModelRuntimeKind, boolean>;
function operation(intent: "chatgpt_login" | "chatgpt_logout", kind: ModelRuntimeKind | null = "workbench"): RootAuthChange {
  return { operationId: "synthetic-operation", profileId: "personal-profile", connectionId: "personal-one", intent,
    state: "in_progress", expectedConfigRevision: 1, targetGeneration: 2, runtimeKind: kind, oldScopes: [],
    steps: [{ runtimeKind: kind ?? "workbench", authGeneration: 2, state: "started", loginId: "synthetic-login-id", reason: null }],
    owner: { instanceId: "test-owner", pid: 1, startFingerprint: "test-fingerprint" }, instanceFence: "test-fence", launchProtocol: 1, processes: [], updatedAt: 1 };
}
beforeEach(() => {
  changes = []; accounts = { workbench: false, application: false };
  invoke.mockReset(); invoke.mockImplementation(async (command: string, args: { request?: { runtimeKind: ModelRuntimeKind; operationId?: string } }) => {
    const kind = args.request?.runtimeKind ?? "workbench";
    if (command === "model_connection_auth_changes") return changes;
    if (command === "model_connection_auth_read") {
      const change = changes.find((item) => item.intent === "chatgpt_login" && item.runtimeKind === kind);
      return { runtimeKind: kind, account: accounts[kind] ? { type: "chatgpt" } : null,
        pendingLoginId: change?.steps[0]?.loginId ?? null, operationId: change?.operationId,
        loginStatus: change ? "pending" : accounts[kind] ? "authenticated" : "signed_out" };
    }
    if (command === "model_connection_auth_start") {
      changes = [operation("chatgpt_login", kind)];
      return { operationId: "synthetic-operation", runtimeKind: kind, loginId: "synthetic-login-id", verificationUrl: "https://auth.openai.com/codex/device", userCode: "TEST-CODE" };
    }
    if (command === "model_connection_auth_cancel" || command === "model_connection_auth_cancel_readonly") { changes = []; return {}; }
    if (command === "model_connection_auth_logout" || command === "model_connection_auth_finish_logout") {
      accounts = { workbench: false, application: false }; changes = []; return { status: "signed_out", affectsAllRuntimeKinds: true };
    }
    if (command === "model_connection_auth_resume") return { operation: changes[0], replayed: false };
    if (command === "model_connection_auth_open") return {};
    throw new Error(command);
  });
});
afterEach(cleanup);

it("进入设置只读两类隔离授权和未完成操作，不自动发起登录或打开网页", async () => {
  render(<ModelConnectionAuth connectionId="personal-one" disabled={false}/>);
  await waitFor(() => expect(screen.getAllByText("未授权")).toHaveLength(2));
  expect(invoke.mock.calls.filter(([command]) => command === "model_connection_auth_read").map(([, args]) => args.request.runtimeKind)).toEqual(["workbench", "application"]);
  expect(invoke.mock.calls.every(([command]) => ["model_connection_auth_read", "model_connection_auth_changes"].includes(command))).toBe(true);
});

it("显式开始后展示设备码，刷新元数据保留设备码，只有用户打开页面时才调用浏览器", async () => {
  render(<ModelConnectionAuth connectionId="personal-one" disabled={false}/>);
  const group = within(screen.getByRole("group", { name: "主对话授权" }));
  await waitFor(() => expect(group.getByRole("button", { name: "开始授权" })).toHaveProperty("disabled", false));
  fireEvent.click(group.getByRole("button", { name: "开始授权" }));
  expect((await group.findByLabelText("授权码")).textContent).toBe("TEST-CODE");
  await waitFor(() => expect(group.getByRole("button", { name: "刷新状态" })).toHaveProperty("disabled", false));
  fireEvent.click(group.getByRole("button", { name: "刷新状态" }));
  await waitFor(() => expect(group.getByRole("button", { name: "打开授权页" })).toHaveProperty("disabled", false));
  expect(group.getByLabelText("授权码").textContent).toBe("TEST-CODE");
  expect(invoke.mock.calls.some(([command]) => command === "model_connection_auth_open")).toBe(false);
  fireEvent.click(group.getByRole("button", { name: "打开授权页" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("model_connection_auth_open", { url: "https://auth.openai.com/codex/device" }));
  expect(within(screen.getByRole("group", { name: "页面授权" })).queryByLabelText("授权码")).toBeNull();
});

it("取消只处理选定运行环境的登录，不重复开始另一授权", async () => {
  render(<ModelConnectionAuth connectionId="personal-one" disabled={false}/>);
  const group = within(screen.getByRole("group", { name: "页面授权" }));
  await waitFor(() => expect(group.getByRole("button", { name: "开始授权" })).toHaveProperty("disabled", false));
  fireEvent.click(group.getByRole("button", { name: "开始授权" })); await group.findByLabelText("授权码");
  await waitFor(() => expect(group.getByRole("button", { name: "取消授权" })).toHaveProperty("disabled", false));
  fireEvent.click(group.getByRole("button", { name: "取消授权" }));
  await waitFor(() => expect(group.queryByLabelText("授权码")).toBeNull());
  expect(invoke).toHaveBeenCalledWith("model_connection_auth_cancel", { request: { connectionId: "personal-one", runtimeKind: "application", loginId: "synthetic-login-id", operationId: "synthetic-operation" } });
  expect(invoke.mock.calls.filter(([command]) => command === "model_connection_auth_start")).toHaveLength(1);
});

it("授权状态失败时显示错误，不把缓存状态视为已授权", async () => {
  invoke.mockRejectedValue(new Error("model_auth_state_unavailable"));
  render(<ModelConnectionAuth connectionId="personal-one" disabled={false}/>);
  await waitFor(() => expect(screen.getAllByRole("alert").length).toBeGreaterThanOrEqual(2));
  expect(screen.queryByText("已授权")).toBeNull();
  expect(invoke.mock.calls.some(([command]) => command === "model_connection_auth_start")).toBe(false);
});

it("退出先说明影响全部环境，取消不退出，确认后同时刷新主对话和页面状态", async () => {
  accounts = { workbench: true, application: true };
  render(<ModelConnectionAuth connectionId="personal-one" disabled={false}/>);
  await waitFor(() => expect(screen.getAllByText("已授权")).toHaveLength(2));
  const group = within(screen.getByRole("group", { name: "页面授权" }));
  fireEvent.click(group.getByRole("button", { name: "退出授权" }));
  let dialog = within(await screen.findByRole("dialog"));
  expect(dialog.getByText(/主对话、页面 AI 及先前授权/)).toBeTruthy();
  fireEvent.click(dialog.getByRole("button", { name: "取消" }));
  expect(invoke.mock.calls.some(([command]) => command === "model_connection_auth_logout")).toBe(false);
  fireEvent.click(group.getByRole("button", { name: "退出授权" }));
  dialog = within(await screen.findByRole("dialog")); fireEvent.click(dialog.getByRole("button", { name: "退出" }));
  await waitFor(() => expect(screen.getAllByText("未授权")).toHaveLength(2));
  expect(invoke.mock.calls.filter(([command]) => command === "model_connection_auth_logout")).toHaveLength(1);
  expect(invoke.mock.calls.filter(([command]) => command === "model_connection_auth_read").slice(-2).map(([, args]) => args.request.runtimeKind)).toEqual(["workbench", "application"]);
});

it("中断退出的核对只读，继续退出需要明确确认", async () => {
  changes = [{ ...operation("chatgpt_logout", null), state: "uncertain" }];
  render(<ModelConnectionAuth connectionId="personal-one" disabled={false}/>);
  await screen.findByText("退出尚未完成");
  fireEvent.click(screen.getByRole("button", { name: "核对状态" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("model_connection_auth_resume", { request: { connectionId: "personal-one", operationId: "synthetic-operation" } }));
  expect(invoke.mock.calls.some(([command]) => command === "model_connection_auth_finish_logout" || command === "model_connection_auth_logout")).toBe(false);
  await waitFor(() => expect(screen.getByRole("button", { name: "继续退出" })).toHaveProperty("disabled", false));
  fireEvent.click(screen.getByRole("button", { name: "继续退出" }));
  fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "继续退出" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("model_connection_auth_finish_logout", { request: { connectionId: "personal-one", operationId: "synthetic-operation" } }));
});

it("重启后没有登录句柄仍保留操作核对和受控取消入口", async () => {
  const change = operation("chatgpt_login", "application");
  changes = [{ ...change, state: "uncertain", steps: [{ ...change.steps[0], loginId: null, state: "pending" }] }];
  render(<ModelConnectionAuth connectionId="personal-one" disabled={false}/>);
  const group = within(screen.getByRole("group", { name: "页面授权" }));
  await waitFor(() => expect(group.getByRole("button", { name: "取消授权" })).toHaveProperty("disabled", false));
  fireEvent.click(group.getByRole("button", { name: "取消授权" }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("model_connection_auth_cancel", { request: { connectionId: "personal-one", runtimeKind: "application", operationId: "synthetic-operation" } }));
  expect(invoke.mock.calls.some(([command]) => command === "model_connection_auth_start")).toBe(false);
});

it("中断只读操作可结束，核对及结束均不自动重发账号读取或退出", async () => {
  changes = [{ ...operation("chatgpt_login"), intent: "chatgpt_status", state: "uncertain" }];
  render(<ModelConnectionAuth connectionId="personal-one" disabled={false}/>);
  await screen.findByText("授权状态核对尚未完成");
  const reads = invoke.mock.calls.filter(([command]) => command === "model_connection_auth_read").length;
  fireEvent.click(screen.getByRole("button", { name: "核对状态" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "结束未完成核对" })).toHaveProperty("disabled", false));
  expect(invoke.mock.calls.filter(([command]) => command === "model_connection_auth_read")).toHaveLength(reads);
  fireEvent.click(screen.getByRole("button", { name: "结束未完成核对" }));
  await waitFor(() => expect(screen.queryByText("授权状态核对尚未完成")).toBeNull());
  expect(invoke).toHaveBeenCalledWith("model_connection_auth_cancel_readonly", { request: { connectionId: "personal-one", operationId: "synthetic-operation" } });
  expect(invoke.mock.calls.filter(([command]) => command === "model_connection_auth_read")).toHaveLength(reads);
  expect(invoke.mock.calls.some(([command]) => ["model_connection_auth_start", "model_connection_auth_logout", "model_connection_auth_finish_logout"].includes(command))).toBe(false);
});

it("状态读取期间的临时操作完成后消失，不把旧列表留成永久授权阻塞", async () => {
  const transient = { ...operation("chatgpt_login"), intent: "chatgpt_status" as const };
  changes = [transient];
  const base = invoke.getMockImplementation()!;
  let release!: () => void;
  const wait = new Promise<void>((resolve) => { release = resolve; });
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === "model_connection_auth_read") { await wait; changes = []; }
    return base(command, args);
  });
  render(<ModelConnectionAuth connectionId="personal-one" disabled={false}/>);
  await screen.findByText("授权状态核对尚未完成");
  release();
  await waitFor(() => expect(screen.queryByText("授权状态核对尚未完成")).toBeNull());
  const group = within(screen.getByRole("group", { name: "主对话授权" }));
  await waitFor(() => expect(group.getByRole("button", { name: "开始授权" })).toHaveProperty("disabled", false));
  expect(invoke.mock.calls.filter(([command]) => command === "model_connection_auth_read")).toHaveLength(2);
  expect(invoke.mock.calls.some(([command]) => command === "model_connection_auth_start")).toBe(false);
});
