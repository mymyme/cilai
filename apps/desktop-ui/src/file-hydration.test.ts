import { afterEach, expect, test, vi } from "vitest";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
import { hydrateFileForRead } from "./file-hydration";

afterEach(() => { vi.useRealTimers(); invoke.mockReset(); });

test("本地文件直接就绪，占位文件按同一下载操作等待", async () => {
  invoke.mockResolvedValueOnce({ state: "ready" });
  expect(await hydrateFileForRead("ws", "本地.txt", new AbortController().signal, vi.fn())).toBe(false);
  vi.useFakeTimers();
  invoke.mockResolvedValueOnce({ state: "downloading", operationId: "download" }).mockResolvedValueOnce({ state: "ready", operationId: "download" });
  const waiting = vi.fn();
  const pending = hydrateFileForRead("ws", "占位.txt", new AbortController().signal, waiting);
  await vi.advanceTimersByTimeAsync(250);
  expect(await pending).toBe(true);
  expect(waiting).toHaveBeenCalledOnce();
  expect(invoke).toHaveBeenLastCalledWith("app_server_request", { method: "file/hydrate", params: { workspaceId: "ws", relativePath: "占位.txt", operationId: "download" } });
});

test("取消等待时取消原下载操作，不再轮询", async () => {
  invoke.mockResolvedValue({ state: "downloading", operationId: "download" });
  const controller = new AbortController();
  const pending = hydrateFileForRead("ws", "占位.txt", controller.signal, () => controller.abort());
  await expect(pending).rejects.toMatchObject({ name: "AbortError" });
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(invoke).toHaveBeenLastCalledWith("app_server_request", { method: "file/hydrate", params: { workspaceId: "ws", relativePath: "占位.txt", operationId: "download", cancel: true } });
});

test("首个请求迟到时仍取消其操作，不通知正文读取", async () => {
  let finish!: (result: unknown) => void;
  invoke.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; })).mockResolvedValue({ state: "cancelled" });
  const controller = new AbortController(); const waiting = vi.fn();
  const pending = hydrateFileForRead("ws", "占位.txt", controller.signal, waiting);
  controller.abort(); finish({ state: "downloading", operationId: "late" });
  await expect(pending).rejects.toMatchObject({ name: "AbortError" });
  expect(waiting).not.toHaveBeenCalled();
  expect(invoke).toHaveBeenLastCalledWith("app_server_request", { method: "file/hydrate", params: { workspaceId: "ws", relativePath: "占位.txt", operationId: "late", cancel: true } });
});

test("下载期间源文件变化明确失败", async () => {
  vi.useFakeTimers();
  invoke.mockResolvedValueOnce({ state: "downloading", operationId: "download" }).mockRejectedValueOnce(new Error("file_hydration_source_changed"));
  const pending = hydrateFileForRead("ws", "占位.txt", new AbortController().signal, vi.fn());
  const assertion = expect(pending).rejects.toThrow("file_hydration_source_changed");
  await vi.advanceTimersByTimeAsync(250);
  await assertion;
});
