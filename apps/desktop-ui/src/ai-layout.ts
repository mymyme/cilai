/*
 * 对话页与浏览器面板分同一条横向空间，谁先退让要有唯一一处说了算，否则两边各算一份，
 * 迟早出现"对话被挤破"或"空着一列什么都不显示"。这里只放算宽度的纯逻辑，不碰界面。
 */

/** 聊天区的下限。与 styles.css 里 .ai-workbench 的 minmax(420px, 1fr) 是同一个数，有测试钉住。 */
export const minChatStageWidth = 420;
/*
 * 活动栏。对话页左边永远有这一条，宽度随主导航收起还是展开而变——两个数都与 styles.css
 * 里 .shell 那几条网格的第一列对应，有测试钉住。
 *
 * 此前这里只有一个 48，等于默认主导航永远是收起的。展开时实际占 196，让宽公式凭空多算
 * 148 像素并全数让给浏览器，对话页被压到下限以下、内容溢出后被 overflow 切掉——看上去
 * 像是浏览器盖在对话上面，实际是对话自己被切了，两种成因的修法完全不同。
 */
export const activityBarWidth = 48;
export const activityBarExpandedWidth = 196;

/** 浏览器面板的下限：再窄下去标签条和地址栏就挤成一团，不如不展开。 */
export const minAiBrowserWidth = 360;
export const maxAiBrowserWidth = 720;
export const defaultAiBrowserWidth = 420;

export const aiBrowserWidthStorageKey = "cilai.workbench.ai-browser-width";

export function activityBarWidthFor(navigationExpanded: boolean): number {
  return navigationExpanded ? activityBarExpandedWidth : activityBarWidth;
}

export function clampAiBrowserWidth(width: number): number {
  return Math.min(maxAiBrowserWidth, Math.max(minAiBrowserWidth, Math.round(width)));
}

export function readAiBrowserWidth(): number {
  try {
    const stored = window.localStorage.getItem(aiBrowserWidthStorageKey);
    if (stored === null || stored.trim() === "") return defaultAiBrowserWidth;
    const width = Number.parseInt(stored, 10);
    return Number.isFinite(width) ? clampAiBrowserWidth(width) : defaultAiBrowserWidth;
  } catch {
    return defaultAiBrowserWidth;
  }
}

/** 对话页自己的最小宽度：侧栏（收起时为零）加聊天区下限。 */
export function conversationMinWidth(input: { railWidth: number; railOpen: boolean }): number {
  return (input.railOpen ? Math.round(input.railWidth) : 0) + minChatStageWidth;
}

/**
 * 工作台放不放得下"对话 + 浏览器"，放得下的话浏览器该多宽。
 *
 * 退让的一律是浏览器：对话页是工作台本身，被挤破就等于这段时间没法干活；浏览器窄一点
 * 还能用，窄到下限以下就干脆不展开——留一条几十像素的空列，比收起来更难用。
 *
 * 活动栏宽度由调用方按主导航当前状态传进来。写成常量就等于假定它不会变，而它会。
 */
export function fitAiBrowser(input: {
  viewportWidth: number;
  activityBarWidth: number;
  conversationMinWidth: number;
  requestedWidth: number;
}): { fits: boolean; width: number } {
  const room = input.viewportWidth - input.activityBarWidth - input.conversationMinWidth;
  if (room < minAiBrowserWidth) return { fits: false, width: 0 };
  return { fits: true, width: Math.min(clampAiBrowserWidth(input.requestedWidth), room) };
}

/** 窗口至少要这么宽，浏览器才展得开。提示员工时用得上。 */
export function minViewportForAiBrowser(conversationMin: number, activityBar: number): number {
  return activityBar + conversationMin + minAiBrowserWidth;
}

/**
 * 展开浏览器而对话页一寸不让，窗口该有多宽。窗口比这个数窄就自动拉到这里。
 *
 * 用的是员工记住的那个宽度，不是浏览器下限：拉到勉强够用，员工每次展开都要重新拖一遍。
 */
export function desiredViewportForAiBrowser(input: {
  activityBarWidth: number;
  conversationMinWidth: number;
  requestedWidth: number;
}): number {
  return input.activityBarWidth + input.conversationMinWidth + clampAiBrowserWidth(input.requestedWidth);
}
