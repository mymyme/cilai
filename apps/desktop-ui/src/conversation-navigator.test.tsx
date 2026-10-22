import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ConversationNavigator, fisheyeScale, previewParts } from "./conversation-navigator";

afterEach(cleanup);

const turn = (id: string, content: string, reply?: string) => ({ messageId: id, content, reply });
/** 导航条至少三次发问才出现，只关心某一条的用例拿这两条补足。 */
const fillers = [turn("f1", "补足一"), turn("f2", "补足二")];

describe("对话导航条", () => {
  it("每次发问一个刻度，AI 的回复不单独占刻度", () => {
    const { container } = render(<ConversationNavigator
      activeMessageId={null}
      messages={[turn("m1", "打开百度", "已打开"), turn("m2", "再搜一下天气"), turn("m3", "换成明天")]}
      onJump={() => {}}/>);
    expect(container.querySelectorAll(".conversation-navigator__tick")).toHaveLength(3);
  });

  // 一两轮的对话一屏就看得全，左缘多出几道刻度只是噪音
  it("少于三次发问时不渲染，恰好三次时出现", () => {
    for (const count of [0, 1, 2]) {
      const messages = Array.from({ length: count }, (_, i) => turn(`m${i}`, `第${i}问`));
      const { container, unmount } = render(<ConversationNavigator activeMessageId={null} messages={messages} onJump={() => {}}/>);
      expect(container.querySelector(".conversation-navigator")).toBeNull();
      unmount();
    }
    const messages = Array.from({ length: 3 }, (_, i) => turn(`m${i}`, `第${i}问`));
    const { container } = render(<ConversationNavigator activeMessageId={null} messages={messages} onJump={() => {}}/>);
    expect(container.querySelectorAll(".conversation-navigator__tick")).toHaveLength(3);
  });

  /*
   * 鱼眼放大：以指针所在那条为中心，长度向两侧依次递减。
   * 只放大被指的那一条会显得突兀——指针在相邻两条之间移动时长度会硬跳。
   */
  describe("鱼眼放大", () => {
    it("中心最长，向两侧单调递减", () => {
      const scales = [0, 1, 2, 3].map(fisheyeScale);
      for (let i = 1; i < scales.length; i += 1) expect(scales[i]).toBeLessThan(scales[i - 1]);
    });

    it("左右对称：指针在上方还是下方，放大幅度一样", () => {
      expect(fisheyeScale(-2)).toBeCloseTo(fisheyeScale(2), 5);
      expect(fisheyeScale(-1)).toBeCloseTo(fisheyeScale(1), 5);
    });

    it("到作用半径就回到原样，不必给整列都算", () => {
      expect(fisheyeScale(4)).toBe(1);
      expect(fisheyeScale(5)).toBe(1);
      expect(fisheyeScale(40)).toBe(1);
    });

    // 倍数由参考图反推，实测 3.28 / 2.25 / 1.75 / 1.35，这里守住量级不被随手改动
    it("放大倍数与参考图一致", () => {
      expect(fisheyeScale(0)).toBeCloseTo(3.3, 1);
      expect(fisheyeScale(1)).toBeCloseTo(2.2, 1);
      expect(fisheyeScale(2)).toBeCloseTo(1.7, 1);
    });
  });

  it("指到某一条时，它和邻近几条一起变长，其余不动", () => {
    const messages = Array.from({ length: 8 }, (_, i) => turn(`m${i}`, `第${i}问`));
    const { container } = render(<ConversationNavigator
      activeMessageId={null} messages={messages} onJump={() => {}}/>);
    const ticks = [...container.querySelectorAll(".conversation-navigator__tick")];
    const scaleOf = (index: number) => {
      const value = (ticks[index].querySelector("span") as HTMLElement).style.transform;
      return Number.parseFloat(value.replace(/[^\d.]/g, "")) || 1;
    };
    fireEvent.pointerEnter(ticks[4]);
    expect(scaleOf(4)).toBeGreaterThan(scaleOf(5));
    expect(scaleOf(5)).toBeGreaterThan(scaleOf(6));
    expect(scaleOf(3)).toBeCloseTo(scaleOf(5), 5);
    expect(scaleOf(0)).toBe(1);
  });

  it("指针离开整条轨道后所有刻度回到原样", () => {
    const messages = Array.from({ length: 4 }, (_, i) => turn(`m${i}`, `第${i}问`));
    const { container } = render(<ConversationNavigator
      activeMessageId={null} messages={messages} onJump={() => {}}/>);
    const nav = container.querySelector(".conversation-navigator") as HTMLElement;
    const ticks = [...container.querySelectorAll(".conversation-navigator__tick")];
    fireEvent.pointerEnter(ticks[1]);
    expect((ticks[1].querySelector("span") as HTMLElement).style.transform).not.toBe("scaleX(1)");
    fireEvent.pointerLeave(nav);
    for (const tick of ticks) {
      expect((tick.querySelector("span") as HTMLElement).style.transform).toBe("scaleX(1)");
    }
  });

  /*
   * 预览要给出这一轮的两端：只看提问认不出是哪一轮——同一个问题会反复问，
   * 真正能区分的是它得到了什么答复。
   */
  it("预览上半是员工发的，下半是 AI 回的", () => {
    expect(previewParts(turn("m1", "请全部完成", "已全部完成本地开发范围。")))
      .toEqual({ title: "请全部完成", body: "已全部完成本地开发范围。" });
  });

  it("还没有回复时只给出提问，不留占位", () => {
    expect(previewParts(turn("m1", "请全部完成"))).toEqual({ title: "请全部完成", body: "" });
  });

  it("多行内容压成一段，预览里要的是能扫一眼的摘要", () => {
    expect(previewParts(turn("m1", "第一行\n\n  第二行  ", "答\n复")))
      .toEqual({ title: "第一行 第二行", body: "答 复" });
  });

  it("悬停后弹出预览，移开即收", async () => {
    render(<ConversationNavigator
      activeMessageId={null}
      messages={[turn("m1", "请全部完成", "已全部完成本地开发范围。"), ...fillers]}
      onJump={() => {}}/>);
    const tick = screen.getByRole("button", { name: /请全部完成/ });
    fireEvent.pointerEnter(tick);
    await waitFor(() => expect(screen.getByText("请全部完成")).toBeTruthy());
    expect(screen.getByText("已全部完成本地开发范围。")).toBeTruthy();
    fireEvent.pointerLeave(tick);
    await waitFor(() => expect(screen.queryByText("已全部完成本地开发范围。")).toBeNull());
  });

  it("点击交出消息标识，由对话页负责滚过去", () => {
    const jumped: string[] = [];
    render(<ConversationNavigator
      activeMessageId={null}
      messages={[turn("m1", "第一问"), turn("m2", "第二问"), ...fillers]}
      onJump={(id) => jumped.push(id)}/>);
    fireEvent.click(screen.getByRole("button", { name: /第二问/ }));
    expect(jumped).toEqual(["m2"]);
  });

  it("当前停留的那一条被标出来，且整条导航只有一处", () => {
    const { container } = render(<ConversationNavigator
      activeMessageId="m2"
      messages={[turn("m1", "第一问"), turn("m2", "第二问"), turn("m3", "第三问")]}
      onJump={() => {}}/>);
    const active = container.querySelectorAll('.conversation-navigator__tick[data-active="true"]');
    expect(active).toHaveLength(1);
    expect(active[0].getAttribute("aria-label")).toContain("第二问");
  });

  it("键盘聚焦同样弹预览，不是只有鼠标能用", async () => {
    render(<ConversationNavigator
      activeMessageId={null}
      messages={[turn("m1", "请全部完成", "已完成"), ...fillers]}
      onJump={() => {}}/>);
    fireEvent.focus(screen.getByRole("button", { name: /请全部完成/ }));
    await waitFor(() => expect(screen.getByText("已完成")).toBeTruthy());
  });

  /*
   * 预览按触发时的位置定位并挂在文档根部，页面一滚它就飘在错误的地方，
   * 还会挡住正文——收起比跟着重算便宜。
   */
  it("页面滚动时收起预览", async () => {
    render(<ConversationNavigator
      activeMessageId={null}
      messages={[turn("m1", "请全部完成", "已完成"), ...fillers]}
      onJump={() => {}}/>);
    fireEvent.pointerEnter(screen.getByRole("button", { name: /请全部完成/ }));
    await waitFor(() => expect(screen.getByText("已完成")).toBeTruthy());
    fireEvent.scroll(window);
    await waitFor(() => expect(screen.queryByText("已完成")).toBeNull());
  });

  it("预览等待期间滚动或改变窗口会取消待弹出内容", () => {
    vi.useFakeTimers();
    try {
      render(<ConversationNavigator activeMessageId={null}
        messages={[turn("m1", "滚动前提问", "不应显示"), ...fillers]} onJump={() => {}}/>);
      const tick = screen.getByRole("button", { name: /滚动前提问/ });
      for (const event of ["scroll", "resize"]) {
        fireEvent.pointerEnter(tick);
        fireEvent(window, new Event(event));
        act(() => vi.advanceTimersByTime(1000));
        expect(screen.queryByText("不应显示")).toBeNull();
      }
    } finally { vi.useRealTimers(); }
  });

  it("离开时清掉待弹的计时器，指针快速划过不会事后弹出来", async () => {
    vi.useFakeTimers();
    try {
      render(<ConversationNavigator
        activeMessageId={null}
        messages={[turn("m1", "请全部完成", "已完成"), ...fillers]}
        onJump={() => {}}/>);
      const tick = screen.getByRole("button", { name: /请全部完成/ });
      fireEvent.pointerEnter(tick);
      fireEvent.pointerLeave(tick);
      vi.advanceTimersByTime(2000);
      expect(screen.queryByText("已完成")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});
