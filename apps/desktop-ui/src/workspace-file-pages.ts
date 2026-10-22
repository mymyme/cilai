import { rpc } from "./app-server-rpc";
import type { WorkspaceFileItem, WorkspaceFilesResult } from "./generated/protocol";

/** 文件列表整轮重读的次数上限。超过说明目录一直在变，那才是真读不出来。 */
export const MAX_FILE_LOAD_ATTEMPTS = 4;

const PAGE_SIZE = 500;

export interface WorkspaceFileSnapshot {
  files: WorkspaceFileItem[];
  revision: number;
}

/** 每翻完一页调一次，返回 false 表示这次加载已被更新的一次取代，立即收手。 */
export type StillWanted = () => boolean;

/**
 * 读出一份修订号一致的完整文件列表。
 *
 * 大工作区要翻几十页，期间文件监听很可能重扫一次并推高工作区修订号。跨修订号拼起来的
 * 列表会漏项或重项，所以只能整轮重来。但这是后台的正常节奏，不是用户能处理的事，
 * 自动重来即可，不该弹提示打断人。
 */
export async function loadWorkspaceFileSnapshot(
  workspaceId: string,
  stillWanted: StillWanted = () => true,
): Promise<WorkspaceFileSnapshot | null> {
  for (let attempt = 0; attempt < MAX_FILE_LOAD_ATTEMPTS; attempt += 1) {
    const files: WorkspaceFileItem[] = [];
    let cursor: string | null = null;
    let revision: number | null = null;
    let changedMidway = false;

    while (true) {
      const page: WorkspaceFilesResult = await rpc<WorkspaceFilesResult>("workspace/files", { workspaceId, cursor, limit: PAGE_SIZE });
      if (!stillWanted()) return null;
      if (revision !== null && page.workspaceRevision !== revision) { changedMidway = true; break; }
      revision = page.workspaceRevision;
      files.push(...page.files);
      if (!page.hasMore || page.nextCursor === null) break;
      // 游标不前进说明服务端分页坏了，再翻下去就是死循环。
      if (page.nextCursor === cursor) throw new Error("文件列表分页游标无效。");
      cursor = page.nextCursor;
    }

    if (changedMidway) continue;
    return { files, revision: revision ?? 0 };
  }
  // 连着几轮都没读到一份完整快照，说明目录一直在变，这才是真的读不出来。
  throw new Error("目录持续变化，暂时读不到完整的文件列表。");
}
