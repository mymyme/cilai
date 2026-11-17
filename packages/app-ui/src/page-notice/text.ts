import type { PageNotice } from "./stack.js";
/** 报错只呈现业务原因和下一步；后端附带的“错误代码：…”属于实现说明，不在浮层中显示。 */
export function noticeText(notice: PageNotice): string {
  return notice.kind === "error" ? notice.message.replace(/[\s，；;,]*错误代码[:：][^\n]*/g, "").trim() : notice.message;
}
