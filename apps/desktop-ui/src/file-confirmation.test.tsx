import { afterEach, expect, test, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { FileConfirmationDialog } from "./file-confirmation";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const props = { baselineVersionId:null, checksumAlgorithm:"sha256", checksumValue:"a".repeat(64), localFileId:"file", workspaceId:"workspace", relativePath:"方案.md", onCancel:vi.fn(), onConfirmed:vi.fn(), onMessage:vi.fn() };
test("人工确认只保存本次本地内容核对，不调用同步", async () => {
  invoke.mockResolvedValue({ confirmationId:"confirmation", replayed:false });
  render(<FileConfirmationDialog {...props}/>);
  expect(screen.queryByRole("button", {name:"确认并提交"})).toBeNull();
  fireEvent.click(screen.getByRole("button", {name:"确认"}));
  await waitFor(() => expect(props.onConfirmed).toHaveBeenCalledOnce());
  expect(invoke).toHaveBeenCalledExactlyOnceWith("file_confirm", {params:expect.objectContaining({localFileId:"file",expectedChecksumValue:"a".repeat(64)})});
  expect(props.onMessage).toHaveBeenCalledWith("本次文件内容已确认。");
});
test("外部修改拒绝确认并保留弹窗", async () => {
  invoke.mockRejectedValue(new Error("confirmation_checksum_mismatch"));
  render(<FileConfirmationDialog {...props}/>);
  fireEvent.click(screen.getByRole("button", {name:"确认"}));
  await screen.findByRole("alert");
  expect(screen.getByRole("alert").textContent).toContain("文件内容已变化");
  expect(props.onCancel).not.toHaveBeenCalled();
  expect(props.onConfirmed).not.toHaveBeenCalled();
});
