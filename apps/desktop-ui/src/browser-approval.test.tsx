import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const listeners = vi.hoisted(() => new Map<string, (payload: unknown) => void>());
vi.mock("./host", () => ({
  invoke,
  listen: (event: string, handler: (payload: unknown) => void) => {
    listeners.set(event, handler);
    return Promise.resolve(() => listeners.delete(event));
  },
}));

import { BrowserApprovalSurface } from "./browser-approval";

const request = {
  confirmationId: "c-1",
  tool: "browser_click",
  site: "https://supplier.example.com",
  summary: "在 https://supplier.example.com 点击 (120, 240)：提交询价单",
  arguments: { x: 120, y: 240, purpose: "提交询价单" },
  riskLevel: "high",
};

const emit = (event: string, payload: unknown) => listeners.get(event)?.(payload);

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(null);
  listeners.clear();
});

afterEach(cleanup);

describe("AI 浏览器写操作的确认", () => {
  it("确认卡片说清站点、操作与参数，员工据此才能判断", async () => {
    render(<BrowserApprovalSurface />);
    await waitFor(() => expect(listeners.has("browser-approval-request")).toBe(true));
    emit("browser-approval-request", request);
    expect(await screen.findByText("https://supplier.example.com")).toBeTruthy();
    expect(screen.getByText("点击页面")).toBeTruthy();
    expect(screen.getByText("提交询价单")).toBeTruthy();
    expect(screen.getByText("120")).toBeTruthy();
  });

  /*
   * 授权按钮由宿主决定给不给。界面自行判断会出现两种坏情况：该给的没给（员工被逐次打扰），
   * 不该给的给了（输入类操作被一次授权盖住，而那类风险不随站点信任而降低）。
   */
  it("宿主允许时才出现按站点授权，点了带着授权标志回传", async () => {
    render(<BrowserApprovalSurface />);
    await waitFor(() => expect(listeners.has("browser-approval-request")).toBe(true));
    emit("browser-approval-request", { ...request, allowSiteGrant: true });
    fireEvent.click(await screen.findByRole("button", { name: "本站点本次对话都允许" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("browser_approval_resolve", {
        confirmationId: "c-1",
        approved: true,
        grantSite: true,
      });
    });
  });

  it("宿主不允许授权时不出现那个按钮", async () => {
    render(<BrowserApprovalSurface />);
    await waitFor(() => expect(listeners.has("browser-approval-request")).toBe(true));
    emit("browser-approval-request", { ...request, allowSiteGrant: false });
    expect(await screen.findByRole("button", { name: "批准本次" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "本站点本次对话都允许" })).toBeNull();
  });

  it("批准与拒绝都带上确认号回传，不静默丢弃", async () => {
    render(<BrowserApprovalSurface />);
    await waitFor(() => expect(listeners.has("browser-approval-request")).toBe(true));
    emit("browser-approval-request", request);
    fireEvent.click(await screen.findByRole("button", { name: "拒绝" }));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("browser_approval_resolve", {
        confirmationId: "c-1",
        approved: false,
        grantSite: false,
      });
    });
  });

  it("没有待确认项时不占据界面", async () => {
    const view = render(<BrowserApprovalSurface />);
    await waitFor(() => expect(listeners.has("browser-approval-request")).toBe(true));
    expect(view.container.querySelector(".browser-approval")).toBeNull();
    expect(document.querySelector(".confirm-backdrop")).toBeNull();
  });

  it("宿主判定失效后撤掉卡片，不让员工对着无效的卡片做决定", async () => {
    render(<BrowserApprovalSurface />);
    await waitFor(() => expect(listeners.has("browser-approval-closed")).toBe(true));
    emit("browser-approval-request", request);
    expect(await screen.findByText("点击页面")).toBeTruthy();
    emit("browser-approval-closed", { confirmationId: "c-1" });
    await waitFor(() => expect(screen.queryByText("点击页面")).toBeNull());
  });

  it("多项待确认时逐项处理，并告知还剩多少", async () => {
    render(<BrowserApprovalSurface />);
    await waitFor(() => expect(listeners.has("browser-approval-request")).toBe(true));
    emit("browser-approval-request", request);
    emit("browser-approval-request", { ...request, confirmationId: "c-2", summary: "第二项" });
    expect(await screen.findByText("另有 1 项待确认")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "批准本次" }));
    await waitFor(() => expect(screen.getByText("第二项")).toBeTruthy());
  });
});
