import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ExportDiagnosticsButton, TestDiagnosticsSettings } from "./test-diagnostics";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
afterEach(cleanup);
const preview = { previewId: "8e294723-4cc7-44c0-a374-932e38d7a99b", digest: "a".repeat(64), expiresAt: Date.now() + 600000,
  bytes: 42, text: "仅包含错误分类与时间。分类 网络；未发现真实模型验收。" };
beforeEach(() => { invoke.mockReset(); });
it("reads persisted logging switch and only changes it on click", async () => {
  invoke.mockResolvedValueOnce({enabled:true,error:null,maxBytes:1048576}).mockResolvedValueOnce({enabled:false,error:null,maxBytes:1048576});
  render(<TestDiagnosticsSettings/>);
  const toggle = screen.getByRole("switch",{name:"记录错误"});
  await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("true"));
  expect(invoke).toHaveBeenCalledTimes(1);
  fireEvent.click(toggle);
  await waitFor(() => expect(toggle.getAttribute("aria-checked")).toBe("false"));
  expect(invoke).toHaveBeenLastCalledWith("test_diagnostics_set_enabled",{enabled:false});
});
it("startup diagnostics preview requires explicit confirmation before saving its exact receipt", async () => {
  invoke.mockResolvedValueOnce(preview).mockResolvedValueOnce(true);
  render(<ExportDiagnosticsButton/>);
  fireEvent.click(screen.getByRole("button",{name:"导出诊断"}));
  const dialog = await screen.findByRole("dialog", {name:"诊断预览"});
  expect(dialog.textContent).toContain(preview.text);
  expect(invoke.mock.calls.map(call => call[0])).toEqual(["test_diagnostics_preview"]);
  fireEvent.click(screen.getByRole("button", {name:"选择保存位置"}));
  await screen.findByText("诊断已导出。");
  expect(invoke).toHaveBeenLastCalledWith("test_diagnostics_export", {previewId:preview.previewId,digest:preview.digest});
});
it("cancelled preview never invokes file export", async () => {
  invoke.mockResolvedValue(preview);
  render(<ExportDiagnosticsButton/>);
  fireEvent.click(screen.getByRole("button",{name:"导出诊断"}));
  fireEvent.click(await screen.findByRole("button",{name:"取消"}));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(invoke).toHaveBeenCalledTimes(1);
});
it("cancelled save keeps the preview and does not show success or recollect", async () => {
  invoke.mockResolvedValueOnce(preview).mockResolvedValueOnce(false);
  render(<ExportDiagnosticsButton/>);
  fireEvent.click(screen.getByRole("button",{name:"导出诊断"}));
  fireEvent.click(await screen.findByRole("button",{name:"选择保存位置"}));
  await waitFor(() => expect((screen.getByRole("button",{name:"选择保存位置"}) as HTMLButtonElement).disabled).toBe(false));
  expect(screen.queryByText("诊断已导出。")).toBeNull();
  expect(screen.getByRole("dialog").textContent).toContain(preview.text);
  expect(invoke).toHaveBeenCalledTimes(2);
});
it("export failure does not change recording preferences or automatically retry", async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === "test_diagnostics_settings") return {enabled:true,error:null,maxBytes:1048576};
    if (command === "test_diagnostics_preview") return preview;
    throw new Error("诊断文件未能保存。");
  });
  render(<TestDiagnosticsSettings/>);
  await waitFor(() => expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("true"));
  fireEvent.click(screen.getByRole("button", {name:"导出诊断"}));
  fireEvent.click(await screen.findByRole("button",{name:"选择保存位置"}));
  expect((await screen.findByRole("alert")).textContent).toContain("诊断文件未能保存");
  expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("true");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(invoke.mock.calls.filter(call => call[0] === "test_diagnostics_export")).toHaveLength(1);
});
it("invalid preview cannot be exported", async () => {
  invoke.mockResolvedValue({...preview,digest:"invalid"});
  render(<ExportDiagnosticsButton/>);
  fireEvent.click(screen.getByRole("button",{name:"导出诊断"}));
  expect((await screen.findByRole("alert")).textContent).toContain("诊断预览无效");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(invoke).toHaveBeenCalledTimes(1);
});
it("settings read failure has an explicit retry without changing the persisted switch", async () => {
  invoke.mockRejectedValueOnce(new Error("日志设置读取失败")).mockResolvedValueOnce({enabled:false,error:null,maxBytes:1048576});
  render(<TestDiagnosticsSettings/>);
  await screen.findByRole("alert");
  expect((screen.getByRole("switch") as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole("button", {name:"重新读取"}));
  await waitFor(() => expect((screen.getByRole("switch") as HTMLButtonElement).disabled).toBe(false));
  expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("false");
  expect(screen.queryByRole("alert")).toBeNull();
  expect(invoke.mock.calls.map(call => call[0])).toEqual(["test_diagnostics_settings","test_diagnostics_settings"]);
});
