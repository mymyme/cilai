import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useState } from "react";
import { useModelHistoryTransfer } from "./model-history-transfer";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
const sent = vi.fn();
const selection = { connectionId: "personal-connection", modelId: "model-one", reasoningEffort: null };
const preview = { ...selection, configRevision: 2, authGeneration: 3, runtimeKind: "workbench", snapshotDigest: "snapshot-2", historyDigest: "history-4",
  messageCount: 4, attachmentCount: 1, serviceName: "新服务", baseUrl: "https://models.example.test/v1", historyScope: "当前会话中获准发送的消息与附件", mayIncurCost: true };
function Harness({ scope = "conversation-one" }: { scope?: string }) {
  const transfer = useModelHistoryTransfer(scope);
  const [draft, setDraft] = useState("保留我的草稿");
  const [error, setError] = useState("");
  return <><input aria-label="草稿" value={draft} onChange={(event) => setDraft(event.target.value)} disabled={transfer.busy}/>
    <button type="button" disabled={transfer.busy} onClick={() => void (async () => {
      try { const result = await transfer.approve(selection, scope); if (result.allowed) sent({ draft, consent: result.consent }); }
      catch (reason) { setError(String(reason)); }
    })()}>发送</button>{transfer.dialog}{error ? <p role="alert">{error}</p> : null}</>;
}
beforeEach(() => { sent.mockReset(); invoke.mockReset(); invoke.mockResolvedValue({ snapshot: selection, snapshotDigest: "snapshot-2", historyTransfer: preview }); });
afterEach(cleanup);

it("跨连接先展示目标和历史范围，确认后只提交精确修订与摘要", async () => {
  render(<Harness/>);
  fireEvent.click(screen.getByRole("button", { name: "发送" }));
  await screen.findByRole("dialog", { name: "向此连接发送历史？" });
  expect(screen.getByText(/目标：新服务/)).toBeTruthy();
  expect(screen.getByText(/4 条消息，1 个附件/)).toBeTruthy();
  expect(sent).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "确认发送" }));
  await waitFor(() => expect(sent).toHaveBeenCalledTimes(1));
  expect(sent.mock.calls[0][0].consent).toMatchObject({ connectionId: selection.connectionId, modelId: selection.modelId,
    configRevision: 2, authGeneration: 3, runtimeKind: "workbench", snapshotDigest: "snapshot-2", historyDigest: "history-4" });
  expect(sent.mock.calls[0][0].consent).not.toHaveProperty("serviceName");
});

it("取消历史发送保留草稿，不提交模型任务", async () => {
  render(<Harness/>);
  fireEvent.click(screen.getByRole("button", { name: "发送" }));
  await screen.findByRole("dialog"); fireEvent.click(screen.getByRole("button", { name: "取消" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(screen.getByLabelText("草稿")).toHaveProperty("value", "保留我的草稿"); expect(sent).not.toHaveBeenCalled();
});

it("切换会话后丢弃旧预检结果，不向新会话弹出旧历史确认", async () => {
  let finish!: (value: unknown) => void;
  invoke.mockReturnValue(new Promise((resolve) => { finish = resolve; }));
  const view = render(<Harness/>); fireEvent.click(screen.getByRole("button", { name: "发送" }));
  view.rerender(<Harness scope="conversation-two"/>);
  finish({ snapshot: selection, snapshotDigest: "snapshot-2", historyTransfer: preview });
  await waitFor(() => expect(screen.getByRole("button", { name: "发送" })).toHaveProperty("disabled", false));
  expect(screen.queryByRole("dialog")).toBeNull(); expect(sent).not.toHaveBeenCalled();
});

it("迟到的另一连接快照明确失败，不借用其历史同意", async () => {
  invoke.mockResolvedValue({ snapshot: { ...selection, connectionId: "other-connection" }, snapshotDigest: "other", historyTransfer: preview });
  render(<Harness/>); fireEvent.click(screen.getByRole("button", { name: "发送" }));
  expect((await screen.findByRole("alert")).textContent).toContain("model_execution_snapshot_changed");
  expect(screen.queryByRole("dialog")).toBeNull(); expect(sent).not.toHaveBeenCalled();
});
