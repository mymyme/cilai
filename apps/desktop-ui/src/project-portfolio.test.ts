import { afterEach, describe, expect, test, vi } from "vitest";
import { relativeDay } from "./datetime";

const DAY = 86_400_000;
const now = new Date("2026-08-24T12:00:00+08:00").getTime();

afterEach(() => vi.useRealTimers());
const at = (offsetDays: number) => {
  vi.useFakeTimers();
  vi.setSystemTime(now);
  return relativeDay(now - offsetDays * DAY);
};

describe("项目列表的更新时间", () => {
  test("按天数给出相对表述", () => {
    expect(at(0)).toBe("今天");
    expect(at(1)).toBe("昨天");
    expect(at(3)).toBe("3 天前");
    expect(at(10)).toBe("1 周前");
    expect(at(45)).toBe("1 个月前");
    expect(at(400)).toBe("1 年前");
  });

  test("时间戳按毫秒解读；若误当成秒再乘 1000，会算到几万年后", () => {
    vi.useFakeTimers();
    vi.setSystemTime(now);
    // 服务端的 updated_at 是毫秒（as_millis），列表曾经又乘了一次 1000。
    expect(relativeDay(now - DAY)).toBe("昨天");
    expect(new Date(now).getFullYear()).toBe(2026);
    expect(new Date(now * 1000).getFullYear()).toBeGreaterThan(50_000);
  });

  test("未来时间不产生负数天数", () => {
    vi.useFakeTimers();
    vi.setSystemTime(now);
    expect(relativeDay(now + DAY)).toBe("今天");
  });
});
