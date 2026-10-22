/**
 * 提示分两档，因为它们要求用户做的事不一样：
 *
 * - info：动作成功了，或者正在进行。用户不需要做任何事，提示自己待在角落里就行。
 * - error：动作没做成。这一档必须挡住界面——横幅贴在页面顶部，用户正盯着刚点的那个
 *   按钮，很容易整条错过，然后以为操作生效了。
 */
export type MessageTone = "info" | "error";

export interface AppMessage {
  text: string;
  tone: MessageTone;
}

/** 组件向上报提示的统一签名；不写 tone 就是 info。 */
export type MessageSink = (message: string, tone?: MessageTone) => void;
