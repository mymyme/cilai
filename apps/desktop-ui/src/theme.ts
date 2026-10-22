export type AppTheme = "light" | "dark";

export const themeStorageKey = "cilai.workbench.theme";

export function readTheme(storage: Pick<Storage, "getItem"> = window.localStorage): AppTheme {
  const stored = storage.getItem(themeStorageKey);
  if (stored === "light" || stored === "dark") return stored;
  return typeof window.matchMedia === "function" && window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function applyTheme(theme: AppTheme, storage: Pick<Storage, "setItem"> = window.localStorage) {
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
  storage.setItem(themeStorageKey, theme);
}
