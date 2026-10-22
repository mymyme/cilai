import type { WorkspaceFileItem } from "./generated/protocol";

/**
 * 文件状态的措辞与配色。同步状态在协议里是几个互相独立的字段（本地变化、确认、冲突），
 * 界面上要合成一句人话；这套映射此前和应用主体混在一起，改文案得先在千行文件里找到它们。
 */
export type StatusTone = "neutral" | "success" | "attention" | "danger";

export function buildStatusChips(file: WorkspaceFileItem | null, confirmationHasButton: boolean, workspaceKind: string): { label: string; tone: StatusTone }[] {
  if (!file) return [];
  const displayStatus = fileDisplayStatus(file.displayState, workspaceKind);
  if (workspaceKind === "local") return displayStatus ? [displayStatus] : [];
  if (displayStatus) {
    if (confirmationHasButton && file.displayState === "confirmation_required") return [];
    return [displayStatus];
  }
  if (file.displayState === "synced" || file.displayState === "confirmed_ready" || file.displayState === "cloud_ready") return [];
  if (file.conflictState !== null && file.conflictState !== "none") {
    return [{ label: conflictLabel(file.conflictState), tone: conflictTone(file.conflictState) }];
  }
  if (file.confirmationState?.startsWith("blocked_")) {
    return [{ label: confirmationLabel(file.confirmationState), tone: confirmationTone(file.confirmationState) }];
  }
  if (file.localChangeState === null || file.localChangeState === "scanning" || file.localChangeState === "unreadable") {
    return [{ label: localChangeLabel(file.localChangeState), tone: localChangeTone(file.localChangeState) }];
  }
  if (confirmationNeedsAction(file.confirmationState)) {
    return confirmationHasButton ? [] : [{ label: confirmationLabel(file.confirmationState), tone: confirmationTone(file.confirmationState) }];
  }
  if (file.confirmationState === "confirmed" || (file.confirmationState === "not_required" && file.localChangeState === "clean")) return [];
  return file.localChangeState === "clean"
    ? []
    : [{ label: localChangeLabel(file.localChangeState), tone: localChangeTone(file.localChangeState) }];
}

const displayStatusLabels: Record<string, { label: string; tone: StatusTone }> = {
  metadata_error: { label: "元数据异常", tone: "danger" },
  permission_revoked: { label: "权限已失效", tone: "danger" },
  sync_conflict: { label: "同步冲突", tone: "danger" },
  outcome_unknown: { label: "提交结果未知", tone: "danger" },
  review_returned: { label: "评审已退回", tone: "danger" },
  pending_review: { label: "等待评审", tone: "attention" },
  confirmation_required: { label: "等待确认", tone: "attention" },
  platform_update: { label: "平台有更新", tone: "attention" },
  local_missing: { label: "本地文件缺失", tone: "danger" },
  local_modified: { label: "本地已修改", tone: "attention" },
  local_added: { label: "本地新增", tone: "attention" },
  not_materialized: { label: "文件尚未下载", tone: "neutral" },
  scanning: { label: "正在扫描", tone: "neutral" },
  unavailable: { label: "文件不可用", tone: "danger" },
};

export function fileDisplayStatus(state: string | null, workspaceKind: string): { state: string; label: string; tone: StatusTone } | null {
  if (!state || state === "synced" || state === "confirmed_ready" || state === "local_ready" || state === "cloud_ready") return null;
  if (workspaceKind === "local" && !["metadata_error", "local_missing", "not_materialized", "scanning", "unavailable"].includes(state)) return null;
  return { state, ...(displayStatusLabels[state] ?? { label: state, tone: "neutral" }) };
}

export function documentStatusLabel(file: WorkspaceFileItem): string {
  if (file.confirmationState === "confirmed" || (file.confirmationState === "not_required" && file.localChangeState === "clean")) {
    return "可信文档";
  }
  return confirmationLabel(file.confirmationState);
}

export function StatusChip({ label, tone }: { label: string; tone: "neutral" | "success" | "attention" | "danger" }) { return <span className="file-status-chip" data-tone={tone}><i/>{label}</span>; }
export function localStatusLabel(status: string) { return ({ creating: "正在创建", scanning: "正在扫描", ready: "可用", attention_required: "需处理", unavailable: "目录不可用", invalid: "状态异常" } as Record<string, string>)[status] ?? status; }
export function presenceLabel(state: string) { return ({ present: "文件存在", missing: "本地缺失", inaccessible: "无法访问" } as Record<string, string>)[state] ?? state; }
export function localChangeLabel(state: string | null) { return state === null ? "状态计算中" : ({ clean: "本地未变化", added: "本地新增", modified: "本地已修改", deleted_candidate: "本地待删除", renamed_candidate: "本地已重命名", unreadable: "本地不可读", scanning: "正在扫描" } as Record<string, string>)[state] ?? state; }
export function confirmationLabel(state: string | null) { return state === null ? "确认状态计算中" : ({ not_required: "无需确认", pending: "等待确认", confirmed: "已确认", invalidated_content_changed: "内容变化，需重确认", invalidated_baseline_changed: "基线变化，需重确认", blocked_by_conflict: "冲突阻止确认", blocked_by_permission: "权限阻止确认" } as Record<string, string>)[state] ?? state; }
export function conflictLabel(state: string | null) { return state === null || state === "none" ? "无冲突" : ({ update_available: "平台有更新", content_diverged: "内容冲突", delete_modify: "删除与修改冲突", rename_conflict: "重命名冲突", path_collision: "路径冲突", case_collision: "大小写冲突", permission_conflict: "权限冲突", metadata_conflict: "元数据冲突", unresolved: "冲突待处理" } as Record<string, string>)[state] ?? state; }
export function localChangeTone(state: string | null): "neutral" | "success" | "attention" | "danger" { if (state === "clean") return "success"; if (state === "unreadable") return "danger"; return state === null || state === "scanning" ? "neutral" : "attention"; }
export function confirmationTone(state: string | null): "neutral" | "success" | "attention" | "danger" { if (state === "confirmed" || state === "not_required") return "success"; if (state?.startsWith("blocked_")) return "danger"; return state === null ? "neutral" : "attention"; }
export function conflictTone(state: string | null): "neutral" | "success" | "attention" | "danger" { if (state === null || state === "none") return "neutral"; if (state === "update_available") return "attention"; return "danger"; }
export function confirmationNeedsAction(state: string | null) { return state === "pending" || state?.startsWith("invalidated_") === true; }
