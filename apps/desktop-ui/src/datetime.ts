/**
 * 界面上所有“某件事发生在什么时候”的呈现都收在这里。
 *
 * 同一个时刻散在八个页面时会长出八种写法，读者每换一个页面就得重新适应一次。
 * 函数按使用场景命名而不是按输出格式命名：调用方该问的是“这里要给读者多少信息”，
 * 而不是“这里要几位数字”。
 *
 * 入参一律是毫秒时间戳，即后端 unix_epoch_millis 的原值。这里不做任何单位换算——
 * 历史上两次显示到公元五万年，都是调用方先自作主张乘了 1000。
 */

/**
 * Intl.DateTimeFormat 的构造开销远大于一次 format，而下面几个函数会被逐行渲染反复调用，
 * 所以实例提到模块顶层，只在首次导入时构造一次。
 */
const AUDIT_FORMAT = new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "medium" });
const LIST_FORMAT = new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
const DAY_FORMAT = new Intl.DateTimeFormat("zh-CN", { month: "numeric", day: "numeric" });
const CLOCK_FORMAT = new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit" });
const CLOCK_SECOND_FORMAT = new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
const DAY_WITH_YEAR_FORMAT = new Intl.DateTimeFormat("zh-CN", { year: "numeric", month: "numeric", day: "numeric" });
const COUNT_FORMAT = new Intl.NumberFormat("zh-CN");

const MINUTE = 60_000;
const DAY = 86_400_000;

/** 项目列表：日期只用来判断“新不新”，相对表述比完整日期短一半，完整值留给悬停。 */
export function relativeDay(millis: number): string {
  const days = Math.floor((Date.now() - millis) / DAY);
  if (days <= 0) return "今天";
  if (days === 1) return "昨天";
  if (days < 7) return `${days} 天前`;
  if (days < 30) return `${Math.floor(days / 7)} 周前`;
  if (days < 365) return `${Math.floor(days / 30)} 个月前`;
  return `${Math.floor(days / 365)} 年前`;
}

/** 运行列表：读者盯的是还没结束的事，“多久以前”比“几点几分”更好判断，分钟级足够。 */
export function relativeSince(millis: number): string {
  const minutes = Math.max(0, Math.floor((Date.now() - millis) / MINUTE));
  if (minutes < 1) return "刚刚";
  if (minutes < 60) return `${minutes} 分钟前`;
  return `${Math.floor(minutes / 60)} 小时前`;
}

/** 详情与审计：要能和运行日志、事件序号对上，必须精确到秒；列表里悬停才展开的完整值也用它。 */
export function auditMoment(millis: number): string {
  return AUDIT_FORMAT.format(millis);
}

/** 变更时间线：当天的记录密集出现，重复年月日只会挤掉标题，跨天才值得补上月日。 */
export function timelineMoment(millis: number): string {
  return isSameDay(millis, Date.now()) ? clockMoment(millis) : listMoment(millis);
}

/** 工作卡片脚注：年份对“最近更新”没有信息量，月日时分已经够定位到某个半天。 */
export function listMoment(millis: number): string {
  return LIST_FORMAT.format(millis);
}

/**
 * 截止日：任务按天交付，显示到时分反而像承诺了某个具体时刻，所以只到天。
 *
 * 但跨年时必须补上年份。逾期任务会在列表里长期挂着，只写月日的话 2025-08-24 和
 * 2026-08-24 长得一模一样，一个逾期一年的任务看起来和今年的一样普通。
 */
export function deadlineDay(millis: number, now: number = Date.now()): string {
  const sameYear = new Date(millis).getFullYear() === new Date(now).getFullYear();
  return (sameYear ? DAY_FORMAT : DAY_WITH_YEAR_FORMAT).format(millis);
}

/** 对话气泡与权限有效期：同一屏里都是当天的事，只需要看钟点。 */
export function clockMoment(millis: number): string {
  return CLOCK_FORMAT.format(millis);
}

/** 逐条运行事件：相邻事件常落在同一分钟内，秒是判断先后的唯一依据。 */
export function eventMoment(millis: number): string {
  return CLOCK_SECOND_FORMAT.format(millis);
}

function isSameDay(left: number, right: number): boolean {
  return new Date(left).toDateString() === new Date(right).toDateString();
}

/**
 * 一段时长。运行列表、运行详情和对话里的进行中提示原本各存一份实现，
 * ai-workbench 自己就存了两份，逻辑逐字相同。
 */
export function durationSpan(millis: number): string {
  const seconds = Math.max(0, Math.floor(millis / 1_000));
  return seconds < 60 ? `${seconds} 秒` : `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`;
}

/** 两个时刻之间的时长。 */
export function durationBetween(start: number, end: number): string {
  return durationSpan(end - start);
}

/** 计数（token 数、条数）。分位符让四位以上的数一眼看出量级。 */
export function formatCount(value: number): string {
  return COUNT_FORMAT.format(value);
}
