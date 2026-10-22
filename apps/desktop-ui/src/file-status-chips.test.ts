import { describe, expect, it } from "vitest";
import { buildStatusChips } from "./file-status";
import type { WorkspaceFileItem } from "./generated/protocol";

const file = (over: Partial<WorkspaceFileItem>) => ({
  localChangeState: "clean",
  confirmationState: "not_required",
  conflictState: "none",
  ...over,
} as WorkspaceFileItem);

const labels = (f: WorkspaceFileItem | null, hasButton = false, workspaceKind = "cloud_team") => buildStatusChips(f, hasButton, workspaceKind).map((c) => c.label);

describe("文件状态胶囊", () => {
  it("未选择文件时不渲染任何胶囊", () => {
    expect(labels(null)).toEqual([]);
  });

  it("一切正常的文件不渲染任何胶囊，而不是三格都说没事", () => {
    expect(labels(file({}))).toEqual([]);
  });

  it("纯云端正常文件不显示内部状态码或本地同步待办", () => {
    expect(labels(file({ displayState: "cloud_ready", localChangeState: null, confirmationState: null }))).toEqual([]);
  });

  it("已确认同样属于无需处理，不占位", () => {
    expect(labels(file({ confirmationState: "confirmed" }))).toEqual([]);
  });

  it("只渲染需要注意的那一项", () => {
    expect(labels(file({ localChangeState: "added" }))).toEqual(["本地新增"]);
    expect(labels(file({ conflictState: "content_diverged" }))).toEqual(["内容冲突"]);
  });

  it("状态计算中只显示一个当前状态", () => {
    expect(labels(file({ localChangeState: null, confirmationState: null }))).toEqual(["状态计算中"]);
  });

  it("待确认按钮在场时，确认胶囊和本地变化胶囊都不再重复一遍待办", () => {
    expect(labels(file({ localChangeState: "added", confirmationState: "pending" }), true)).toEqual([]);
    expect(labels(file({ localChangeState: "modified", confirmationState: "pending" }), true)).toEqual([]);
  });

  it("确认后的文件属于正常可信文档，不再显示本地新增", () => {
    expect(labels(file({ displayState: "confirmed_ready", localChangeState: "added", confirmationState: "confirmed" }))).toEqual([]);
    expect(labels(file({ localChangeState: "added", confirmationState: "confirmed" }))).toEqual([]);
  });

  it("按钮不在场时优先显示冲突，不堆叠确认状态", () => {
    expect(labels(file({ confirmationState: "pending", conflictState: "content_diverged" }), false))
      .toEqual(["内容冲突"]);
  });

  it("多项异常时只显示最高优先级状态", () => {
    expect(labels(file({ localChangeState: "modified", confirmationState: "blocked_by_permission", conflictState: "unresolved" })))
      .toEqual(["冲突待处理"]);
  });

  it("本地文件不因旧确认或本地修改显示待办，但仍显示读取异常", () => {
    expect(labels(file({ displayState: "confirmation_required", localChangeState: "added", confirmationState: "pending" }), false, "local")).toEqual([]);
    expect(labels(file({ displayState: "local_modified", localChangeState: "modified" }), false, "local")).toEqual([]);
    expect(labels(file({ displayState: "unavailable", localChangeState: "unreadable" }), false, "local")).toEqual(["文件不可用"]);
  });
});
