export interface HostPlatformUi {
  fileManagerName: "访达" | "文件管理器";
  globalSearchShortcut: "⌘K" | "Ctrl+K";
  runtimeName: "macOS" | "Windows";
  trashName: "废纸篓" | "回收站";
}

const macosUi: HostPlatformUi = {
  fileManagerName: "访达",
  globalSearchShortcut: "⌘K",
  runtimeName: "macOS",
  trashName: "废纸篓",
};

const windowsUi: HostPlatformUi = {
  fileManagerName: "文件管理器",
  globalSearchShortcut: "Ctrl+K",
  runtimeName: "Windows",
  trashName: "回收站",
};

/** 平台文案只接受后端明确返回的正向枚举；未知值不借用任一已知平台。 */
export function hostPlatformUi(platform: string | null | undefined): HostPlatformUi | null {
  if (platform === "macos") return macosUi;
  if (platform === "windows") return windowsUi;
  return null;
}

export function matchesGlobalSearchShortcut(
  platform: string | null | undefined,
  event: Pick<KeyboardEvent, "altKey" | "ctrlKey" | "key" | "metaKey" | "shiftKey">,
) {
  if (event.key.toLocaleLowerCase() !== "k" || event.altKey || event.shiftKey) return false;
  if (platform === "macos") return event.metaKey && !event.ctrlKey;
  if (platform === "windows") return event.ctrlKey && !event.metaKey;
  return false;
}
