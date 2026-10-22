import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const listeners = vi.hoisted(() => new Map<string, (payload: unknown) => void>());
vi.mock("./host", () => ({
  invoke,
  listen: (event: string, handler: (payload: unknown) => void) => {
    listeners.set(event, handler);
    return Promise.resolve(() => listeners.delete(event));
  },
}));

import { AiBrowserPanel, AiBrowserToggle, aiBrowserToggleIntent, type AiBrowserState } from "./ai-browser";

const CONV = "conv-1";
const tab = (id: string, url: string, title = "", conversationId: string | null = CONV) => ({
  id, url, title, conversationId, loading: false, canGoBack: false, canGoForward: false,
});

const state = (overrides: Partial<AiBrowserState> = {}): AiBrowserState => ({
  running: true,
  driver: "ai",
  activeId: "t1",
  activeByConversation: { [CONV]: "t1" },
  tabs: [tab("t1", "https://supplier.example.com/quote", "询价单")],
  ...overrides,
});

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(null);
  listeners.clear();
});

afterEach(cleanup);

const panel = (overrides: Partial<AiBrowserState> = {}, open = true, conversationId: string | null = CONV) =>
  render(<AiBrowserPanel conversationId={conversationId} onClose={() => {}} onWidthChange={() => {}} open={open} state={state(overrides)} width={420}/>);

describe("工作台内的浏览器模块", () => {
  it("标签栏列出全部标签，当前标签有标记", () => {
    const { container } = panel({
      tabs: [tab("t1", "https://a.example.com/", "甲"), tab("t2", "https://b.example.com/", "乙")],
    });
    expect(screen.getByText("甲")).toBeTruthy();
    expect(screen.getByText("乙")).toBeTruthy();
    expect(container.querySelectorAll('.ai-browser__tab[data-active="true"]')).toHaveLength(1);
  });

  it("地址栏提交走员工导航，不经站点策略", async () => {
    panel();
    const input = screen.getByLabelText("地址");
    fireEvent.change(input, { target: { value: "gov.cn" } });
    fireEvent.submit(input.closest("form") as HTMLFormElement);
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("browser_employee_navigate", { conversationId: CONV, url: "https://gov.cn" });
    });
  });

  it("已经带协议的地址原样提交，不再补一层", async () => {
    panel();
    const input = screen.getByLabelText("地址");
    fireEvent.change(input, { target: { value: "http://127.0.0.1:8080/x" } });
    fireEvent.submit(input.closest("form") as HTMLFormElement);
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("browser_employee_navigate", { conversationId: CONV, url: "http://127.0.0.1:8080/x" });
    });
  });

  it("前进后退按当前标签的可用性禁用，不放点不动的按钮", () => {
    panel({ tabs: [{ ...tab("t1", "https://a.example.com/"), canGoBack: true }] });
    expect(screen.getByLabelText("后退").hasAttribute("disabled")).toBe(false);
    expect(screen.getByLabelText("前进").hasAttribute("disabled")).toBe(true);
  });

  it("检查按钮开合开发者工具", async () => {
    panel();
    fireEvent.click(screen.getByLabelText("检查"));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("browser_devtools", { conversationId: CONV }));
  });

  it("新建与关闭标签", async () => {
    panel();
    fireEvent.click(screen.getByLabelText("新建标签"));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("browser_tab_open", { conversationId: CONV }));
    fireEvent.click(screen.getByLabelText("关闭标签"));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("browser_tab_close", { conversationId: CONV, tabId: "t1" }));
  });

  it("驾驶权只有一个：AI 操作中可接管，员工操作中可交还", async () => {
    const { unmount } = panel();
    expect(screen.getByText("AI 操作中")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "接管" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("browser_takeover_begin", { conversationId: CONV }));
    unmount();
    panel({ driver: "employee" });
    expect(screen.getByText("你在操作，AI 已暂停")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "交还给 AI" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("browser_takeover_end", { conversationId: CONV }));
  });

  /*
   * 收起只能隐藏，不能卸载。卸载会连同标签的内容元素一起移除——AI 正在浏览的页面就此被
   * 杀掉，员工收一下面板等于打断了它。
   */
  it("收起时面板仍在文档里，只是不呈现", () => {
    const { container } = panel({}, false);
    const el = container.querySelector(".ai-browser");
    expect(el).not.toBeNull();
    expect(el?.getAttribute("data-open")).toBe("false");
  });

  it("宿主请求开标签时建内容元素，标识写在地址里传过去", async () => {
    const { container } = panel({ tabs: [], activeId: null, running: false });
    await waitFor(() => expect(listeners.has("browser-open-tab")).toBe(true));
    listeners.get("browser-open-tab")?.({ tabId: "t-新", conversationId: CONV });
    await waitFor(() => {
      const view = container.querySelector(".ai-browser__view");
      expect(view?.getAttribute("src")).toContain(`cilaiTab=${encodeURIComponent("t-新")}`);
      expect(view?.getAttribute("src")).toContain(`cilaiConversation=${CONV}`);
      expect(view?.getAttribute("partition")).toBe("persist:ai-browser-legacy");
    });
  });

  /*
   * 浏览器归属发起它的那次对话。切到别的对话就不该看到它——那是另一件事的上下文；
   * 但标签不能因此被卸载，切回来还要在。
   */
  it("只呈现当前对话开出来的标签", () => {
    const { container } = panel({
      tabs: [tab("t1", "https://a.example.com/", "本对话"), tab("t2", "https://b.example.com/", "别处", "conv-2")],
      activeByConversation: { [CONV]: "t1", "conv-2": "t2" },
    });
    expect(screen.getByText("本对话")).toBeTruthy();
    expect(screen.queryByText("别处")).toBeNull();
    expect(container.querySelectorAll(".ai-browser__tab")).toHaveLength(1);
  });

  it("切到没有标签的对话时不列标签，面板仍在文档里", () => {
    // 呈现与否由调用方按同一份判定给出，面板只照做；元素不能因此卸载
    const { container } = panel({}, false, "conv-空");
    const el = container.querySelector(".ai-browser");
    expect(el).not.toBeNull();
    expect(el?.getAttribute("data-open")).toBe("false");
    expect(container.querySelectorAll(".ai-browser__tab")).toHaveLength(0);
  });

  it("不在对话页时不列标签", () => {
    const { container } = panel({}, false, null);
    expect(container.querySelectorAll(".ai-browser__tab")).toHaveLength(0);
  });

  it("别的对话的标签留在文档里，切回去时还在", async () => {
    const { container } = panel({ tabs: [], activeByConversation: {} }, true, "conv-2");
    await waitFor(() => expect(listeners.has("browser-open-tab")).toBe(true));
    listeners.get("browser-open-tab")?.({ tabId: "t9", conversationId: CONV });
    await waitFor(() => {
      expect(container.querySelectorAll(".ai-browser__view")).toHaveLength(1);
    });
    // 不属于当前对话，因此不呈现，但元素还在——卸载等于把那个对话的页面杀掉
    expect(container.querySelector('.ai-browser__view[data-active]')).toBeNull();
  });

  it("工具栏入口在浏览时给出提示，员工不必展开才知道", () => {
    const { container } = render(
      <AiBrowserToggle onToggle={() => {}} open={false} state={state()}/>);
    expect(container.querySelector('.ai-browser-toggle[data-running="true"]')).not.toBeNull();
    expect(container.querySelector(".ai-browser-toggle__dot")).not.toBeNull();
  });

  /*
   * "正在浏览"曾经也是把整颗图标染成强调色，于是 AI 一开始浏览，收起态和展开态就同色，
   * 员工点完看不出有没有生效。两个状态必须落在不同的属性上，样式才分得开。
   */
  it("展开与收起落在不同的属性上，正在浏览不会把两者抹平", () => {
    const collapsed = render(<AiBrowserToggle onToggle={() => {}} open={false} state={state()}/>);
    expect(collapsed.container.querySelector('.ai-browser-toggle[aria-expanded="false"]')).not.toBeNull();
    const expanded = render(<AiBrowserToggle onToggle={() => {}} open={true} state={state()}/>);
    expect(expanded.container.querySelector('.ai-browser-toggle[aria-expanded="true"]')).not.toBeNull();
    // 两者都在浏览中，区别只能来自展开属性本身
    expect(collapsed.container.querySelector(".ai-browser-toggle__dot")).not.toBeNull();
    expect(expanded.container.querySelector(".ai-browser-toggle__dot")).not.toBeNull();
  });

  it("没有在浏览时不显示提示点", () => {
    const { container } = render(
      <AiBrowserToggle onToggle={() => {}} open={false} state={state({ running: false })}/>);
    expect(container.querySelector(".ai-browser-toggle__dot")).toBeNull();
  });

  /*
   * 入口曾经点下去没有任何反应：它按收起标记翻转，而呈现与否还要求本对话已有标签，
   * 于是"要展开得先有标签、要有标签得先展开"成了死结。这组用例把四种状态都钉住。
   */
  describe("入口点击的意图", () => {
    it("面板正开着就是收起", () => {
      expect(aiBrowserToggleIntent({ open: true, conversationId: CONV, hasTabs: true, fits: true }))
        .toEqual({ action: "collapse" });
    });

    it("没有对话时说清楚要先选对话，而不是默默什么都不做", () => {
      expect(aiBrowserToggleIntent({ open: false, conversationId: null, hasTabs: false, fits: true }))
        .toEqual({ action: "needConversation" });
    });

    it("本对话还没有标签时，展开的同时开一个", () => {
      expect(aiBrowserToggleIntent({ open: false, conversationId: CONV, hasTabs: false, fits: true }))
        .toEqual({ action: "expand", openTab: true });
    });

    it("本对话已有标签时只管展开，不再多开一个", () => {
      expect(aiBrowserToggleIntent({ open: false, conversationId: CONV, hasTabs: true, fits: true }))
        .toEqual({ action: "expand", openTab: false });
    });

    it("窗口窄到放不下时说清楚，不再默默展开一块看不见的面板", () => {
      expect(aiBrowserToggleIntent({ open: false, conversationId: CONV, hasTabs: false, fits: false }))
        .toEqual({ action: "tooNarrow" });
    });

    it("关掉最后一个标签后再点，仍然是展开而不是收起", () => {
      // 此时收起标记还是假，但面板已经不呈现了——按呈现判断才不会空点一次
      expect(aiBrowserToggleIntent({ open: false, conversationId: CONV, hasTabs: false, fits: true }))
        .toEqual({ action: "expand", openTab: true });
    });
  });
});

it("整页截图按宿主尺寸调整内容承载并确认，结束后恢复默认尺寸", async () => {
  const { container, rerender } = panel({ tabs: [], activeId: null });
  await waitFor(() => expect(listeners.has("browser-open-tab")).toBe(true));
  listeners.get("browser-open-tab")?.({ tabId: "capture-tab", conversationId: CONV });
  const view = container.querySelector("webview") as HTMLElement;
  const measured = vi.spyOn(view, "getBoundingClientRect").mockImplementation(() => ({
    x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0,
    width: Number.parseInt(view.style.width) || 420,
    height: Number.parseInt(view.style.height) || 700,
    toJSON: () => ({}),
  }));
  const show = (captureLayout: { id: string; size: { width: number; height: number } | null }) =>
    rerender(<AiBrowserPanel conversationId={CONV} onClose={() => {}} onWidthChange={() => {}} open
      state={state({ activeId: "capture-tab", activeByConversation: { [CONV]: "capture-tab" },
        tabs: [{ ...tab("capture-tab", "https://example.com"), captureLayout }] })} width={420}/>);
  show({ id: "capture-start", size: { width: 1440, height: 3600 } });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("browser_capture_layout_ready", {
    tabId: "capture-tab", requestId: "capture-start", width: 1440, height: 3600,
  }));
  expect(view.style.width).toBe("1440px");
  show({ id: "capture-end", size: null });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("browser_capture_layout_ready", {
    tabId: "capture-tab", requestId: "capture-end", width: 420, height: 700,
  }));
  expect(view.style.width).toBe("");
  expect(view.style.height).toBe("");
  measured.mockRestore();
});
