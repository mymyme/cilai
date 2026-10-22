import { useEffect, useRef, useState } from "react";
import { rpc } from "./app-server-rpc";
import type { WorkspaceIndexPathResult, WorkspaceIndexStatusResult } from "./generated/protocol";

/** 索引正在进行时的轮询间隔：够看出进展，又不至于让进度查询占满连接。 */
export const ACTIVE_INDEX_POLL_MS = 1000;
/** 空闲时的轮询间隔：只为发现后台重扫失败，不需要很快。 */
export const IDLE_INDEX_POLL_MS = 5000;

const ACTIVE_PHASES = new Set(["queued", "scanning", "hashing", "committing"]);

export function isIndexActive(status: WorkspaceIndexStatusResult | null): boolean {
  return status !== null && ACTIVE_PHASES.has(status.phase);
}

const FAILURE_LABELS: Record<string, string> = {
  workspace_scan_incomplete: "部分文件或文件夹无法读取，索引没有更新",
  workspace_root_unavailable: "文件夹无法访问，可能已被移动或没有读取权限",
  workspace_root_not_directory: "工作区路径不是文件夹，索引没有更新",
  workspace_target_unavailable: "索引目标已不存在或无法读取，索引没有更新",
  workspace_path_outside: "索引目标超出工作区，索引没有更新",
  workspace_target_not_directory: "索引目标不是文件夹，索引没有更新",
  workspace_target_ignored: "索引目标位于忽略目录，索引没有更新",
  workspace_target_unsupported: "索引目标类型不受支持，索引没有更新",
  workspace_checksum_failed: "读取文件内容失败，索引没有更新",
  workspace_index_write_failed: "写入索引失败",
  workspace_watch_unavailable: "无法监听这个文件夹，可能已被移动或没有读取权限",
  workspace_index_unavailable: "后台索引服务不可用",
  workspace_not_found: "工作区已不存在",
};

export function indexFailureLabel(reason: string | null | undefined): string {
  return (reason && FAILURE_LABELS[reason]) ?? `建立索引失败（${reason ?? "原因未知"}）`;
}

/** 顶部进度行的文字。空闲时不显示任何东西，返回 null。 */
export function indexProgressLabel(status: WorkspaceIndexStatusResult | null): string | null {
  if (!status) return null;
  switch (status.phase) {
    case "queued": return "正在准备建立索引…";
    case "scanning": return status.entriesExamined > 0 ? `正在建立索引 · 已发现 ${status.entriesExamined.toLocaleString("zh-CN")} 项` : "正在建立索引…";
    case "hashing": return status.filesToHash > 0 ? `正在读取文件内容 · ${status.filesHashed.toLocaleString("zh-CN")} / ${status.filesToHash.toLocaleString("zh-CN")}` : "正在读取文件内容…";
    case "committing": return status.batchesTotal > 1 ? `正在写入索引 · 第 ${Math.min(status.batchesCommitted + 1, status.batchesTotal)} / ${status.batchesTotal} 批` : "正在写入索引…";
    case "failed": return indexFailureLabel(status.reason);
    default: return null;
  }
}

/** 请求后台立即为工作区建一次索引。不等它完成。 */
export async function requestWorkspaceIndex(workspaceId: string): Promise<void> {
  await rpc("workspace/indexRequest", { workspaceId });
}

/** 优先为单个条目建立索引记录，返回它的文件编号。 */
export function indexWorkspacePath(workspaceId: string, relativePath: string): Promise<WorkspaceIndexPathResult> {
  return rpc<WorkspaceIndexPathResult>("workspace/indexPath", { workspaceId, relativePath });
}

/**
 * 跟踪一个本地工作区的后台索引进度。
 *
 * 索引从进行中回到空闲（或失败）时调用 `onSettled`：外壳据此重读工作区和文件列表，
 * 文件树继续读取本地目录，索引只补充引用和内容状态。
 */
export function useWorkspaceIndexStatus(workspaceId: string | null, enabled: boolean, onSettled: () => void, refreshToken = 0) {
  const [status, setStatus] = useState<WorkspaceIndexStatusResult | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const settled = useRef(onSettled);
  settled.current = onSettled;

  useEffect(() => {
    setStatus(null);
    setPollError(null);
    if (!workspaceId || !enabled) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let wasActive = false;
    const poll = async () => {
      let next: WorkspaceIndexStatusResult | null = null;
      try {
        next = await rpc<WorkspaceIndexStatusResult>("workspace/indexStatus", { workspaceId });
        if (cancelled) return;
        setPollError(null);
        setStatus(next);
      } catch (error) {
        if (cancelled) return;
        setPollError(String(error));
      }
      const active = isIndexActive(next);
      if (wasActive && !active && next !== null) settled.current();
      wasActive = active;
      timer = setTimeout(() => void poll(), active ? ACTIVE_INDEX_POLL_MS : IDLE_INDEX_POLL_MS);
    };
    void poll();
    return () => { cancelled = true; if (timer) clearTimeout(timer); };
  }, [workspaceId, enabled, refreshToken]);

  return { status, pollError };
}
