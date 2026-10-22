import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { ApplicationHandoffs, type ApplicationHandoff } from "./application-handoffs";
const state = vi.hoisted(() => ({ entries: [] as ApplicationHandoff[], flush: vi.fn(), refresh: vi.fn(), invoke: vi.fn() }));
vi.mock("./host", () => ({ invoke: state.invoke }));
vi.mock("./conversation-session", () => ({
  useConversationSessionState: () => [state.entries, (update: (values: ApplicationHandoff[]) => ApplicationHandoff[]) => { state.entries = update(state.entries); }],
  useConversationSessionFlush: () => state.flush,
  useConversationSessionRefresh: () => state.refresh,
  useConversationSessionId: () => "profile-session",
}));
const pending: ApplicationHandoff = { handoffId: "handoff-run", applicationId: "air", runId: "run", requestId: "request-1", source: { applicationName: "空气质量热力图" }, output: "原始回答" };
const project = { projectId: "project-1", displayName: "测试项目", workspaceIds: [], primaryWorkspaceId: null, primaryWorkspaceRoot: null };
const props = () => ({ projects: [project], projectsReady: true, projectsError: "", onRetryProjects: vi.fn(), onOpen: vi.fn() });
beforeEach(() => { state.entries = [structuredClone(pending)]; state.flush.mockReset().mockResolvedValue(undefined); state.refresh.mockReset().mockResolvedValue(undefined); state.invoke.mockReset().mockResolvedValue({ conversationId: "conversation-1" }); });
afterEach(cleanup);
test("转交直接弹出项目选择，不关联项目后打开正式对话", async () => {
  const view = props(); render(<ApplicationHandoffs {...view}/>);
  expect(await screen.findByRole("dialog", { name: "转交到主对话" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "不关联项目" }));
  await waitFor(() => expect(view.onOpen).toHaveBeenCalledWith("conversation-1"));
  expect(state.invoke).toHaveBeenCalledExactlyOnceWith("ai_application_handoff_commit", { request: { sessionId: "profile-session", handoffId: "handoff-run", projectId: null } });
  expect(screen.queryByRole("dialog")).toBeNull();
});
test("选择项目后一次创建，提交期间不能重复点击", async () => {
  let resolve!: (value: unknown) => void;
  state.invoke.mockImplementation(() => new Promise((done) => { resolve = done; }));
  const view = props(); render(<ApplicationHandoffs {...view}/>);
  fireEvent.click(await screen.findByRole("button", { name: "测试项目" }));
  await waitFor(() => expect(state.invoke).toHaveBeenCalledTimes(1));
  expect(screen.getByRole("button", { name: "测试项目" }).hasAttribute("disabled")).toBe(true);
  expect(screen.getByRole("button", { name: "取消" }).hasAttribute("disabled")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "测试项目" }));
  await act(async () => resolve({ conversationId: "project-conversation" }));
  expect(state.invoke).toHaveBeenCalledExactlyOnceWith("ai_application_handoff_commit", { request: { sessionId: "profile-session", handoffId: "handoff-run", projectId: "project-1" } });
  await waitFor(() => expect(view.onOpen).toHaveBeenCalledWith("project-conversation"));
});
test("取消不创建对话，保留其他转交", async () => {
  state.entries.push({ ...pending, handoffId: "legacy", requestId: undefined });
  state.invoke.mockResolvedValue({status:"cancelled"});
  state.refresh.mockImplementation(async()=>{state.entries=state.entries.filter(entry=>entry.handoffId!=="handoff-run")});
  const view = props(); render(<ApplicationHandoffs {...view}/>);
  fireEvent.click(await screen.findByRole("button", { name: "取消" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(state.entries.map((entry) => entry.handoffId)).toEqual(["legacy"]);
  expect(state.invoke).toHaveBeenCalledExactlyOnceWith("ai_application_handoff_cancel",{request:{sessionId:"profile-session",handoffId:"handoff-run",projectId:null}}); expect(view.onOpen).not.toHaveBeenCalled();
});
test("创建失败保留来源，重试后再打开", async () => {
  state.invoke.mockRejectedValueOnce(new Error("保存失败"));
  const view = props(); render(<ApplicationHandoffs {...view}/>);
  fireEvent.click(await screen.findByRole("button", { name: "不关联项目" }));
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(state.entries[0]?.output).toBe("原始回答"); expect(view.onOpen).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "不关联项目" }));
  await waitFor(() => expect(view.onOpen).toHaveBeenCalledWith("conversation-1"));
});
test("旧转交可以直接选择项目，不再生成输入草稿", async () => {
  state.entries = [{ ...pending, requestId: undefined }];
  const view = props(); render(<ApplicationHandoffs {...view}/>);
  expect(screen.queryByRole("dialog")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "空气质量热力图 · 继续转交" }));
  fireEvent.click(screen.getByRole("button", { name: "测试项目" }));
  await waitFor(() => expect(view.onOpen).toHaveBeenCalledWith("conversation-1"));
});

test("取消结果不明时保留转交与选择窗口", async()=>{
  state.invoke.mockRejectedValue(new Error("application_handoff_creation_unconfirmed"));
  const view=props();render(<ApplicationHandoffs {...view}/>);
  fireEvent.click(await screen.findByRole("button",{name:"取消"}));
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(screen.getByRole("dialog")).toBeTruthy();expect(state.entries).toHaveLength(1);expect(view.onOpen).not.toHaveBeenCalled();
});
