import { afterEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => vi.fn());
vi.mock("./app-server-rpc", () => ({ rpc }));

import { MAX_FILE_LOAD_ATTEMPTS, loadWorkspaceFileSnapshot } from "./workspace-file-pages";

afterEach(() => vi.clearAllMocks());

const page = (revision: number, ids: string[], nextCursor: string | null) => ({
  workspaceId: "ws",
  workspaceRevision: revision,
  files: ids.map((id) => ({ localFileId: id })),
  nextCursor,
  hasMore: nextCursor !== null,
});

const ids = (snapshot: { files: { localFileId: string }[] } | null) =>
  snapshot?.files.map((file) => file.localFileId) ?? null;

describe("读一份修订号一致的文件列表", () => {
  it("单页直接返回", async () => {
    rpc.mockResolvedValueOnce(page(7, ["a"], null));
    const snapshot = await loadWorkspaceFileSnapshot("ws");
    expect(ids(snapshot)).toEqual(["a"]);
    expect(snapshot?.revision).toBe(7);
  });

  it("多页拼接，游标依次传下去", async () => {
    rpc.mockResolvedValueOnce(page(7, ["a"], "c1")).mockResolvedValueOnce(page(7, ["b"], null));
    expect(ids(await loadWorkspaceFileSnapshot("ws"))).toEqual(["a", "b"]);
    expect(rpc.mock.calls[1][1]).toMatchObject({ cursor: "c1" });
  });

  it("翻页途中目录变了就整轮重来，而不是报错给用户", async () => {
    // 大工作区要翻几十页，监听器重扫会推高修订号。跨修订号拼起来的列表会漏项或重项。
    rpc
      .mockResolvedValueOnce(page(7, ["a"], "c1"))
      .mockResolvedValueOnce(page(8, ["b"], null))
      .mockResolvedValueOnce(page(8, ["a"], "c1"))
      .mockResolvedValueOnce(page(8, ["b"], null));
    const snapshot = await loadWorkspaceFileSnapshot("ws");
    expect(ids(snapshot)).toEqual(["a", "b"]);
    expect(snapshot?.revision).toBe(8);
    // 重来的那一轮必须从头翻，不能接着上一轮的游标继续拼。
    expect(rpc.mock.calls[2][1]).toMatchObject({ cursor: null });
  });

  it("目录一直在变，重试到上限才承认读不出来", async () => {
    let revision = 0;
    rpc.mockImplementation((_m: string, params: { cursor: string | null }) =>
      Promise.resolve(params.cursor === null ? page(++revision, ["a"], "c1") : page(++revision, ["b"], null))
    );
    await expect(loadWorkspaceFileSnapshot("ws")).rejects.toThrow("目录持续变化");
    expect(rpc).toHaveBeenCalledTimes(MAX_FILE_LOAD_ATTEMPTS * 2);
  });

  it("已被更新的一次加载取代时立刻收手，不再翻剩下的页", async () => {
    rpc.mockResolvedValueOnce(page(7, ["a"], "c1"));
    expect(await loadWorkspaceFileSnapshot("ws", () => false)).toBeNull();
    expect(rpc).toHaveBeenCalledTimes(1);
  });

  it("游标原地不动说明分页坏了，立即停住不进死循环", async () => {
    rpc.mockResolvedValueOnce(page(7, ["a"], "c1")).mockResolvedValueOnce(page(7, ["b"], "c1"));
    await expect(loadWorkspaceFileSnapshot("ws")).rejects.toThrow("游标无效");
  });
});
