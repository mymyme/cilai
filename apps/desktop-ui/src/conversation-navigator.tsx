import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "./page-portal";
import "./conversation-navigator.css";

/**
 * 对话导航条。
 *
 * 一次对话动辄几十轮，要回看"我当时是怎么问的"只能一路往上滚。这条刻度把每次发问
 * 标在滚动区左缘：刻度的长短跟着那条消息的长短走，整条读起来就是这次对话的形状，
 * 长段落和一句追问一眼能分开。悬停出预览，点一下跳过去。
 *
 * 预览同时给出这一轮的两端：上面是员工自己发的内容，下面是 AI 那次回复的开头。
 * 只看提问往往认不出是哪一轮——同一个问题会反复问，真正能区分的是它得到了什么答复。
 *
 * 只标用户发出的内容。AI 的回复通常很长且连续，全都标上去刻度会挤成一片，反而找不到
 * 落点；而员工要定位的本来就是"我提过什么"。
 */

/** 悬停多久弹预览。短到不用等，又不会在指针划过整条时一路闪。 */
const previewDelay = 240;

/*
 * 鱼眼放大的两个系数，由参考图反推：以悬停那条为中心，长度向两侧按指数衰减。
 * 实测倍数 3.28 / 2.25 / 1.75 / 1.35 / 1.00，下面这组系数算出来是
 * 3.30 / 2.23 / 1.66 / 1.35 / 1.19，与之几乎重合。
 */
const fisheyeAmplitude = 2.3;
const fisheyeDecay = 1.6;
/** 到这个距离就回到原样。参考图里第 4 条起已与静止时等长。 */
const fisheyeReach = 4;
/**
 * 至少这么多次发问才出导航条。一两轮的对话一屏就看得全，滚一下就回到开头，
 * 左缘多出几道刻度只是噪音。
 */
export const navigatorMinMessages = 3;

export type NavigatorMessage = {
  messageId: string;
  /** 员工这一轮发出的内容。 */
  content: string;
  /** AI 对这一轮的回复。还没答复时为空。 */
  reply?: string;
};

/**
 * 距离悬停项 distance 条的刻度放大多少倍。
 *
 * 只放大被指的那一条会显得突兀：指针在相邻两条之间移动时，长度会在 1 倍和 3 倍之间
 * 硬跳。让邻近几条按距离依次放大，整列跟着指针连续起伏，落点也更容易对准。
 */
export function fisheyeScale(distance: number): number {
  if (!Number.isFinite(distance) || Math.abs(distance) >= fisheyeReach) return 1;
  return 1 + fisheyeAmplitude * Math.exp(-Math.abs(distance) / fisheyeDecay);
}

/** 把一行行文本压成一段连续文字：预览里要的是能扫一眼的摘要，不是原样排版。 */
function flatten(value: string | undefined): string {
  return (value ?? "").trim().split("\n").map((line) => line.trim()).filter(Boolean).join(" ");
}

/** 预览分两端：上面是员工发的，下面是 AI 回的。 */
export function previewParts(message: NavigatorMessage): { title: string; body: string } {
  return { title: flatten(message.content), body: flatten(message.reply) };
}

export function ConversationNavigator(props: {
  messages: NavigatorMessage[];
  /** 当前停在哪一条上，随滚动变化。 */
  activeMessageId: string | null;
  onJump: (messageId: string) => void;
}) {
  const [preview, setPreview] = useState<{ messageId: string; left: number; top: number } | null>(null);
  /*
   * 指针停在第几条上。鱼眼要按"与它的距离"给每条算倍数，而 CSS 选择器表达不了这种
   * 相对关系——:hover 只知道自己是不是被指的那一个，不知道离被指的那个有多远。
   */
  const [hovered, setHovered] = useState<number | null>(null);
  const timerRef = useRef<number | null>(null);
  const trackRef = useRef<HTMLElement | null>(null);
  const anchors = useRef(new Map<string, HTMLElement>());

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  useEffect(() => clearTimer, [clearTimer]);

  /*
   * 轨道一次只露出固定条数，长对话里当前那一条会滚出视野——整条索引就只剩个装饰，
   * 看不出自己停在哪一段。这里把它带回可见范围。
   *
   * 只改轨道自己的 scrollTop，不用 scrollIntoView：后者会把每一层可滚动祖先都一起滚，
   * 其中就有消息列表本身，而当前项正是随消息列表滚动算出来的——推着它滚会反过来改写
   * 当前项，两边互相触发。
   */
  useEffect(() => {
    const track = trackRef.current;
    const anchor = props.activeMessageId ? anchors.current.get(props.activeMessageId) : null;
    if (!track || !anchor) return;
    const top = anchor.offsetTop;
    const bottom = top + anchor.offsetHeight;
    if (top < track.scrollTop) track.scrollTop = top;
    else if (bottom > track.scrollTop + track.clientHeight) track.scrollTop = bottom - track.clientHeight;
  }, [props.activeMessageId]);

  const openPreview = useCallback((messageId: string) => {
    clearTimer();
    timerRef.current = window.setTimeout(() => {
      const anchor = anchors.current.get(messageId);
      if (!anchor) return;
      const rect = anchor.getBoundingClientRect();
      setPreview({ messageId, left: Math.round(rect.right + 10), top: Math.round(rect.top) });
    }, previewDelay);
  }, [clearTimer]);

  const closePreview = useCallback(() => {
    clearTimer();
    setPreview(null);
  }, [clearTimer]);

  /*
   * 滚动或改窗口时收起预览：它经传送门挂在文档根部，按触发时的位置定位，页面一动
   * 就会飘在错误的地方。收起比跟着重算便宜，也不会在滚动中挡住正文。
   */
  useEffect(() => {
    window.addEventListener("scroll", closePreview, true);
    window.addEventListener("resize", closePreview);
    return () => {
      window.removeEventListener("scroll", closePreview, true);
      window.removeEventListener("resize", closePreview);
    };
  }, [closePreview]);

  if (props.messages.length < navigatorMinMessages) return null;

  const previewed = preview ? props.messages.find((item) => item.messageId === preview.messageId) : null;
  const parts = previewed ? previewParts(previewed) : null;

  return <>
    <nav
      aria-label="跳转到我发出的内容"
      className="conversation-navigator"
      /* 离开整条轨道才收起放大：逐条清的话，指针在两条之间挪动会一路闪 */
      onPointerLeave={() => setHovered(null)}
      ref={trackRef}>
      {props.messages.map((message, index) => {
        const active = message.messageId === props.activeMessageId;
        return <button
          aria-current={active ? "true" : undefined}
          aria-label={`第 ${index + 1} 次发问：${previewParts(message).title.slice(0, 40)}`}
          className="conversation-navigator__tick"
          data-active={active}
          key={message.messageId}
          onBlur={() => { closePreview(); setHovered(null); }}
          onClick={() => { closePreview(); props.onJump(message.messageId); }}
          onFocus={() => { openPreview(message.messageId); setHovered(index); }}
          onPointerEnter={() => { openPreview(message.messageId); setHovered(index); }}
          onPointerLeave={closePreview}
          ref={(node) => {
            if (node) anchors.current.set(message.messageId, node);
            else anchors.current.delete(message.messageId);
          }}
          type="button"><span style={{
            transform: `scaleX(${hovered === null ? 1 : fisheyeScale(index - hovered)})`,
          }}/></button>;
      })}
    </nav>
    {preview && parts ? createPortal(
      <div className="conversation-navigator__preview" style={{ left: preview.left, top: preview.top }}>
        <strong>{parts.title}</strong>
        {parts.body ? <p>{parts.body}</p> : null}
      </div>,
      document.body,
    ) : null}
  </>;
}
