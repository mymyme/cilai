import type { AutomationDraft } from "./automation-dialog";

export function automationNextPreview(draft: Pick<AutomationDraft, "scheduleKind" | "intervalMinutes" | "timeOfDayMinutes" | "weekdays">, from = new Date()): Date | null {
  if (draft.scheduleKind === "interval") return draft.intervalMinutes > 0 ? new Date(from.getTime() + draft.intervalMinutes * 60_000) : null;
  if (!Number.isInteger(draft.timeOfDayMinutes) || draft.timeOfDayMinutes < 0 || draft.timeOfDayMinutes >= 1440) return null;
  const hour = Math.floor(draft.timeOfDayMinutes / 60);
  const minute = draft.timeOfDayMinutes % 60;
  for (let offset = 0; offset < 56; offset++) {
    const date = new Date(from.getFullYear(), from.getMonth(), from.getDate() + offset, hour, minute);
    if (date.getHours() !== hour || date.getMinutes() !== minute || date <= from) continue;
    if (draft.scheduleKind === "weekly" && !draft.weekdays.includes(date.getDay() || 7)) continue;
    return date;
  }
  return null;
}
