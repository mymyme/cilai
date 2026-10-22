import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { loadAiSnapshot, aiRuntimeChangedEvent, emptyAiSnapshot, type AiModelInfo } from "./ai-runtime";
import { ModelCatalogNotice, ModelCatalogProvider, useModelCatalog, type ModelCatalogUpdate } from "./use-model-catalog";

const host = vi.hoisted(() => ({ invoke: vi.fn(), handler: null as null | ((update: ModelCatalogUpdate) => void) }));
vi.mock("./host", () => ({
  invoke: host.invoke,
  listen: async (_event: string, handler: (update: ModelCatalogUpdate) => void) => {
    host.handler = handler;
    return () => { if (host.handler === handler) host.handler = null; };
  },
}));
afterEach(() => { cleanup(); host.invoke.mockReset(); host.handler = null; vi.useRealTimers(); });
const wrapper = ({ children }: { children: ReactNode }) => <ModelCatalogProvider profileId="profile-1">{children}</ModelCatalogProvider>;
function update(ids = ["model-1"], revision = 1, options: Partial<ModelCatalogUpdate["catalog"]> = {}): ModelCatalogUpdate {
  return { profileId: "profile-1", catalog: {
    generation: 1, revision, checkedAt: Date.now(), revalidateAfter: Date.now() + 300_000, error: null,
    status: { configured: true, models: ids.map((modelId) => ({ modelId, label: modelId }) as AiModelInfo) },
    ...options,
  } };
}

test("模型请求等待时本地快照独立完成，首次加载有明确状态", async () => {
  let complete!: (value: ModelCatalogUpdate) => void;
  host.invoke.mockImplementation((command: string) => {
    if (command === "ai_runtime_snapshot") return Promise.resolve(emptyAiSnapshot);
    if (command === "ai_model_catalog_read") return new Promise((resolve) => { complete = resolve; });
    throw new Error(command);
  });
  const { result } = renderHook(() => useModelCatalog(true), { wrapper });
  await waitFor(() => expect(result.current.loading).toBe(true));
  expect(await loadAiSnapshot()).toEqual(emptyAiSnapshot);
  await act(async () => complete(update()));
  expect(result.current.loaded).toBe(true);
  expect(result.current.loading).toBe(false);
});

test("多个消费者和页面切换共享目录，聚焦与普通对话事件不重复请求", async () => {
  host.invoke.mockResolvedValue(update());
  const Probe = ({ name }: { name: string }) => { const value = useModelCatalog(true); return <span>{name}:{value.models.length}</span>; };
  const { rerender } = render(<ModelCatalogProvider profileId="profile-1"><Probe name="对话"/><Probe name="自动化"/></ModelCatalogProvider>);
  await screen.findByText("对话:1");
  const calls = host.invoke.mock.calls.length;
  rerender(<ModelCatalogProvider profileId="profile-1"><Probe name="设置"/></ModelCatalogProvider>);
  await act(async () => {
    window.dispatchEvent(new Event("focus"));
    document.dispatchEvent(new Event("visibilitychange"));
    window.dispatchEvent(new Event(aiRuntimeChangedEvent));
  });
  expect(screen.getByText("设置:1")).toBeTruthy();
  expect(host.invoke).toHaveBeenCalledTimes(calls);
});

test("主动刷新保持名称、加载状态和未变化的列表引用", async () => {
  host.invoke.mockResolvedValue(update());
  const { result } = renderHook(() => useModelCatalog(true), { wrapper });
  await waitFor(() => expect(result.current.loaded).toBe(true));
  const models = result.current.models;
  let complete!: (value: ModelCatalogUpdate) => void;
  host.invoke.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  let pending!: Promise<void>;
  act(() => { pending = result.current.refresh(); });
  expect(result.current.loading).toBe(false);
  expect(result.current.models).toBe(models);
  await act(async () => { complete(update(["model-1"], 2)); await pending; });
  expect(result.current.models).toBe(models);
  act(() => host.handler?.(update(["new-name"], 3)));
  expect(result.current.models[0].label).toBe("new-name");
});

test("多个刷新入口共用同一请求，空目录仍在有效期内", async () => {
  host.invoke.mockResolvedValue(update([]));
  const { result } = renderHook(() => useModelCatalog(true), { wrapper });
  await waitFor(() => expect(result.current.loaded).toBe(true));
  const calls = host.invoke.mock.calls.length;
  act(() => { window.dispatchEvent(new Event("focus")); });
  expect(host.invoke).toHaveBeenCalledTimes(calls);
  let complete!: (value: ModelCatalogUpdate) => void;
  host.invoke.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  let first!: Promise<void>, second!: Promise<void>;
  act(() => { first = result.current.refresh(); second = result.current.refresh(); });
  expect(first).toBe(second);
  expect(host.invoke).toHaveBeenCalledTimes(calls + 1);
  await act(async () => { complete(update([], 2)); await first; });
});

test("刷新失败清除可用集合但保留名称，旧响应不能覆盖失败，重试可恢复", async () => {
  host.invoke.mockResolvedValue(update());
  const { result } = renderHook(() => useModelCatalog(true), { wrapper });
  await waitFor(() => expect(result.current.loaded).toBe(true));
  let complete!: (value: ModelCatalogUpdate) => void;
  host.invoke.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  let pending!: Promise<void>;
  act(() => { pending = result.current.refresh(); });
  act(() => host.handler?.(update([], 3, { status: null, error: "model_connection_disabled", checkedAt: null, revalidateAfter: null })));
  expect(result.current.models).toEqual([]);
  expect(result.current.displayModels[0].label).toBe("model-1");
  expect(result.current.error).toContain("此连接已停用");
  await act(async () => { complete(update(["stale"], 2)); await pending; });
  expect(result.current.models).toEqual([]);
  host.invoke.mockResolvedValueOnce(update(["restored"], 4));
  await act(async () => result.current.refresh());
  expect(result.current.models[0].modelId).toBe("restored");
  expect(result.current.error).toBeNull();
});

test("长时间闲置、聚焦和页面切换不刷新，不因校验期限经过而清空目录", async () => {
  vi.useFakeTimers();
  host.invoke.mockResolvedValue(update());
  const { result, rerender } = renderHook(() => useModelCatalog(true), { wrapper });
  await act(async () => {});
  const calls = host.invoke.mock.calls.length;
  const models = result.current.models;
  await act(async () => { await vi.advanceTimersByTimeAsync(86_400_000); });
  act(() => {
    window.dispatchEvent(new Event("focus"));
    document.dispatchEvent(new Event("visibilitychange"));
    window.dispatchEvent(new Event(aiRuntimeChangedEvent));
  });
  rerender();
  expect(host.invoke).toHaveBeenCalledTimes(calls);
  expect(result.current.loaded).toBe(true);
  expect(result.current.models).toBe(models);
  expect(result.current.error).toBeNull();
  let complete!: (value: ModelCatalogUpdate) => void;
  host.invoke.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  act(() => { result.current.refreshOnOpen(); result.current.refreshOnOpen(); });
  expect(host.invoke).toHaveBeenCalledTimes(calls + 1);
  expect(host.invoke).toHaveBeenLastCalledWith("ai_model_catalog_read", { reason: "picker" });
  expect(result.current.loading).toBe(false);
  expect(result.current.models).toBe(models);
  await act(async () => complete(update(["current"], 2)));
  expect(result.current.models[0].modelId).toBe("current");
});

test("打开选择器会主动更新未过期目录", async () => {
  host.invoke.mockResolvedValue(update());
  const { result } = renderHook(() => useModelCatalog(true), { wrapper });
  await waitFor(() => expect(result.current.loaded).toBe(true));
  host.invoke.mockResolvedValueOnce(update(["new-model"], 2));
  await act(async () => result.current.refreshOnOpen());
  expect(host.invoke).toHaveBeenLastCalledWith("ai_model_catalog_read", { reason: "picker" });
  expect(result.current.models[0].modelId).toBe("new-model");
});

test("会话启动主动读取目录，无需先进入模型页面", async () => {
  host.invoke.mockResolvedValue(update());
  render(<ModelCatalogProvider profileId="profile-1"><div>项目</div></ModelCatalogProvider>);
  await waitFor(() => expect(host.invoke).toHaveBeenCalledWith("ai_model_catalog_read", { reason: "startup" }));
});

test("企业切换丢弃旧请求及旧通知", async () => {
  let complete!: (value: ModelCatalogUpdate) => void;
  host.invoke.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  const Probe = () => { const value = useModelCatalog(true); return <span>{value.models.map((model) => model.modelId).join(",") || "无目录"}</span>; };
  const { rerender } = render(<ModelCatalogProvider key="one" profileId="profile-1"><Probe/></ModelCatalogProvider>);
  await act(async () => {});
  host.invoke.mockResolvedValue({ ...update(["company-two"]), profileId: "profile-2" });
  rerender(<ModelCatalogProvider key="two" profileId="profile-2"><Probe/></ModelCatalogProvider>);
  await screen.findByText("company-two");
  await act(async () => complete(update(["company-one"])));
  act(() => host.handler?.(update(["company-one"], 99)));
  expect(screen.getByText("company-two")).toBeTruthy();
});

test("目录提醒保留完整原因与重试", () => {
  const refresh = vi.fn().mockResolvedValue(undefined);
  const catalog = { error: "当前成员暂无此连接已停用。", refresh };
  const { rerender } = render(<ModelCatalogNotice catalog={catalog} variant="conversation"/>);
  expect(screen.getByRole("alert").textContent).toContain(catalog.error);
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  expect(refresh).toHaveBeenCalledTimes(1);
  rerender(<ModelCatalogNotice catalog={{ ...catalog, error: null }}/>);
  expect(screen.queryByRole("alert")).toBeNull();
});


test("企业会话等待就绪期间不请求，就绪后主动加载", async () => {
  host.invoke.mockResolvedValue(update());
  const Probe = () => { const value = useModelCatalog(true); return <span>{value.loaded ? "已就绪" : "等待"}</span>; };
  const { rerender } = render(<ModelCatalogProvider profileId="profile-1" enabled={false}><Probe/></ModelCatalogProvider>);
  await act(async () => {});
  expect(host.invoke).not.toHaveBeenCalled();
  rerender(<ModelCatalogProvider profileId="profile-1" enabled><Probe/></ModelCatalogProvider>);
  await screen.findByText("已就绪");
  expect(host.invoke).toHaveBeenCalledTimes(1);
});

test("首次读取期间的更新按身份代次与修订合并", async () => {
  let complete!: (value: ModelCatalogUpdate) => void;
  host.invoke.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  const { result } = renderHook(() => useModelCatalog(true), { wrapper });
  await act(async () => {});
  act(() => {
    host.handler?.(update(["旧身份目录"], 99, { generation: 0 }));
    host.handler?.(update(["最新目录"], 2));
  });
  expect(result.current.models).toEqual([]);
  await act(async () => complete(update(["初次响应"], 1)));
  expect(result.current.models[0].modelId).toBe("最新目录");
});
