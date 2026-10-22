import { describe, expect, it } from "vitest";
import { hostPlatformUi, matchesGlobalSearchShortcut } from "./host-platform-ui";

const shortcut = (overrides: Partial<KeyboardEvent> = {}) => ({
  altKey: false,
  ctrlKey: false,
  key: "k",
  metaKey: false,
  shiftKey: false,
  ...overrides,
} as KeyboardEvent);

describe("宿主平台界面文案", () => {
  it("macOS 使用命令键、访达和废纸篓", () => {
    expect(hostPlatformUi("macos")).toEqual({
      fileManagerName: "访达",
      globalSearchShortcut: "⌘K",
      runtimeName: "macOS",
      trashName: "废纸篓",
    });
    expect(matchesGlobalSearchShortcut("macos", shortcut({ metaKey: true }))).toBe(true);
    expect(matchesGlobalSearchShortcut("macos", shortcut({ ctrlKey: true }))).toBe(false);
  });

  it("Windows 使用 Ctrl、文件管理器和回收站", () => {
    expect(hostPlatformUi("windows")).toEqual({
      fileManagerName: "文件管理器",
      globalSearchShortcut: "Ctrl+K",
      runtimeName: "Windows",
      trashName: "回收站",
    });
    expect(matchesGlobalSearchShortcut("windows", shortcut({ ctrlKey: true }))).toBe(true);
    expect(matchesGlobalSearchShortcut("windows", shortcut({ metaKey: true }))).toBe(false);
  });

  it("未知平台不显示平台文案，也不注册任一平台快捷键", () => {
    expect(hostPlatformUi(null)).toBeNull();
    expect(hostPlatformUi("other")).toBeNull();
    expect(matchesGlobalSearchShortcut("other", shortcut({ ctrlKey: true }))).toBe(false);
    expect(matchesGlobalSearchShortcut("other", shortcut({ metaKey: true }))).toBe(false);
  });

  it("带额外修饰键时不误触发搜索", () => {
    expect(matchesGlobalSearchShortcut("macos", shortcut({ metaKey: true, shiftKey: true }))).toBe(false);
    expect(matchesGlobalSearchShortcut("windows", shortcut({ altKey: true, ctrlKey: true }))).toBe(false);
  });
});
