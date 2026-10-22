/**
 * 项目上下文与所在页面的落盘。它们决定重开应用时回到哪儿。
 *
 * 存的是 id 而不是整份快照：项目可能已经改名、阶段可能已经删掉，重开时按 id 重新
 * 取一遍，拿不到就当没存过。存快照会让界面显示一份已经不成立的旧事实。
 */

const projectDetailKey = "cilai.workbench.last-project-detail";
const pageKey = "cilai.workbench.last-page";

export interface StoredProjectDetail {
  projectId: string;
  phaseId: string | null;
}

export function readStoredProjectDetail(): StoredProjectDetail | null {
  const raw = window.localStorage.getItem(projectDetailKey);
  if (raw === null) return null;
  const parsed = parseStoredProjectDetail(raw);
  // 解析不出来的多半是旧版本留下的形状，留着只会每次启动都失败一次。
  if (parsed === null) window.localStorage.removeItem(projectDetailKey);
  return parsed;
}

export function writeStoredProjectDetail(value: StoredProjectDetail): void {
  window.localStorage.setItem(projectDetailKey, JSON.stringify(value));
}

export function clearStoredProjectDetail(): void {
  window.localStorage.removeItem(projectDetailKey);
}

export function readStoredPage(): string | null {
  return window.localStorage.getItem(pageKey);
}

export function writeStoredPage(page: string): void {
  window.localStorage.setItem(pageKey, page);
}

export function parseStoredProjectDetail(raw: string): { projectId: string; phaseId: string | null } | null {
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    const record = parsed as Record<string, unknown>;
    if (typeof record.projectId !== "string") return null;
    return { projectId: record.projectId, phaseId: typeof record.phaseId === "string" ? record.phaseId : null };
  } catch {
    return null;
  }
}
