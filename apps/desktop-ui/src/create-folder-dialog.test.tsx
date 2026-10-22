import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { WorkspaceSummary } from "./generated/protocol";
import { CreateFolderDialog, currentDirectory } from "./create-folder-dialog";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
const workspace = { workspaceId: "ws-1", workspaceKind: "local", displayName: "项目资料" } as WorkspaceSummary;
beforeEach(() => { invoke.mockReset(); });
afterEach(cleanup);

test("按选中的目录或文件所在目录确定新增位置", () => {
  expect(currentDirectory(null)).toBe("");
  expect(currentDirectory({ relativePath: "scripts", fileType: "directory" })).toBe("scripts");
  expect(currentDirectory({ relativePath: "scripts/run.py", fileType: "file" })).toBe("scripts");
  expect(currentDirectory({ relativePath: "README.md", fileType: "file" })).toBe("");
  expect(currentDirectory({ relativePath: "资料\\报告.md", fileType: "file" })).toBe("资料");
});

test("只在当前工作空间创建文件夹，成功后刷新目录", async () => {
  invoke.mockResolvedValue({ relativePath: "scripts/新文件夹" });
  const onCreated = vi.fn().mockResolvedValue(undefined);
  const onClose = vi.fn();
  render(<CreateFolderDialog workspace={workspace} parentRelativePath="scripts" onClose={onClose} onCreated={onCreated}/>);
  fireEvent.change(screen.getByLabelText("文件夹名称"), { target: { value: "新文件夹" } });
  fireEvent.click(screen.getByRole("button", { name: "新建" }));
  await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
  expect(invoke).toHaveBeenCalledExactlyOnceWith("app_server_request", { method: "workspace/createDirectory", params: { workspaceId: "ws-1", parentRelativePath: "scripts", name: "新文件夹", clientOperationId: expect.stringMatching(/^desktop-create-directory-/) } });
  expect(onCreated).toHaveBeenCalledWith("scripts/新文件夹");
});

test("创建重名失败保留输入并明确显示错误", async () => {
  invoke.mockRejectedValue(new Error("workspace_directory_already_exists"));
  const onClose = vi.fn();
  render(<CreateFolderDialog workspace={workspace} parentRelativePath="" onClose={onClose} onCreated={vi.fn()}/>);
  fireEvent.change(screen.getByLabelText("文件夹名称"), { target: { value: "已有目录" } });
  fireEvent.click(screen.getByRole("button", { name: "新建" }));
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", "已存在同名文件或文件夹。");
  expect(screen.getByDisplayValue("已有目录")).toBeTruthy();
  expect(onClose).not.toHaveBeenCalled();
});

test("创建后扫描失败只重试刷新，不重复写入文件系统", async () => {
  invoke.mockResolvedValue({ relativePath: "新文件夹" });
  const onCreated = vi.fn().mockRejectedValueOnce(new Error("scan_failed")).mockResolvedValue(undefined);
  const onClose = vi.fn();
  render(<CreateFolderDialog workspace={workspace} parentRelativePath="" onClose={onClose} onCreated={onCreated}/>);
  fireEvent.change(screen.getByLabelText("文件夹名称"), { target: { value: "新文件夹" } });
  fireEvent.click(screen.getByRole("button", { name: "新建" }));
  await screen.findByRole("alert");
  fireEvent.click(screen.getByRole("button", { name: "刷新" }));
  await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(onCreated).toHaveBeenCalledTimes(2);
});
