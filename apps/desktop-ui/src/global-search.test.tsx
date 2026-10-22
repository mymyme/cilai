import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => vi.fn());
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./host", () => ({ invoke }));
vi.mock("./app-server-rpc", () => ({ rpc }));

import { GlobalSearchPage } from "./global-search";

const localItem = {
  hitType: "local",
  providerId: "com.cilai.workbench.local",
  objectType: "file",
  objectId: "record-1",
  title: "排污许可证",
  summary: "许可证摘要",
  matchReason: "档案名称匹配",
  updatedAt: "2026-09-19T00:00:00.000Z",
  deepLink: null,
  localTarget: {projectId:null,workspaceId:"workspace-1",phaseId:null,deliverableId:null,localFileId:"file-1",relativePath:"许可证.md"},
  sourceIdentity: { providerId: "com.cilai.workbench.local" },
  businessReference: null,
  knowledgeReference: null,
};

beforeEach(() => {
  invoke.mockImplementation(async (_command, args) => ({ groupId: args.archived ? "native-archived-conversations" : "native-conversations", providerId: args.archived ? "native-archived-conversations" : "native-conversations", label: "对话", kind: "conversations", items: [], nextCursor: null }));
  rpc.mockResolvedValue({
    requestId: "search-1",
    authorizationRevision: "a".repeat(64),
    groups: [{
      groupId: "local",
      providerId: "com.cilai.workbench.local",
      label: "本地文件",
      kind: "local",
      items: [localItem],
      nextCursor: null,
    }],
    partialFailures: [{
      groupId: "knowledge",
      providerId: "com.cilai.workbench.knowledge",
      code: "knowledge_unavailable",
      message: "本地索引检索未完成",
    }],
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("统一检索页面", () => {
  it("默认只请求本地工作与原生会话并展示部分失败", async () => {
    render(
      <GlobalSearchPage
        hostPlatform="macos"
        onMessage={vi.fn()}
        onOpen={vi.fn()}
        profileId="profile-a"
      />,
    );
    fireEvent.change(screen.getByRole("searchbox", { name: "全局搜索" }), {
      target: { value: "许可证" },
    });

    await waitFor(() => {
      expect(rpc).toHaveBeenCalledWith("unifiedSearch/query", {
        query: "许可证",
        scope: ["local"],
        limit: 8,
        cursors: [],
      });
    });
    expect(await screen.findByRole("button", { name: "排污许可证" })).toBeTruthy();
    expect(screen.getByText("本地索引检索未完成")).toBeTruthy();
    expect(screen.getByText("请求 search-1")).toBeTruthy();
  });

  it("点击本地结果只把统一结果对象交给受控打开链路", async () => {
    const onOpen = vi.fn();
    render(
      <GlobalSearchPage
        hostPlatform="macos"
        onMessage={vi.fn()}
        onOpen={onOpen}
        profileId="profile-a"
      />,
    );
    fireEvent.change(screen.getByRole("searchbox", { name: "全局搜索" }), {
      target: { value: "许可证" },
    });
    fireEvent.click(await screen.findByRole("button", { name: /排污许可证/ }));
    expect(onOpen).toHaveBeenCalledWith(localItem);
  });

  it("资料切换或新查询取消旧请求时不显示失败提示", async () => {
    const onMessage = vi.fn();
    rpc.mockRejectedValueOnce(new Error("unified_search_superseded"));
    render(
      <GlobalSearchPage
        hostPlatform="macos"
        onMessage={onMessage}
        onOpen={vi.fn()}
        profileId="profile-a"
      />,
    );
    fireEvent.change(screen.getByRole("searchbox", { name: "全局搜索" }), {
      target: { value: "许可证" },
    });
    await waitFor(() => expect(rpc).toHaveBeenCalled());
    await waitFor(() => expect(onMessage).not.toHaveBeenCalled());
    expect(screen.queryByText(/搜索结果读取失败/)).toBeNull();
  });
});

it("筛选来源并扩大每组结果范围，换关键词后恢复默认范围", async () => {
  rpc.mockImplementation(async (_method, params) => ({ requestId: "many", groups: [{ groupId: "local", providerId: "local", label: "本地工作", items: Array.from({ length: params.limit }, (_, index) => ({ ...localItem, objectId: `item-${index}`, title: `结果 ${index}` })), nextCursor: null }], partialFailures: [] }));
  render(<GlobalSearchPage hostPlatform="macos" onMessage={vi.fn()} onOpen={vi.fn()} profileId="a"/>);
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "文档" } });
  expect(await screen.findByText("已显示 8 项")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "本地工作" }));
  await waitFor(() => expect(rpc).toHaveBeenLastCalledWith("unifiedSearch/query", { query: "文档", scope: ["local"], limit: 8, cursors: [] }));
  fireEvent.click(await screen.findByRole("button", { name: "每组显示 20 项" }));
  expect(await screen.findByText("已显示 20 项")).toBeTruthy();
  expect(screen.getByText("已显示 20 项")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "每组显示 20 项" })).toBeNull();
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "报告" } });
  await waitFor(() => expect(rpc).toHaveBeenLastCalledWith("unifiedSearch/query", { query: "报告", scope: ["local"], limit: 8, cursors: [] }));
});

it("加载失败只在结果区呈现，重试成功后恢复结果", async () => {
  const onMessage = vi.fn();
  rpc.mockRejectedValueOnce(new Error("连接已断开"));
  render(<GlobalSearchPage hostPlatform="macos" onMessage={onMessage} onOpen={vi.fn()} profileId="a"/>);
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "许可证" } });
  expect(await screen.findByText("搜索结果读取失败")).toBeTruthy();
  expect(onMessage).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "重试" }));
  expect(await screen.findByRole("button", { name: "排污许可证" })).toBeTruthy();
  expect(screen.queryByText("搜索结果读取失败")).toBeNull();
});

it("旧来源的迟到结果不能覆盖当前查询，切换资料清空输入", async () => {
  let resolveOld!: (value: unknown) => void;
  rpc.mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve; }));
  const props = { hostPlatform: "macos", onMessage: vi.fn(), onOpen: vi.fn(), profileId: "a" };
  const page = render(<GlobalSearchPage {...props}/>);
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "许可证" } });
  await waitFor(() => expect(resolveOld).toBeTypeOf("function"));
  fireEvent.click(screen.getByRole("button", { name: "本地工作" }));
  expect(await screen.findByRole("button", { name: "排污许可证" })).toBeTruthy();
  resolveOld({ groups: [{ providerId: "old", groupId: "old", label: "旧来源", items: [{ ...localItem, title: "不应出现的旧结果" }] }], partialFailures: [] });
  await waitFor(() => expect(screen.queryByText("不应出现的旧结果")).toBeNull());
  page.rerender(<GlobalSearchPage {...props} profileId="b"/>);
  expect((screen.getByRole("searchbox") as HTMLInputElement).value).toBe("");
  expect(screen.queryByRole("button", { name: "排污许可证" })).toBeNull();
});

it("所有来源失败时不把未知结果显示为没有匹配结果", async () => {
  rpc.mockResolvedValueOnce({ groups: [], partialFailures: [{ groupId: "knowledge", providerId: "knowledge", message: "本地索引暂不可用" }] });
  render(<GlobalSearchPage hostPlatform="macos" onMessage={vi.fn()} onOpen={vi.fn()} profileId="a"/>);
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "许可证" } });
  expect(await screen.findByText("本地索引暂不可用")).toBeTruthy();
  expect(screen.queryByText("没有匹配结果")).toBeNull();
  expect(screen.getByRole("button", { name: "重试" })).toBeTruthy();
});


it("按来源游标加载后续结果，合并时去重且不改变其他来源", async () => {
  const group = { groupId: "local", providerId: "com.cilai.workbench.local", kind: "local", label: "本地文件", items: [localItem], nextCursor: "signed-next" };
  const other = { groupId: "local-other", providerId: "local-other", kind: "local", label: "本地工作", items: [{ ...localItem, title: "本地文件", objectId: "local-file" }], nextCursor: null };
  rpc.mockResolvedValueOnce({ requestId: "page-1", authorizationRevision: "a".repeat(64), groups: [group, other], partialFailures: [] });
  rpc.mockResolvedValueOnce({ requestId: "page-2", authorizationRevision: "a".repeat(64), groups: [{ ...group, items: [localItem, { ...localItem, objectId: "second", title: "第二份许可证" }], nextCursor: null }], partialFailures: [] });
  render(<GlobalSearchPage hostPlatform="macos" onMessage={vi.fn()} onOpen={vi.fn()} profileId="a"/>);
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "许可证" } });
  fireEvent.click(await screen.findByRole("button", { name: "加载更多本地文件" }));
  expect(await screen.findByRole("button", { name: "第二份许可证" })).toBeTruthy();
  expect(screen.getAllByRole("button", { name: "排污许可证" })).toHaveLength(1);
  expect(screen.getByRole("button", {name:"本地文件"})).toBeTruthy();
  expect(rpc).toHaveBeenLastCalledWith("unifiedSearch/query", { query: "许可证", scope: ["local"], limit: 8, cursors: [{ providerId: "com.cilai.workbench.local", value: "signed-next" }] });
  expect(screen.queryByRole("button", { name: "加载更多本地文件" })).toBeNull();
});

it("搜索范围不提供企业来源", () => {
  render(<GlobalSearchPage hostPlatform="macos" onMessage={vi.fn()} onOpen={vi.fn()} profileId="a"/>);
  expect(screen.queryByRole("button",{name:"企业知识"})).toBeNull();
  expect(screen.queryByRole("button",{name:"应用数据"})).toBeNull();
});
