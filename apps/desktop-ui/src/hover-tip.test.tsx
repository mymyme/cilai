import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { HoverTip } from "./hover-tip";

afterEach(() => { cleanup(); vi.useRealTimers(); });
beforeEach(() => { vi.useFakeTimers({ shouldAdvanceTime: true }); });

const renderTip = (enabled = true) => render(
  <HoverTip enabled={enabled} label="自动化">
    {(tip) => <button onPointerDown={tip.onPointerDown} onPointerEnter={tip.onPointerEnter} onPointerLeave={tip.onPointerLeave} ref={tip.ref as never} type="button">图标</button>}
  </HoverTip>,
);

describe("侧边栏收起后的悬停气泡", () => {
  it("停留满一秒才弹，划过时不弹", async () => {
    renderTip();
    fireEvent.pointerEnter(screen.getByRole("button"));
    vi.advanceTimersByTime(900);
    expect(screen.queryByRole("tooltip")).toBeNull();
    vi.advanceTimersByTime(200);
    await waitFor(() => expect(screen.getByRole("tooltip").textContent).toBe("自动化"));
  });

  it("指针移开即收，不会留在屏幕上", async () => {
    renderTip();
    const button = screen.getByRole("button");
    fireEvent.pointerEnter(button);
    vi.advanceTimersByTime(1100);
    await waitFor(() => expect(screen.getByRole("tooltip")).toBeTruthy());
    fireEvent.pointerLeave(button);
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
  });

  it("按下即收：已经知道要去哪了，气泡留着只会挡住刚切过去的页面", async () => {
    renderTip();
    const button = screen.getByRole("button");
    fireEvent.pointerEnter(button);
    vi.advanceTimersByTime(1100);
    await waitFor(() => expect(screen.getByRole("tooltip")).toBeTruthy());
    fireEvent.pointerDown(button);
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
  });

  it("侧边栏展开时不弹：名称本来就看得见", () => {
    renderTip(false);
    fireEvent.pointerEnter(screen.getByRole("button"));
    vi.advanceTimersByTime(1500);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });
});
