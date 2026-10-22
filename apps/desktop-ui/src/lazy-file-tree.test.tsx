import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => vi.fn());
vi.mock("./app-server-rpc", () => ({ rpc }));

import { LazyFileTree, flattenLevels } from "./lazy-file-tree";
import type { WorkspaceDirectoryEntry, WorkspaceFileItem } from "./generated/protocol";

afterEach(() => { cleanup(); rpc.mockReset(); vi.useRealTimers(); });

const entry = (relativePath: string, fileType = "file"): WorkspaceDirectoryEntry => ({
  relativePath,
  name: relativePath.split("/").at(-1) ?? relativePath,
  fileType,
  sizeBytes: fileType === "file" ? 1 : null,
  modifiedAt: 1,
});

const listing = (relativePath: string | null, entries: WorkspaceDirectoryEntry[], issueCount = 0) => ({ workspaceId: "ws", relativePath, entries, issueCount });

const indexed = (relativePath: string, localFileId: string) => ({ relativePath, localFileId, fileType: "file", presenceState: "present", displayState: null }) as unknown as WorkspaceFileItem;

function renderTree(overrides: Partial<Parameters<typeof LazyFileTree>[0]> = {}) {
  const handlers = { onFile: vi.fn(), onUnindexedFile: vi.fn(), onContext: vi.fn(), onDirectory: vi.fn() };
  const view = render(<LazyFileTree workspaceId="ws" workspaceKind="local" indexedFiles={[]} showHidden={false} selectedFileId={null} pendingPath={null} {...handlers} {...overrides}/>);
  return { ...handlers, ...view };
}

describe("逐层摊平已读到的目录", () => {
  it("未展开的目录不产出子行", () => {
    const levels = new Map([["", { state: "ready" as const, entries: [entry("docs", "directory"), entry("a.md")], issueCount: 0 }]]);
    const rows = flattenLevels(levels, new Set(), false);
    expect(rows.map((row) => row.kind === "entry" ? row.entry.relativePath : row.text)).toEqual(["docs", "a.md"]);
  });

  it("展开但还在读的目录显示一行读取中，读失败的给出原因", () => {
    const levels = new Map<string, Parameters<typeof flattenLevels>[0] extends Map<string, infer L> ? L : never>([
      ["", { state: "ready", entries: [entry("docs", "directory"), entry("gone", "directory")], issueCount: 0 }],
      ["docs", { state: "loading" }],
      ["gone", { state: "error", message: "这个文件夹已不存在或无法读取" }],
    ]);
    const rows = flattenLevels(levels, new Set(["docs", "gone"]), false);
    expect(rows.map((row) => row.kind === "entry" ? row.entry.relativePath : row.text)).toEqual(["docs", "正在读取…", "gone", "这个文件夹已不存在或无法读取"]);
  });

  it("隐藏以点开头的条目，除非打开了显示隐藏文件", () => {
    const levels = new Map([["", { state: "ready" as const, entries: [entry(".env"), entry("a.md")], issueCount: 0 }]]);
    expect(flattenLevels(levels, new Set(), false)).toHaveLength(1);
    expect(flattenLevels(levels, new Set(), true)).toHaveLength(2);
  });

  it("有读不到的条目时明确提示这一层不完整", () => {
    const levels = new Map([["", { state: "ready" as const, entries: [entry("a.md")], issueCount: 2 }]]);
    const rows = flattenLevels(levels, new Set(), false);
    expect(rows.at(-1)).toMatchObject({ kind: "status", text: "有 2 项无法读取", tone: "danger" });
  });
});

describe("直接读取本地目录的文件树", () => {
  it("先只读根目录一层，点开文件夹才读下一层", async () => {
    rpc.mockResolvedValueOnce(listing(null, [entry("docs", "directory"), entry("readme.md")]));
    renderTree();
    await screen.findByText("readme.md");
    expect(rpc).toHaveBeenCalledTimes(1);
    expect(rpc).toHaveBeenCalledWith("workspace/listDirectory", { workspaceId: "ws", relativePath: null });

    rpc.mockResolvedValueOnce(listing("docs", [entry("docs/plan.md")]));
    fireEvent.click(screen.getByText("docs"));
    await screen.findByText("plan.md");
    expect(rpc).toHaveBeenLastCalledWith("workspace/listDirectory", { workspaceId: "ws", relativePath: "docs" });

    rpc.mockResolvedValueOnce(listing("docs", [entry("docs/new.md")]));
    fireEvent.click(screen.getByText("docs"));
    fireEvent.click(screen.getByText("docs"));
    await screen.findByText("new.md");
    expect(screen.queryByText("plan.md")).toBeNull();
    expect(rpc).toHaveBeenCalledTimes(3);
  });

  it("已进索引的文件直接打开，未进索引的交给优先索引", async () => {
    rpc.mockResolvedValueOnce(listing(null, [entry("done.md"), entry("new.md")]));
    const handlers = renderTree({ indexedFiles: [indexed("done.md", "file-done")] });
    fireEvent.click(await screen.findByText("done.md"));
    expect(handlers.onFile).toHaveBeenCalledWith("file-done");
    fireEvent.click(screen.getByText("new.md"));
    expect(handlers.onUnindexedFile).toHaveBeenCalledWith("new.md");
  });

  it("已有索引不决定目录内容，磁盘新增和删除在下一次刷新直接显示", async () => {
    vi.useFakeTimers();
    rpc.mockResolvedValue(listing(null, [entry("old.md")]));
    await act(async () => { renderTree({ indexedFiles: [indexed("old.md", "file-old"), indexed("historic.md", "file-history")] }); });
    expect(screen.getByText("old.md")).toBeTruthy();
    expect(screen.queryByText("historic.md")).toBeNull();
    rpc.mockResolvedValue(listing(null, [entry("new.md")]));
    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(screen.getByText("new.md")).toBeTruthy();
    expect(screen.queryByText("old.md")).toBeNull();
  });

  it("恢复深层文件时展开父目录并读取当前目录", async () => {
    rpc.mockImplementation((_method: string, params: { relativePath: string | null }) => Promise.resolve(params.relativePath === "docs"
      ? listing("docs", [entry("docs/saved.md")])
      : listing(null, [entry("docs", "directory")])));
    renderTree({ indexedFiles: [indexed("docs/saved.md", "saved")], selectedFileId: "saved" });
    expect((await screen.findByText("saved.md")).closest("button")?.className).toContain("selected");
  });

  it("晚到的旧目录结果不能覆盖新结果", async () => {
    let finishOld!: (value: ReturnType<typeof listing>) => void;
    rpc.mockReturnValueOnce(new Promise((resolve) => { finishOld = resolve; }));
    const view = renderTree();
    rpc.mockResolvedValueOnce(listing(null, [entry("new.md")]));
    await act(async () => {
      view.rerender(<LazyFileTree workspaceId="ws" workspaceKind="local" indexedFiles={[]} showHidden={false} selectedFileId={null} pendingPath={null} refreshToken={1} onFile={view.onFile} onUnindexedFile={view.onUnindexedFile} onContext={view.onContext}/>);
    });
    await screen.findByText("new.md");
    await act(async () => { finishOld(listing(null, [entry("old.md")])); });
    expect(screen.queryByText("old.md")).toBeNull();
    expect(screen.getByText("new.md")).toBeTruthy();
  });

  it("正在优先索引的文件不能重复点击", async () => {
    rpc.mockResolvedValueOnce(listing(null, [entry("new.md")]));
    renderTree({ pendingPath: "new.md" });
    const button = (await screen.findByText("new.md")).closest("button");
    expect(button?.disabled).toBe(true);
    expect(button?.getAttribute("aria-busy")).toBe("true");
  });

  it("读目录失败时给出原因，重试后恢复", async () => {
    rpc.mockRejectedValueOnce(new Error("workspace_root_unavailable"));
    renderTree();
    await screen.findByText("工作区文件夹无法访问");
    rpc.mockResolvedValueOnce(listing(null, [entry("a.md")]));
    fireEvent.click(screen.getByText("重试"));
    await screen.findByText("a.md");
  });

  it.each([
    ["workspace_directory_busy", "这个目录仍在读取，请稍后重试"],
    ["workspace_directory_capacity_reached", "同时读取的目录较多，请稍后重试"],
    ["workspace_root_unavailable", "工作区文件夹无法访问"],
  ])("错误目录 %s 停止轮询，手动重试后恢复刷新", async (reason, label) => {
    vi.useFakeTimers();
    rpc.mockRejectedValueOnce(new Error(reason));
    await act(async () => { renderTree(); });
    expect(screen.getByText(label)).toBeTruthy();
    await act(async () => { await vi.advanceTimersByTimeAsync(9000); });
    expect(rpc).toHaveBeenCalledTimes(1);
    rpc.mockResolvedValue(listing(null, [entry("after.md")]));
    await act(async () => { fireEvent.click(screen.getByText("重试")); });
    expect(screen.getByText("after.md")).toBeTruthy();
    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(rpc).toHaveBeenCalledTimes(3);
  });

  it("错误子目录不自动重读，正常根目录继续刷新", async () => {
    vi.useFakeTimers();
    rpc.mockImplementation((_method: string, params: { relativePath: string | null }) => params.relativePath === "docs"
      ? Promise.reject(new Error("workspace_directory_busy"))
      : Promise.resolve(listing(null, [entry("docs", "directory")])));
    await act(async () => { renderTree(); });
    await act(async () => { fireEvent.click(screen.getByText("docs")); });
    expect(screen.getByText("这个目录仍在读取，请稍后重试")).toBeTruthy();
    await act(async () => { await vi.advanceTimersByTimeAsync(9000); });
    expect(rpc.mock.calls.filter((call) => call[1].relativePath === null)).toHaveLength(4);
    expect(rpc.mock.calls.filter((call) => call[1].relativePath === "docs")).toHaveLength(1);
  });

  it("显式刷新会重新读取错误目录", async () => {
    vi.useFakeTimers();
    rpc.mockRejectedValueOnce(new Error("workspace_directory_busy"));
    let view!: ReturnType<typeof renderTree>;
    await act(async () => { view = renderTree(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(9000); });
    expect(rpc).toHaveBeenCalledTimes(1);
    rpc.mockResolvedValue(listing(null, [entry("after.md")]));
    await act(async () => {
      view.rerender(<LazyFileTree workspaceId="ws" workspaceKind="local" indexedFiles={[]} showHidden={false} selectedFileId={null} pendingPath={null} refreshToken={1} onFile={view.onFile} onUnindexedFile={view.onUnindexedFile} onContext={view.onContext}/>);
    });
    expect(screen.getByText("after.md")).toBeTruthy();
    expect(rpc).toHaveBeenCalledTimes(2);
  });

  it("刷新时重读已读过的各层，展开状态保留", async () => {
    rpc.mockResolvedValueOnce(listing(null, [entry("docs", "directory")]));
    const view = renderTree();
    rpc.mockResolvedValueOnce(listing("docs", [entry("docs/a.md")]));
    fireEvent.click(await screen.findByText("docs"));
    await screen.findByText("a.md");

    rpc.mockImplementation((_method: string, params: { relativePath: string | null }) => Promise.resolve(params.relativePath === "docs"
      ? listing("docs", [entry("docs/a.md"), entry("docs/b.md")])
      : listing(null, [entry("docs", "directory")])));
    await act(async () => {
      view.rerender(<LazyFileTree workspaceId="ws" workspaceKind="local" indexedFiles={[]} showHidden={false} selectedFileId={null} pendingPath={null} refreshToken={1} onFile={view.onFile} onUnindexedFile={view.onUnindexedFile} onContext={view.onContext}/>);
    });
    await screen.findByText("b.md");
    expect(screen.getByText("a.md")).toBeTruthy();
  });
});
