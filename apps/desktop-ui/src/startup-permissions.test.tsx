import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { StartupPermissions } from "./startup-permissions";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
afterEach(cleanup);
beforeEach(() => invoke.mockReset());
const inspection = {status:"confirmationRequired", planId:"one-use", directory:"C:\\Users\\test\\AppData\\Local\\com.cilai.workbench\\dev", userSid:"original-user"};

it("does not request UAC on load or read-only check; repairs only after confirmation", async () => {
  const recovered = vi.fn();
  invoke.mockResolvedValueOnce(inspection).mockResolvedValueOnce({status:"ready"});
  render(<StartupPermissions onRecovered={recovered}/>);
  expect(invoke).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", {name:"检查并修复启动权限"}));
  await screen.findByText("Windows 用户：original-user");
  expect(invoke.mock.calls.map(call => call[0])).toEqual(["startup_permissions_check"]);
  fireEvent.click(screen.getByRole("button", {name:"修复并重试启动"}));
  await screen.findByText("权限复查通过，本地服务已重新启动。");
  expect(invoke).toHaveBeenLastCalledWith("startup_permissions_repair", {planId:"one-use"});
  expect(recovered).toHaveBeenCalledOnce();
});
it("cancel and failure never report recovery or reuse a consumed confirmation", async () => {
  const recovered = vi.fn();
  invoke.mockResolvedValueOnce(inspection);
  render(<StartupPermissions onRecovered={recovered}/>);
  fireEvent.click(screen.getByRole("button", {name:"检查并修复启动权限"}));
  fireEvent.click(await screen.findByRole("button", {name:"取消"}));
  expect(invoke).toHaveBeenCalledTimes(1);
  invoke.mockResolvedValueOnce(inspection).mockRejectedValueOnce(new Error("已取消管理员授权"));
  fireEvent.click(screen.getByRole("button", {name:"检查并修复启动权限"}));
  fireEvent.click(await screen.findByRole("button", {name:"修复并重试启动"}));
  await screen.findByText(/已取消管理员授权/);
  expect(recovered).not.toHaveBeenCalled();
  expect(screen.queryByRole("button", {name:"修复并重试启动"})).toBeNull();
});

it("an already elevated Windows identity can confirm recovery without being told to reopen normally", async () => {
  invoke.mockResolvedValueOnce({...inspection, processElevated:true}).mockResolvedValueOnce({status:"ready"});
  render(<StartupPermissions/>);
  fireEvent.click(screen.getByRole("button", {name:"检查并修复启动权限"}));
  await screen.findByText(/当前工作台已具有管理员权限/);
  expect(screen.queryByText(/以普通用户/)).toBeNull();
  expect(invoke).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole("button", {name:"修复并重试启动"}));
  await screen.findByText("权限复查通过，本地服务已重新启动。");
  expect(invoke).toHaveBeenLastCalledWith("startup_permissions_repair", {planId:"one-use"});
});
