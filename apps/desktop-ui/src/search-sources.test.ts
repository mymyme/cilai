import { beforeEach, expect, it, vi } from "vitest";
const rpc = vi.hoisted(() => vi.fn());
const invoke = vi.hoisted(() => vi.fn());
vi.mock("./app-server-rpc", () => ({ rpc }));
vi.mock("./host", () => ({ invoke }));
import { searchSources } from "./search-sources";
beforeEach(() => { vi.clearAllMocks(); });
it("分页原生归档只调用对应线程索引", async () => {
  invoke.mockResolvedValue({ providerId: "native-archived-conversations", items: [], nextCursor: null });
  await searchSources({ query: "历史", scope: ["local", "conversations"], cursors: [{ providerId: "native-archived-conversations", value: "page2" }], limit: 8 });
  expect(rpc).not.toHaveBeenCalled();
  expect(invoke).toHaveBeenCalledExactlyOnceWith("ai_conversation_search", { query: "历史", archived: true, cursor: "page2", limit: 8 });
});
it("原生搜索失败保留其他来源并明确列出失败", async () => {
  rpc.mockResolvedValue({ requestId: "r", authorizationRevision: null, groups: [{ providerId: "local", items: [] }], partialFailures: [] });
  invoke.mockRejectedValue(new Error("thread service stopped"));
  const result = await searchSources({ query: "历史", scope: ["local", "conversations"], cursors: [], limit: 8 });
  expect(result.groups).toHaveLength(1);
  expect(result.partialFailures.map((failure) => failure.providerId)).toEqual(["native-conversations", "native-archived-conversations"]);
});

it("企业搜索来源在发出请求前拒绝", async () => {
  await expect(searchSources({query:"资料",scope:["knowledge"],cursors:[],limit:8})).rejects.toThrow("personal_search_source_removed");
  expect(rpc).not.toHaveBeenCalled(); expect(invoke).not.toHaveBeenCalled();
});
