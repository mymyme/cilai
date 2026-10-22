import { afterEach, describe, expect, test, vi } from "vitest";
import { auditMoment, clockMoment, deadlineDay, durationBetween, durationSpan, eventMoment, formatCount, listMoment, relativeDay, relativeSince, timelineMoment } from "./datetime";

const MINUTE = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;

/** 用本地时间字段构造，断言才不会随运行机器所在时区漂移。 */
const moment = new Date(2026, 7, 24, 12, 34, 56).getTime();

afterEach(() => vi.useRealTimers());

/** 相对表述都以“现在”为基准，不把时钟钉死就没有确定结果。 */
function freezeAt(now: number) {
  vi.useFakeTimers();
  vi.setSystemTime(now);
}

describe("相对时间", () => {
  test("不足一分钟统一说“刚刚”，秒级数字会在列表里反复跳动", () => {
    freezeAt(moment);
    expect(relativeSince(moment)).toBe("刚刚");
    expect(relativeSince(moment - 59_000)).toBe("刚刚");
  });

  test("跨过整分钟才进位到分钟", () => {
    freezeAt(moment);
    expect(relativeSince(moment - MINUTE)).toBe("1 分钟前");
    expect(relativeSince(moment - 59 * MINUTE)).toBe("59 分钟前");
  });

  test("跨过整小时才进位到小时，不足一小时的余数不显示", () => {
    freezeAt(moment);
    expect(relativeSince(moment - HOUR)).toBe("1 小时前");
    expect(relativeSince(moment - 90 * MINUTE)).toBe("1 小时前");
  });

  test("跨天后继续按小时累加：这里看的是还没结束的运行，超过一天本身就是异常信号", () => {
    freezeAt(moment);
    expect(relativeSince(moment - DAY)).toBe("24 小时前");
    expect(relativeSince(moment - 3 * DAY)).toBe("72 小时前");
  });

  test("按天数分档，各档位在边界上进位", () => {
    freezeAt(moment);
    expect(relativeDay(moment)).toBe("今天");
    expect(relativeDay(moment - DAY)).toBe("昨天");
    expect(relativeDay(moment - 6 * DAY)).toBe("6 天前");
    expect(relativeDay(moment - 7 * DAY)).toBe("1 周前");
    expect(relativeDay(moment - 30 * DAY)).toBe("1 个月前");
    expect(relativeDay(moment - 365 * DAY)).toBe("1 年前");
  });

  test("本机时钟落后于服务端时不出现负数", () => {
    freezeAt(moment);
    expect(relativeSince(moment + HOUR)).toBe("刚刚");
    expect(relativeDay(moment + DAY)).toBe("今天");
  });
});

describe("各场景的显示粒度", () => {
  test("审计时刻精确到秒，才能和运行日志、事件序号对上", () => {
    expect(auditMoment(moment)).toBe("2026年8月24日 12:34:56");
  });

  test("列表脚注与截止日不带年份", () => {
    expect(listMoment(moment)).toBe("8/24 12:34");
    expect(deadlineDay(moment)).toBe("8/24");
  });

  test("钟点只到分钟，逐条运行事件才补上秒", () => {
    expect(clockMoment(moment)).toBe("12:34");
    expect(eventMoment(moment)).toBe("12:34:56");
  });
});

describe("变更时间线", () => {
  test("当天的记录只给钟点", () => {
    freezeAt(moment);
    expect(timelineMoment(moment)).toBe("12:34");
    expect(timelineMoment(new Date(2026, 7, 24, 0, 5).getTime())).toBe("00:05");
  });

  test("跨天的记录补上月日", () => {
    freezeAt(moment);
    expect(timelineMoment(moment - DAY)).toBe("8/23 12:34");
  });
});

describe("时间戳单位", () => {
  test("入参按毫秒解读；若误当成秒再乘 1000，会算到几万年后", () => {
    // 后端给的是 unix_epoch_millis，历史上两次由调用方多乘了一次 1000。
    expect(auditMoment(moment)).toContain("2026年");
    expect(new Date(moment * 1000).getFullYear()).toBeGreaterThan(50_000);
  });
});

describe("截止日", () => {
  const inYear = new Date(2026, 7, 24, 12).getTime();

  test("同一年只给月日，不用年份占地方", () => {
    expect(deadlineDay(new Date(2026, 2, 3).getTime(), inYear)).not.toContain("2026");
  });

  test("跨年补上年份：逾期一年的任务不能和今年的长得一样", () => {
    const lastYear = deadlineDay(new Date(2025, 7, 24).getTime(), inYear);
    const thisYear = deadlineDay(new Date(2026, 7, 24).getTime(), inYear);
    expect(lastYear).toContain("2025");
    expect(lastYear).not.toBe(thisYear);
  });
});

describe("时长", () => {
  test("不足一分钟只给秒", () => {
    expect(durationSpan(4_200)).toBe("4 秒");
  });

  test("超过一分钟给分和秒", () => {
    expect(durationSpan(75_000)).toBe("1 分 15 秒");
  });

  test("负数当作零，不显示负时长", () => {
    expect(durationSpan(-5_000)).toBe("0 秒");
  });

  test("两个时刻之间与直接给时长一致", () => {
    expect(durationBetween(1_000, 76_000)).toBe(durationSpan(75_000));
  });
});

describe("计数", () => {
  test("四位以上加分位符，便于一眼看出量级", () => {
    expect(formatCount(1234567)).toContain(",");
  });
});
