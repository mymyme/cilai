import type { ConflictItem, WorkspaceFileItem, WorkspaceSummary } from "./generated/protocol";

export type LoadState = "loading" | "ready" | "error";

/**
 * 工作区与它的文件列表。
 *
 * 这几样东西此前是七个各自独立的 useState，散在十几个写入点上。问题不在数量，
 * 在于它们之间有约束而语法上看不出来：切换工作区必须同时清掉文件、选中项、修订号
 * 和冲突列表——漏掉任何一样，界面就会拿旧工作区的数据配新工作区的标题。
 * 收成一个 reducer 之后，这条约束是一个 case 而不是四行相邻的 setter。
 */
export interface WorkspaceState {
  workspaces: WorkspaceSummary[];
  selectedWorkspaceId: string | null;
  files: WorkspaceFileItem[];
  workspaceRevision: number;
  selectedFileId: string | null;
  conflicts: ConflictItem[];
  loadState: LoadState;
  /** 读取失败的原因。界面要把它显示出来——"读不出来"和"这里是空的"必须长得不一样。 */
  loadError: unknown;
}

export const initialWorkspaceState: WorkspaceState = {
  workspaces: [],
  selectedWorkspaceId: null,
  files: [],
  workspaceRevision: 0,
  selectedFileId: null,
  conflicts: [],
  loadState: "loading",
  loadError: null,
};

export type WorkspaceAction =
  /** 工作区清单读到了。persistedId 是上次退出时选中的那个。 */
  | { type: "workspacesLoaded"; workspaces: WorkspaceSummary[]; persistedId: string | null }
  | { type: "workspaceListFailed"; error: unknown }
  | { type: "workspaceSelected"; workspaceId: string | null }
  | { type: "filesLoaded"; files: WorkspaceFileItem[]; revision: number; persistedId: string | null }
  | { type: "fileSelected"; localFileId: string | null }
  | { type: "conflictsLoaded"; conflicts: ConflictItem[] }
  /** 显示过滤（隐藏文件开关、搜索）变化后重新挑一个仍然可见的文件。 */
  | { type: "visibleFilesChanged"; visible: WorkspaceFileItem[] };

export function workspaceReducer(state: WorkspaceState, action: WorkspaceAction): WorkspaceState {
  switch (action.type) {
    case "workspacesLoaded": {
      const selectedWorkspaceId = reconcileSelectedWorkspace(state.selectedWorkspaceId, action.persistedId, action.workspaces);
      const selected = workspaceReducer(state, { type: "workspaceSelected", workspaceId: selectedWorkspaceId });
      const unchanged = state.workspaces.length === action.workspaces.length && state.workspaces.every((workspace, index) => {
        const next = action.workspaces[index];
        const keys = Object.keys(workspace) as (keyof WorkspaceSummary)[];
        return keys.length === Object.keys(next).length && keys.every((key) => workspace[key] === next[key]);
      });
      if (unchanged && selected === state && state.loadState === "ready" && state.loadError === null) return state;
      return {
        ...selected,
        workspaces: unchanged ? state.workspaces : action.workspaces,
        selectedWorkspaceId,
        loadState: "ready",
        loadError: null,
      };
    }
    case "workspaceListFailed":
      return { ...state, loadState: "error", loadError: action.error };
    case "workspaceSelected":
      // 换工作区就是换一整套事实。旧的文件、选中项、修订号和冲突都不再成立，
      // 留下任何一样都会让界面拿旧数据配新标题。
      if (action.workspaceId === state.selectedWorkspaceId) return state;
      return {
        ...state,
        selectedWorkspaceId: action.workspaceId,
        files: [],
        workspaceRevision: 0,
        selectedFileId: null,
        conflicts: [],
      };
    case "filesLoaded":
      return {
        ...state,
        files: action.files,
        workspaceRevision: action.revision,
        selectedFileId: reconcileSelectedFile(state.selectedFileId, action.persistedId, action.files),
      };
    case "fileSelected":
      return { ...state, selectedFileId: action.localFileId };
    case "conflictsLoaded":
      return { ...state, conflicts: action.conflicts };
    case "visibleFilesChanged": {
      const next = pickSelectedFile(state.selectedFileId, action.visible);
      return next === state.selectedFileId ? state : { ...state, selectedFileId: next };
    }
  }
}

/**
 * 选中项按"保持 → 可预览 → 存在 → 无"降级。
 *
 * 保持优先是因为文件列表会因为扫描、事件回放反复重建，每次都跳回第一个会把
 * 用户正在看的文件挤走。
 */
export function pickSelectedFile(current: string | null, files: WorkspaceFileItem[]): string | null {
  if (current !== null && files.some((item) => item.localFileId === current)) return current;
  return files.find(isPreviewableFile)?.localFileId ?? files.find(isPresentFile)?.localFileId ?? null;
}

/**
 * 文件列表读回后，先保留当前选择，再恢复这个工作区上次打开的文件，最后才自动挑选。
 */
export function reconcileSelectedFile(
  current: string | null,
  persistedId: string | null,
  files: WorkspaceFileItem[],
): string | null {
  if (current !== null && files.some((item) => item.localFileId === current)) return current;
  if (persistedId !== null && files.some((item) => item.localFileId === persistedId)) return persistedId;
  return pickSelectedFile(null, files);
}

/**
 * 选哪个工作区：当前这个仍在就不动，否则用上次退出时记住的，再否则第一个。
 *
 * 顺序是有讲究的：优先保持当前，是因为工作区清单会在每次事件回放后重读，
 * 若每次都跳回记住的那个，用户刚切过去的工作区会被弹回来。
 */
export function reconcileSelectedWorkspace(
  current: string | null,
  persistedId: string | null,
  workspaces: WorkspaceSummary[],
): string | null {
  const exists = (id: string | null) => id !== null && workspaces.some((item) => item.workspaceId === id);
  if (exists(current)) return current;
  if (exists(persistedId)) return persistedId;
  return workspaces[0]?.workspaceId ?? null;
}

export function isPresentFile(file: WorkspaceFileItem): boolean {
  return file.fileType === "file" && file.presenceState === "present";
}

export function isPreviewablePath(path: string) { const extension = path.split(".").at(-1)?.toLowerCase(); return extension !== undefined && previewableExtensions.has(extension); }

export function isPreviewableFile(file: WorkspaceFileItem) { return isPresentFile(file) && isPreviewablePath(file.relativePath); }

const previewableExtensions = new Set(["md", "markdown", "mkd", "pdf", "txt", "log", "csv", "tsv", "json", "jsonl", "yaml", "yml", "toml", "xml", "ini", "conf", "properties", "sql", "sh", "bash", "zsh", "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "kt", "kts", "swift", "c", "h", "cc", "cpp", "hpp", "cs", "html", "css", "scss"]);
