import type { WorkspaceFileItem, WorkspaceSummary } from "./generated/protocol";
import type { ContextMenuItem } from "./context-menu";
import { hostPlatformUi } from "./host-platform-ui";

/** 对一个文件能做的事。做法留在调用方，这份清单只回答"有哪些、什么顺序、什么时候不可用"。 */
export interface FileActions {
  preview: (file: WorkspaceFileItem) => void;
  showDiff: (file: WorkspaceFileItem) => void;
  openDetail: (file: WorkspaceFileItem) => void;
  discussWithAgent: (file: WorkspaceFileItem) => void;
  copy: (value: string, what: string) => void;
  openInSystem: (relativePath: string, openMode: "default" | "finder") => void;
  rescan: () => void;
  requestDelete: (file: WorkspaceFileItem) => void;
}

/**
 * 文件右键菜单的内容。
 *
 * 分组顺序是刻意的：先看（预览、差异），再复制路径，再交给系统，最后才是删除——
 * 破坏性的动作单独垫在末尾，免得和上面高频的动作挨在一起被误点。
 *
 * 目录没有预览和差异这些概念，整段跳过而不是逐条置灰：一串灰掉的菜单项等于让
 * 用户自己去猜为什么不能用。
 */
export function fileContextMenuItems(
  file: WorkspaceFileItem,
  workspace: WorkspaceSummary | null,
  workspaceBusy: boolean,
  actions: FileActions,
  hostPlatform: string | null,
): ContextMenuItem[] {
  const directory = file.fileType === "directory";
  const cloud = workspace?.workspaceKind === "cloud_team";
  const readOnly = cloud && workspace?.permissionRole === "viewer";
  const previewable = !directory && file.fileType === "file" && file.presenceState === "present";
  const absolutePath = workspace ? cloud ? `${workspace.displayName}/${file.relativePath}` : `${workspace.localRoot}/${file.relativePath}` : file.relativePath;
  const openDetail = () => actions.openDetail(file);
  const platformUi = hostPlatformUi(hostPlatform);
  return [
    ...(directory ? [] : [
      { label: "在应用内预览", disabled: !previewable, onSelect: () => actions.preview(file) },
      { label: "查看与基线的差异", disabled: !file.baselineVersionId, onSelect: () => actions.showDiff(file) },
      { separator: true } as const,
      { label: "与智能体讨论这个文件", disabled: !previewable || !workspace, onSelect: () => actions.discussWithAgent(file) },
    ]),
    { separator: true },
    { label: "复制相对路径", onSelect: () => void actions.copy(file.relativePath, "相对路径") },
    { label: cloud ? "复制云端路径" : "复制完整路径", onSelect: () => void actions.copy(absolutePath, cloud ? "云端路径" : "完整路径") },
    ...(file.checksumValue ? [{ label: "复制内容校验值", onSelect: () => void actions.copy(file.checksumValue!, "校验值") }] : []),
    { separator: true },
    ...(cloud ? [] : [{ label: "用系统默认程序打开", onSelect: () => void actions.openInSystem(file.relativePath, "default") }]),
    ...(platformUi && !cloud ? [{ label: `在${platformUi.fileManagerName}中显示`, onSelect: () => void actions.openInSystem(file.relativePath, "finder") }] : []),
    { separator: true },
    ...(directory ? [] : [{ label: "文件详情", onSelect: openDetail } as const]),
    { label: cloud ? "刷新云端空间" : "重新扫描工作区", disabled: workspaceBusy, onSelect: () => actions.rescan() },
    { separator: true },
    { label: directory ? "删除文件夹" : "删除文件", danger: true, disabled: !workspace || readOnly, onSelect: () => actions.requestDelete(file) },
  ];
}
