import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { PersonalCapabilityManager } from "./personal-capability-manager";
const host = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("./host", () => host);
const project = { creationProjectId: "personal1", title: "我的工具", packageDigest: "sha256:one", stateRevision: 2, status: "installed" };
const onChanged = vi.fn();
const onClose = vi.fn();
beforeEach(() => { vi.clearAllMocks(); host.invoke.mockImplementation(async command=>command==="personal_application_data_list"?[]:{ project, enabled: true }); });
afterEach(cleanup);
function open() { render(<PersonalCapabilityManager projectId="personal1" onChanged={onChanged} onClose={onClose}/>); }
test("停用使用最新版本并同步状态，重新启用使用新修订", async () => {
  open();
  const disable = await screen.findByRole("button", { name: "停用" });
  host.invoke.mockResolvedValue({ project: { ...project, stateRevision: 3 }, enabled: false });
  fireEvent.click(disable);
  const enable = await screen.findByRole("button", { name: "启用" });
  expect(host.invoke).toHaveBeenLastCalledWith("personal_capability_manage", { request: expect.objectContaining({ creationProjectId: "personal1", expectedRevision: 2, packageDigest: "sha256:one", action: "disable", clientOperationId: expect.any(String) }) });
  expect(onChanged).toHaveBeenCalledWith({ project: { ...project, stateRevision: 3 }, enabled: false });
  host.invoke.mockResolvedValue({ project: { ...project, stateRevision: 4 }, enabled: true });
  fireEvent.click(enable);
  await screen.findByRole("button", { name: "停用" });
  expect(host.invoke).toHaveBeenLastCalledWith("personal_capability_manage", { request: expect.objectContaining({ expectedRevision: 3, action: "enable" }) });
});
test("卸载需明确确认，取消不改变安装，成功后关闭管理", async () => {
  open(); fireEvent.click(await screen.findByRole("button", { name: "卸载" }));
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  expect(host.invoke.mock.calls.filter(([command])=>command==="personal_capability_manage")).toHaveLength(0);
  fireEvent.click(screen.getByRole("button", { name: "卸载" }));
  expect(screen.getByText("卸载后无法运行，创建项目和个人数据会保留。")).toBeTruthy();
  host.invoke.mockResolvedValue({ project: { ...project, status: "packaged", stateRevision: 3 }, enabled: null });
  fireEvent.click(screen.getByRole("button", { name: "确认卸载" }));
  await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  expect(onChanged).toHaveBeenCalledWith({ project: { ...project, status: "packaged", stateRevision: 3 }, enabled: null });
});
test("操作结果未知时先刷新事实，不继续提交旧修订", async () => {
  open(); const disable = await screen.findByRole("button", { name: "停用" });
  host.invoke.mockRejectedValue(new Error("disconnected")); fireEvent.click(disable);
  await screen.findByRole("alert");
  expect(screen.queryByRole("button", { name: "启用" })).toBeNull();
  expect(screen.queryByRole("button", { name: "停用" })).toBeNull();
  expect(onChanged).toHaveBeenCalledTimes(1);
  host.invoke.mockResolvedValue({ project: { ...project, stateRevision: 3 }, enabled: false });
  fireEvent.click(screen.getByRole("button", { name: "刷新" }));
  await screen.findByRole("button", { name: "启用" });
});
