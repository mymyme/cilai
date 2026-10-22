import { describe, expect, it } from "vitest";
import {
  initialWorkspaceState, pickSelectedFile, reconcileSelectedFile, reconcileSelectedWorkspace, workspaceReducer,
  type WorkspaceState,
} from "./workspace-state";
import type { ConflictItem, WorkspaceFileItem, WorkspaceSummary } from "./generated/protocol";

const ws = (id: string) => ({ workspaceId: id, displayName: id, localRoot: `/tmp/${id}` } as WorkspaceSummary);
const file = (id: string, over: Partial<WorkspaceFileItem> = {}) => ({
  localFileId: id, relativePath: `${id}.md`, fileType: "file", presenceState: "present", ...over,
} as WorkspaceFileItem);

const loaded = (over: Partial<WorkspaceState> = {}): WorkspaceState => ({
  ...initialWorkspaceState,
  workspaces: [ws("a"), ws("b")],
  selectedWorkspaceId: "a",
  files: [file("f1"), file("f2")],
  workspaceRevision: 7,
  selectedFileId: "f1",
  conflicts: [{ conflictId: "c1" } as ConflictItem],
  loadState: "ready",
  ...over,
});

describe("切换工作区", () => {
  it("同时清掉文件、选中项、修订号和冲突：留下任何一样都会拿旧数据配新标题", () => {
    const next = workspaceReducer(loaded(), { type: "workspaceSelected", workspaceId: "b" });
    expect(next.selectedWorkspaceId).toBe("b");
    expect(next.files).toEqual([]);
    expect(next.selectedFileId).toBeNull();
    expect(next.workspaceRevision).toBe(0);
    expect(next.conflicts).toEqual([]);
  });

  it("选中的还是同一个时原样返回，不触发无谓的重新加载", () => {
    const state = loaded();
    expect(workspaceReducer(state, { type: "workspaceSelected", workspaceId: "a" })).toBe(state);
  });

  it("清空选择同样要清干净", () => {
    const next = workspaceReducer(loaded(), { type: "workspaceSelected", workspaceId: null });
    expect(next.selectedWorkspaceId).toBeNull();
    expect(next.files).toEqual([]);
  });
});

describe("工作区清单重读后选谁", () => {
  it("定时读到相同清单时保留状态与清单引用，不触发项目列表重载", () => {
    const state = loaded();
    const next = workspaceReducer(state, { type: "workspacesLoaded", workspaces: state.workspaces.map((workspace) => ({ ...workspace })), persistedId: "a" });
    expect(next).toBe(state);
    expect(next.workspaces).toBe(state.workspaces);
  });

  it.each([{ displayName: "新名称" }, { permissionRole: "viewer" }, { stateRevision: 3 }, { localRoot: "/tmp/moved" }])("清单内容变化时仍更新：%j", (change) => {
    const state = loaded();
    const workspaces = state.workspaces.map((workspace, index) => index === 0 ? { ...workspace, ...change } : workspace);
    const next = workspaceReducer(state, { type: "workspacesLoaded", workspaces, persistedId: "a" });
    expect(next.workspaces).toEqual(workspaces);
    expect(next.workspaces).not.toBe(state.workspaces);
    expect(next.selectedFileId).toBe(state.selectedFileId);
  });

  it("当前这个还在就不动：清单每次事件回放都会重读，跳走会把用户刚切的工作区弹回来", () => {
    expect(reconcileSelectedWorkspace("b", "a", [ws("a"), ws("b")])).toBe("b");
  });

  it("当前这个没了才用上次记住的", () => {
    expect(reconcileSelectedWorkspace("gone", "b", [ws("a"), ws("b")])).toBe("b");
  });

  it("记住的那个也没了才退到第一个", () => {
    expect(reconcileSelectedWorkspace("gone", "also-gone", [ws("a"), ws("b")])).toBe("a");
  });

  it("一个工作区都没有时是 null，不是崩溃", () => {
    expect(reconcileSelectedWorkspace("gone", "also-gone", [])).toBeNull();
  });

  it("读到清单即进入 ready", () => {
    const next = workspaceReducer(initialWorkspaceState, { type: "workspacesLoaded", workspaces: [ws("a")], persistedId: null });
    expect(next.loadState).toBe("ready");
    expect(next.selectedWorkspaceId).toBe("a");
  });

  it("当前目录从本地消失时清除旧文件及预览选择", () => {
    const next = workspaceReducer(loaded(), { type: "workspacesLoaded", workspaces: [ws("b")], persistedId: "a" });
    expect(next.selectedWorkspaceId).toBe("b");
    expect(next.files).toEqual([]);
    expect(next.selectedFileId).toBeNull();
    expect(next.conflicts).toEqual([]);
  });

  it("读取失败进入 error，已有的工作区不清空", () => {
    const next = workspaceReducer(loaded(), { type: "workspaceListFailed", error: new Error("读不到") });
    expect(next.loadState).toBe("error");
    expect(next.workspaces).toHaveLength(2);
  });

  it("失败原因要留下来：界面得说清是读不出来，而不是这里是空的", () => {
    const cause = new Error("本地服务没有响应");
    const next = workspaceReducer(loaded(), { type: "workspaceListFailed", error: cause });
    expect(next.loadError).toBe(cause);
  });

  it("重试成功后清掉失败原因，否则界面会一直挂着一条早已过期的错误", () => {
    const failed = workspaceReducer(loaded(), { type: "workspaceListFailed", error: new Error("x") });
    const next = workspaceReducer(failed, { type: "workspacesLoaded", workspaces: [ws("a")], persistedId: null });
    expect(next.loadState).toBe("ready");
    expect(next.loadError).toBeNull();
  });
});

describe("选中文件的降级顺序", () => {
  it("当前这个还在就保持：列表会因扫描反复重建，每次跳回第一个会把用户看的文件挤走", () => {
    expect(pickSelectedFile("f2", [file("f1"), file("f2")])).toBe("f2");
  });

  it("当前这个没了就挑第一个可预览的，而不是第一个文件", () => {
    const files = [file("bin", { relativePath: "a.bin" }), file("doc", { relativePath: "b.md" })];
    expect(pickSelectedFile("gone", files)).toBe("doc");
  });

  it("没有可预览的才退到第一个存在的文件", () => {
    const files = [file("bin", { relativePath: "a.bin" }), file("bin2", { relativePath: "b.bin" })];
    expect(pickSelectedFile("gone", files)).toBe("bin");
  });

  it("目录与缺失的文件不参与挑选", () => {
    const files = [
      file("dir", { fileType: "directory", relativePath: "d" }),
      file("missing", { presenceState: "missing", relativePath: "m.md" }),
    ];
    expect(pickSelectedFile("gone", files)).toBeNull();
  });

  it("空列表返回 null", () => {
    expect(pickSelectedFile("f1", [])).toBeNull();
  });
});

describe("文件列表重读后恢复选择", () => {
  it("当前文件仍存在时优先保持当前文件", () => {
    expect(reconcileSelectedFile("f2", "f1", [file("f1"), file("f2")])).toBe("f2");
  });

  it("当前文件为空时恢复这个工作区上次打开的文件", () => {
    expect(reconcileSelectedFile(null, "f2", [file("f1"), file("f2")])).toBe("f2");
  });

  it("已保存文件已经不存在时才按默认顺序挑选", () => {
    expect(reconcileSelectedFile(null, "gone", [file("f1"), file("f2")])).toBe("f1");
  });
});

describe("显示过滤变化", () => {
  it("选中项被过滤掉时改选一个仍然可见的", () => {
    const next = workspaceReducer(loaded(), { type: "visibleFilesChanged", visible: [file("f2")] });
    expect(next.selectedFileId).toBe("f2");
  });

  it("选中项仍然可见时原样返回，避免无谓的重渲染", () => {
    const state = loaded();
    expect(workspaceReducer(state, { type: "visibleFilesChanged", visible: [file("f1"), file("f2")] })).toBe(state);
  });
});

describe("文件列表读回来", () => {
  it("同时更新修订号，并按降级顺序确定选中项", () => {
    const next = workspaceReducer(loaded({ selectedFileId: "gone" }), {
      type: "filesLoaded", files: [file("f9")], revision: 12, persistedId: null,
    });
    expect(next.workspaceRevision).toBe(12);
    expect(next.selectedFileId).toBe("f9");
  });
});
