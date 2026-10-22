import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { ArchiveNotice } from "./archive-notice";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
afterEach(() => { cleanup(); vi.useRealTimers(); invoke.mockReset(); });

describe("归档提示", () => {
  test("悬停及键盘焦点暂停自动关闭", async () => {
    vi.useFakeTimers();
    const onClose = vi.fn();
    render(<ArchiveNotice operation={{ id: 1, conversationIds: ["a"] }} onClose={onClose} onView={vi.fn()} onMessage={vi.fn()}/>);
    fireEvent.mouseEnter(screen.getByRole("status"));
    await act(async () => vi.advanceTimersByTime(30_000));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.mouseLeave(screen.getByRole("status"));
    fireEvent.focus(screen.getByRole("button", { name: "撤销" }));
    await act(async () => vi.advanceTimersByTime(30_000));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.blur(screen.getByRole("button", { name: "撤销" }), { relatedTarget: document.body });
    await act(async () => vi.advanceTimersByTime(15_000));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
  test("撤销失败保持提示，重试仍绑定本次归档并阻止重复提交", async () => {
    const onClose = vi.fn(); const onMessage = vi.fn();
    let reject!: (error: Error) => void;
    invoke.mockImplementationOnce(() => new Promise((_resolve, fail) => { reject = fail; }));
    render(<ArchiveNotice operation={{ id: 1, conversationIds: ["a", "b"] }} onClose={onClose} onView={vi.fn()} onMessage={onMessage}/>);
    fireEvent.click(screen.getByRole("button", { name: "撤销" }));
    fireEvent.click(screen.getByRole("button", { name: "撤销中…" }));
    expect(invoke).toHaveBeenCalledTimes(1);
    await act(async () => reject(new Error("ai_state_write_failed")));
    expect(onMessage).toHaveBeenCalledWith(expect.stringContaining("撤销失败"), "error");
    expect(onClose).not.toHaveBeenCalled();
    invoke.mockResolvedValueOnce({ stateRevision: 2, conversations: [], changedConversationIds: [], failures: [] });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "撤销" })));
    expect(invoke).toHaveBeenLastCalledWith("ai_conversation_set_archived", { request: { conversationIds: ["a", "b"], archived: false } });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
  test("关闭旧提示后其迟到的撤销回执不关闭新提示", async () => {
    let resolve!: (value: unknown) => void;
    invoke.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    const oldClose = vi.fn(); const newClose = vi.fn();
    const view = render(<ArchiveNotice key="old" operation={{ id: 1, conversationIds: ["a"] }} onClose={oldClose} onView={vi.fn()} onMessage={vi.fn()}/>);
    fireEvent.click(screen.getByRole("button", { name: "撤销" }));
    view.rerender(<ArchiveNotice key="new" operation={{ id: 2, conversationIds: ["b"] }} onClose={newClose} onView={vi.fn()} onMessage={vi.fn()}/>);
    await act(async () => resolve({ stateRevision: 2, conversations: [], changedConversationIds: [], failures: [] }));
    expect(oldClose).not.toHaveBeenCalled(); expect(newClose).not.toHaveBeenCalled();
    expect(screen.getByText("已归档对话")).toBeTruthy();
  });
});
