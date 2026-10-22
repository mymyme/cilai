import { expect, test } from "vitest";
import { automationNextPreview } from "./automation-preview";
const daily = { scheduleKind: "daily" as const, timeOfDayMinutes: 9 * 60, weekdays: [1], intervalMinutes: 60 };
test("每日预览越过已过时间并保持当地时刻", () => {
  expect(automationNextPreview(daily, new Date(2026, 9, 8, 10))).toEqual(new Date(2026, 9, 9, 9));
  expect(automationNextPreview(daily, new Date(2026, 9, 8, 8))).toEqual(new Date(2026, 9, 8, 9));
});
test("每周预览只采用明确选定日期，空日期不能排期", () => {
  expect(automationNextPreview({ ...daily, scheduleKind: "weekly", weekdays: [5, 1] }, new Date(2026, 9, 8, 10))).toEqual(new Date(2026, 9, 9, 9));
  expect(automationNextPreview({ ...daily, scheduleKind: "weekly", weekdays: [] })).toBeNull();
});
test("间隔按经过时长，非法时间不生成预计运行", () => {
  const from = new Date(2026, 9, 8, 10);
  expect(automationNextPreview({ ...daily, scheduleKind: "interval" }, from)?.getTime()).toBe(from.getTime() + 3600_000);
  expect(automationNextPreview({ ...daily, timeOfDayMinutes: NaN })).toBeNull();
});
