import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { WorkspacePermissions } from "./workspace-permissions";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
afterEach(cleanup);
beforeEach(() => invoke.mockReset());
const needsRepair = {status: "repairRequired", directory: "F:\\项目目录", userSid: "S-1-test", planId: "one-use"};
it("检查不提权，确认指定目录后才请求授权", async () => {
  invoke.mockResolvedValue(needsRepair);
  render(<WorkspacePermissions workspaceId="project"/>);
  fireEvent.click(await screen.findByRole("button", {name:"修复项目权限"}));
  expect((await screen.findByRole("dialog")).textContent).toContain("F:\\项目目录");
  expect(invoke).toHaveBeenCalledTimes(2);
  fireEvent.click(screen.getByRole("button", {name:"取消"}));
  expect(invoke).toHaveBeenCalledTimes(2);
  fireEvent.click(screen.getByRole("button", {name:"修复项目权限"}));
  await screen.findByRole("dialog");
  invoke.mockResolvedValue({...needsRepair,status:"ready",planId:null});
  fireEvent.click(screen.getByRole("button", {name:"请求管理员授权"}));
  await screen.findByText(/已通过不依赖管理员权限的复查/);
  expect(invoke).toHaveBeenLastCalledWith("workspace_permissions_repair", {workspaceId:"project",planId:"one-use"});
});
it("已提权时仍需确认，复用现有授权执行同一修复流程", async () => {
  invoke.mockResolvedValue({...needsRepair, processElevated:true});
  render(<WorkspacePermissions workspaceId="project"/>);
  fireEvent.click(await screen.findByRole("button", {name:"修复项目权限"}));
  await screen.findByRole("dialog");
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(screen.queryByRole("button", {name:"请求管理员授权"})).toBeNull();
  fireEvent.click(screen.getByRole("button", {name:"确认修复"}));
  await waitFor(() => expect(invoke).toHaveBeenLastCalledWith("workspace_permissions_repair", {workspaceId:"project",planId:"one-use"}));
});
it("授权取消后明确显示结果，并要求重新检查", async () => {
  invoke.mockResolvedValueOnce(needsRepair).mockResolvedValueOnce(needsRepair).mockRejectedValueOnce(new Error("已取消管理员授权，项目权限未修改。"));
  render(<WorkspacePermissions workspaceId="project"/>);
  fireEvent.click(await screen.findByRole("button", {name:"修复项目权限"}));
  fireEvent.click(await screen.findByRole("button", {name:"请求管理员授权"}));
  expect((await screen.findByRole("alert")).textContent).toContain("已取消管理员授权");
  expect(screen.queryByRole("button", {name:"修复项目权限"})).toBeNull();
});
it("切换项目丢弃旧检查和旧确认", async () => {
  let finish!: (value:unknown) => void;
  invoke.mockImplementationOnce(() => new Promise(resolve => {finish = resolve;})).mockResolvedValue({status:"ready"});
  const view = render(<WorkspacePermissions workspaceId="old"/>);
  view.rerender(<WorkspacePermissions workspaceId="new"/>);
  await screen.findByText("当前用户可以更新项目目录的沙箱权限。");
  finish(needsRepair);
  await waitFor(() => expect(screen.queryByRole("button", {name:"修复项目权限"})).toBeNull());
});
it("缓存显示不复用旧修复计划，用户操作时重新生成确认", async () => {
  let checks=0;
  invoke.mockImplementation(async(command)=>command==="workspace_permissions_check"
    ? {...needsRepair,planId:++checks===1 ? "expired" : "fresh"}
    : {status:"ready"});
  render(<WorkspacePermissions workspaceId="project"/>);
  fireEvent.click(await screen.findByRole("button",{name:"修复项目权限"}));
  fireEvent.click(await screen.findByRole("button",{name:"请求管理员授权"}));
  await screen.findByText(/已通过不依赖管理员权限的复查/);
  expect(invoke).toHaveBeenLastCalledWith("workspace_permissions_repair",{workspaceId:"project",planId:"fresh"});
  expect(checks).toBe(2);
});

it.each([
  ["\\\\?\\F:\\项目目录", "F:\\项目目录"],
  ["\\\\?\\UNC\\server\\share\\项目目录", "\\\\server\\share\\项目目录"],
])("展示可读路径，授权仍只提交原生一次性计划 %s", async (directory, displayed) => {
  invoke.mockResolvedValue({...needsRepair,directory});
  render(<WorkspacePermissions workspaceId="project"/>);
  expect((await screen.findByTitle(displayed)).textContent).toBe(displayed);
  fireEvent.click(screen.getByRole("button",{name:"修复项目权限"}));
  expect((await screen.findByRole("dialog")).textContent).toContain(displayed);
  fireEvent.click(screen.getByRole("button",{name:"请求管理员授权"}));
  await waitFor(() => expect(invoke).toHaveBeenLastCalledWith("workspace_permissions_repair", {workspaceId:"project",planId:"one-use"}));
});
