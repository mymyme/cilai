import { afterEach, describe, expect, it } from "vitest";
import { applyTheme, readTheme, themeStorageKey } from "./theme";

afterEach(() => {
  window.localStorage.clear();
  delete document.documentElement.dataset.theme;
  document.documentElement.style.colorScheme = "";
});

describe("应用主题", () => {
  it("读取已保存的浅色或深色选择", () => {
    window.localStorage.setItem(themeStorageKey, "light");
    expect(readTheme()).toBe("light");
    window.localStorage.setItem(themeStorageKey, "dark");
    expect(readTheme()).toBe("dark");
  });

  it("切换后同时更新页面主题并持久化", () => {
    applyTheme("light");
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.style.colorScheme).toBe("light");
    expect(window.localStorage.getItem(themeStorageKey)).toBe("light");
  });
});
