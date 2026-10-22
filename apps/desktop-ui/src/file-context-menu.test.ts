import { describe, expect, it, vi } from "vitest";
import { fileContextMenuItems, type FileActions } from "./file-context-menu";
import type { WorkspaceFileItem, WorkspaceSummary } from "./generated/protocol";

const workspace = { workspaceId: "ws-1", displayName: "主工作区", localRoot: "/tmp/ws" } as WorkspaceSummary;

const file = (over: Partial<WorkspaceFileItem> = {}) => ({
  localFileId: "file-1",
  relativePath: "01合同/合同文件.md",
  fileType: "file",
  presenceState: "present",
  baselineVersionId: "ver-1",
  checksumValue: "abc",
  confirmationState: "pending",
  ...over,
} as WorkspaceFileItem);

const noop: FileActions = {
  preview: () => {}, showDiff: () => {}, openDetail: () => {}, discussWithAgent: () => {},
  copy: () => {}, openInSystem: () => {}, rescan: () => {}, requestDelete: () => {},
};

type Action = Exclude<ReturnType<typeof fileContextMenuItems>[number], { separator: true }>;
const actionsOf = (items: ReturnType<typeof fileContextMenuItems>) => items.filter((item): item is Action => !item.separator);
const labels = (items: ReturnType<typeof fileContextMenuItems>) => actionsOf(items).map((item) => item.label);
const find = (items: ReturnType<typeof fileContextMenuItems>, label: string) => actionsOf(items).find((item) => item.label === label);

describe("文件右键菜单", () => {
  it("删除排在最后：破坏性动作不与高频动作挨在一起", () => {
    const names = labels(fileContextMenuItems(file(), workspace, false, noop, "macos"));
    expect(names.at(-1)).toBe("删除文件");
  });

  it("目录整段跳过预览与确认，而不是留一串灰掉的菜单项", () => {
    const names = labels(fileContextMenuItems(file({ fileType: "directory" }), workspace, false, noop, "macos"));
    expect(names).not.toContain("在应用内预览");
    expect(names).not.toContain("确认当前版本");
    expect(names).not.toContain("文件详情");
    expect(names.at(-1)).toBe("删除文件夹");
  });

  it("没有基线时不能看差异，没有校验值时不提供复制校验值", () => {
    const items = fileContextMenuItems(file({ baselineVersionId: null, checksumValue: null }), workspace, false, noop, "macos");
    expect(find(items, "查看与基线的差异")?.disabled).toBe(true);
    expect(labels(items)).not.toContain("复制内容校验值");
  });

  it("不展示不能进入确认流程的菜单入口", () => {
    const names = labels(fileContextMenuItems(file(), workspace, false, noop, "macos"));
    expect(names).not.toContain("确认当前版本");
    expect(names).not.toContain("查看确认历史");
  });

  it("没有工作区时不能删除：删除针对的是磁盘上的文件", () => {
    expect(find(fileContextMenuItems(file(), null, false, noop, "macos"), "删除文件")?.disabled).toBe(true);
  });

  it("完整路径由工作区根拼出，没有工作区时退回相对路径", () => {
    const copy = vi.fn();
    const pick = (ws: WorkspaceSummary | null) =>
      find(fileContextMenuItems(file(), ws, false, { ...noop, copy }, "macos"), "复制完整路径")?.onSelect();
    pick(workspace);
    expect(copy).toHaveBeenLastCalledWith("/tmp/ws/01合同/合同文件.md", "完整路径");
    pick(null);
    expect(copy).toHaveBeenLastCalledWith("01合同/合同文件.md", "完整路径");
  });

  it("文件定位入口按明确平台命名，未知平台不展示", () => {
    expect(labels(fileContextMenuItems(file(), workspace, false, noop, "macos"))).toContain("在访达中显示");
    expect(labels(fileContextMenuItems(file(), workspace, false, noop, "windows"))).toContain("在文件管理器中显示");
    expect(labels(fileContextMenuItems(file(), workspace, false, noop, "other")).some((label) => label.startsWith("在") && label.endsWith("中显示"))).toBe(false);
  });
});
