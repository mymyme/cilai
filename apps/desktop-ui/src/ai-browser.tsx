import { useCallback, useEffect, useRef, useState } from "react";
import { invoke, listen } from "./host";
import { clampAiBrowserWidth, defaultAiBrowserWidth, maxAiBrowserWidth, minAiBrowserWidth } from "./ai-layout";
import "./ai-browser.css";

/**
 * 工作台内的浏览器模块。
 *
 * 承载方式与业务应用容器相同：内容嵌在本页面里由渲染引擎直接呈现，因此标签页、开发者工具
 * 与页面之外的原生弹层都按常规工作——这几样在离屏加画布的老做法下都做不了。
 *
 * 驾驶权只有一个。AI 操作期间它的导航受站点策略约束；员工一旦自己输入地址或点「接管」，
 * 驾驶权转到员工，AI 的写操作随即被拒。员工自己的导航不受站点策略约束——策略约束的是
 * AI 能去哪里，不是员工能去哪里。
 *
 * 员工打开的标签 AI 可以读取。这是这套东西的用处所在（"帮我看看这个页面"），代价是那些
 * 页面的内容会进入模型上下文，因此站点策略不再等于"AI 能看到什么"的边界。
 */

export type BrowserTab = {
  id: string;
  /** 开出这个标签的那次对话。切到别的对话就不该看到它——那是另一件事的上下文。 */
  conversationId: string | null;
  url: string;
  title: string;
  loading: boolean;
  captureLayout?: { id: string; size: { width: number; height: number } | null };
  canGoBack: boolean;
  canGoForward: boolean;
};

export type AiBrowserState = {
  profileId?: string;
  running: boolean;
  driver: "ai" | "employee";
  activeId: string | null;
  activeByConversation: Record<string, string>;
  tabs: BrowserTab[];
};

const IDLE: AiBrowserState = {
  profileId: "legacy", running: false, driver: "ai", activeId: null, activeByConversation: {}, tabs: [],
};

/** 订阅宿主的运行状态。面板与入口共用同一份，不各自记账。 */
/*
 * 窗口有多宽。布局要据此决定浏览器让到什么程度，而这个判断每次重排都要重算一遍，
 * 只能来自真实窗口宽度——写死断点在可调宽度的侧栏面前必然失准。
 */
export function useViewportWidth(): number {
  const [width, setWidth] = useState(() => (typeof window === "undefined" ? 0 : window.innerWidth));
  useEffect(() => {
    const measure = () => setWidth(window.innerWidth);
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, []);
  return width;
}

export function useAiBrowserState(): AiBrowserState {
  const [state, setState] = useState<AiBrowserState>(IDLE);
  useEffect(() => {
    let disposed = false;
    let dispose: (() => void) | undefined;
    void listen<AiBrowserState>("browser-state", setState).then((off) => {
      if (disposed) off();
      else dispose = off;
    });
    return () => {
      disposed = true;
      dispose?.();
    };
  }, []);
  return state;
}

function Icon(props: { path: string }) {
  return (
    <svg aria-hidden="true" fill="none" height="15" viewBox="0 0 16 16" width="15">
      <path d={props.path} stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.5"/>
    </svg>
  );
}

const BACK = "M10 3L5 8l5 5";
const FORWARD = "M6 3l5 5-5 5";
const RELOAD = "M13 8a5 5 0 1 1-1.6-3.7M13 2v3h-3";
const INSPECT = "M2.5 3.5h11v9h-11zM2.5 6h11M5 9l1.5 1.5L5 12";
const CLOSE = "M4 4l8 8M12 4l-8 8";
const PLUS = "M8 3.5v9M3.5 8h9";
/** 键盘每按一下挪多少。与对话侧栏的分割线取同一个步长。 */
const browserKeyboardStep = 16;

/**
 * 点入口该做什么。按"眼下看不看得见"判断，不按收起标记：员工关掉最后一个标签后面板
 * 就不呈现了，收起标记却还是假，此时再点入口只会把标记置真——看上去像点了没反应。
 * 展开时本对话还没有标签就得顺手开一个，否则"能否展开"与"有无标签"互为前提，谁也起不来。
 * 窗口窄到放不下时也要说出来：此时收起是布局替员工做的决定，不说等于按钮又成了哑的。
 */
export function aiBrowserToggleIntent(input: {
  open: boolean;
  conversationId: string | null;
  hasTabs: boolean;
  fits: boolean;
}): { action: "collapse" } | { action: "needConversation" } | { action: "tooNarrow" }
  | { action: "expand"; openTab: boolean } {
  if (input.open) return { action: "collapse" };
  if (input.conversationId === null) return { action: "needConversation" };
  if (!input.fits) return { action: "tooNarrow" };
  return { action: "expand", openTab: !input.hasTabs };
}

export function AiBrowserPanel(props: {
  state: AiBrowserState;
  /** 当前对话。不在对话页时为空，此时不呈现任何标签。 */
  conversationId: string | null;
  open: boolean;
  /** 员工拖出来的宽度。真正生效的宽度由布局裁剪，这里只负责把拖动结果报回去。 */
  width: number;
  onWidthChange: (width: number, persist: boolean) => void;
  onClose: () => void;
}) {
  const stageRef = useRef<HTMLDivElement>(null);
  const viewsRef = useRef(new Map<string, HTMLElement>());
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { driver } = props.state;
  const conversationId = props.conversationId;
  // 只呈现本次对话开出来的标签。其余标签仍在文档里跑着，切回去就还在
  const tabs = props.state.tabs.filter((tab) => tab.conversationId === conversationId);
  const activeId = props.state.activeByConversation[String(conversationId)] ?? null;
  const active = tabs.find((tab) => tab.id === activeId) ?? tabs[0] ?? null;

  const run = useCallback((command: string, args?: Record<string, unknown>) => {
    setError(null);
    void invoke(command, { conversationId, ...args }).catch(
      (reason: unknown) => setError(String(reason)));
  }, [conversationId]);

  /*
   * 宿主请开标签时才建元素。内容进程只能由界面创建，宿主据此接管——标签标识写在元素的
   * 初始地址里，附着事件本身不带参数，那是唯一能把标识带过去的位置。
   */
  useEffect(() => {
    let disposed = false;
    let dispose: (() => void) | undefined;
    void listen<{ tabId: string; conversationId: string | null }>(
      "browser-open-tab", ({ tabId, conversationId: conversation }) => {
      const stage = stageRef.current;
      if (!stage || viewsRef.current.has(tabId)) return;
      const view = document.createElement("webview");
      view.className = "ai-browser__view";
      view.setAttribute("partition", `persist:ai-browser-${props.state.profileId ?? "legacy"}`);
      view.setAttribute("allowpopups", "");
      const query = new URLSearchParams({ cilaiTab: tabId });
      if (conversation) query.set("cilaiConversation", conversation);
      view.setAttribute("src", `about:blank?${query.toString()}`);
      stage.append(view);
      viewsRef.current.set(tabId, view);
    }).then((off) => {
      if (disposed) off();
      else dispose = off;
    });
    return () => {
      disposed = true;
      dispose?.();
    };
  }, [props.state.profileId]);

  /*
   * 只移除宿主已撤销的标签。
   *
   * 判定必须用全量标签，不能用按对话过滤后的那份——过滤后别的对话的标签看起来"不在了"，
   * 元素随之被删，内容进程跟着销毁，那个对话的页面就此丢失。
   */
  useEffect(() => {
    const alive = new Set(props.state.tabs.map((tab) => tab.id));
    for (const [id, view] of viewsRef.current) {
      if (alive.has(id)) continue;
      view.remove();
      viewsRef.current.delete(id);
    }
    // 只有当前对话的当前标签呈现；别的对话的标签隐藏但不卸载，切回去时还在
    for (const [id, view] of viewsRef.current) {
      view.toggleAttribute("data-active", id === active?.id);
    }
  }, [props.state.tabs, active?.id]);

  useEffect(() => {
    for (const tab of props.state.tabs) {
      const request = tab.captureLayout, view = viewsRef.current.get(tab.id);
      if (!view || !request || view.dataset.captureLayout === request.id) continue;
      view.dataset.captureLayout = request.id;
      view.style.width = request.size ? `${request.size.width}px` : "";
      view.style.height = request.size ? `${request.size.height}px` : "";
      requestAnimationFrame(() => requestAnimationFrame(() => {
        if (!view.isConnected || view.dataset.captureLayout !== request.id) return;
        const rect = view.getBoundingClientRect();
        void invoke("browser_capture_layout_ready", {
          tabId: tab.id, requestId: request.id, width: rect.width, height: rect.height,
        }).catch((reason: unknown) => setError(String(reason)));
      }));
    }
  }, [props.state.tabs]);

  // 地址栏跟随当前标签，但员工正在输入时不覆盖他打了一半的内容
  useEffect(() => {
    if (!editing) setDraft(active?.url ?? "");
  }, [active?.url, editing]);

  /*
   * 分割线往左拖是把浏览器拉宽，所以位移要取反。拖动过程中不写存储，松手才写一次——
   * 每帧写一次存储会把主线程拖住，拖动本身先卡起来。
   */
  const [resizing, setResizing] = useState(false);
  const resizeStart = useRef<{ clientX: number; width: number } | null>(null);
  const widthRef = useRef(props.width);
  widthRef.current = props.width;
  /*
   * 回调放进 ref 再用：拖动时每动一下都会让父组件重渲染，依赖写成回调本身的话监听会被
   * 反复摘下重挂，松手那一下正好落在空档里就会漏掉，鼠标松了框还在跟着走。
   */
  const widthChangeRef = useRef(props.onWidthChange);
  widthChangeRef.current = props.onWidthChange;

  useEffect(() => {
    if (!resizing) return;
    const move = (event: PointerEvent) => {
      const start = resizeStart.current;
      if (!start) return;
      widthChangeRef.current(start.width - (event.clientX - start.clientX), false);
    };
    const finish = () => {
      widthChangeRef.current(widthRef.current, true);
      resizeStart.current = null;
      setResizing(false);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
    window.addEventListener("pointercancel", finish);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      window.removeEventListener("pointercancel", finish);
    };
  }, [resizing]);

  const startResize = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    resizeStart.current = { clientX: event.clientX, width: widthRef.current };
    setResizing(true);
    event.preventDefault();
  };

  const resizeWithKeyboard = (event: React.KeyboardEvent<HTMLDivElement>) => {
    let next: number | null = null;
    if (event.key === "ArrowLeft") next = widthRef.current + browserKeyboardStep;
    if (event.key === "ArrowRight") next = widthRef.current - browserKeyboardStep;
    if (event.key === "Home") next = maxAiBrowserWidth;
    if (event.key === "End") next = minAiBrowserWidth;
    if (next === null) return;
    event.preventDefault();
    /*
     * 先把 ref 推到新值再上报。按住方向键不放时，多次按键会落在同一帧里，而 ref 只在渲染
     * 时才从属性同步——不先推进的话这一串按键算出来的是同一个目标值，只挪得动一步。
     */
    widthRef.current = clampAiBrowserWidth(next);
    props.onWidthChange(widthRef.current, true);
  };

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    const value = draft.trim();
    if (!value) return;
    const url = /^[a-z][a-z0-9+.-]*:/i.test(value) ? value : `https://${value}`;
    setEditing(false);
    run("browser_employee_navigate", { url });
  };

  return (
    <section
      aria-label="浏览器"
      className={`ai-browser${resizing ? " is-resizing" : ""}`}
      data-driver={driver}
      data-open={props.open}>
      <div
        aria-label="调整浏览器宽度"
        aria-orientation="vertical"
        aria-valuemax={maxAiBrowserWidth}
        aria-valuemin={minAiBrowserWidth}
        aria-valuenow={props.width}
        className="ai-browser__resizer"
        onDoubleClick={() => props.onWidthChange(defaultAiBrowserWidth, true)}
        onKeyDown={resizeWithKeyboard}
        onPointerDown={startResize}
        role="separator"
        tabIndex={0}
        title="拖动调整浏览器宽度；双击恢复默认宽度"/>
      <header className="ai-browser__tabs">
        {tabs.map((tab) => (
          <button
            className="ai-browser__tab"
            data-active={tab.id === activeId}
            key={tab.id}
            onClick={() => run("browser_tab_activate", { tabId: tab.id })}
            title={tab.url}
            type="button"
          >
            <span className="ai-browser__tab-label">{tab.title || tab.url || "新标签"}</span>
            <span
              aria-label="关闭标签"
              className="ai-browser__tab-close"
              onClick={(event) => { event.stopPropagation(); run("browser_tab_close", { tabId: tab.id }); }}
              role="button"
              tabIndex={-1}
            ><Icon path={CLOSE}/></span>
          </button>
        ))}
        <button aria-label="新建标签" className="ai-browser__new" onClick={() => run("browser_tab_open")} title="新建标签" type="button">
          <Icon path={PLUS}/>
        </button>
        <span className="ai-browser__spacer"/>
        <button aria-label="收起浏览器" className="ai-browser__close" onClick={props.onClose} title="收起" type="button">
          <Icon path={CLOSE}/>
        </button>
      </header>

      <div className="ai-browser__bar">
        <button aria-label="后退" disabled={!active?.canGoBack} onClick={() => run("browser_back")} title="后退" type="button"><Icon path={BACK}/></button>
        <button aria-label="前进" disabled={!active?.canGoForward} onClick={() => run("browser_forward")} title="前进" type="button"><Icon path={FORWARD}/></button>
        <button aria-label="刷新" onClick={() => run("browser_reload")} title="刷新" type="button"><Icon path={RELOAD}/></button>
        <form className="ai-browser__address" onSubmit={submit}>
          <input
            aria-label="地址"
            onBlur={() => setEditing(false)}
            onChange={(event) => { setEditing(true); setDraft(event.target.value); }}
            placeholder="输入网址"
            spellCheck={false}
            value={draft}
          />
        </form>
        <button aria-label="检查" onClick={() => run("browser_devtools")} title="检查" type="button"><Icon path={INSPECT}/></button>
      </div>

      <div className="ai-browser__driver">
        <span>{driver === "employee" ? "你在操作，AI 已暂停" : "AI 操作中"}</span>
        <button
          onClick={() => run(driver === "employee" ? "browser_takeover_end" : "browser_takeover_begin")}
          type="button"
        >{driver === "employee" ? "交还给 AI" : "接管"}</button>
      </div>

      <div className="ai-browser__stage" ref={stageRef}/>
      {error ? <p className="ai-browser__error">{error}</p> : null}
    </section>
  );
}

/** 收起后回到工具栏的入口。浏览器没在跑时不出现，不放一个点不开的按钮。 */
export function AiBrowserToggle(props: { state: AiBrowserState; open: boolean; onToggle: () => void }) {
  const label = props.open ? "收起浏览器" : "展开浏览器";
  return (
    <button
      aria-expanded={props.open}
      aria-label={label}
      className="ai-browser-toggle"
      data-driver={props.state.driver}
      data-running={props.state.running}
      data-tip={label}
      onClick={props.onToggle}
      type="button"
    >
      <svg aria-hidden="true" fill="none" height="18" viewBox="0 0 18 18" width="18">
        <circle cx="9" cy="9" r="6.5" stroke="currentColor" strokeWidth="1.5"/>
        <path d="M2.5 9h13M9 2.5c1.8 2 2.7 4.2 2.7 6.5S10.8 13.5 9 15.5C7.2 13.5 6.3 11.3 6.3 9S7.2 4.5 9 2.5z" stroke="currentColor" strokeWidth="1.3"/>
      </svg>
      {/* AI 正在浏览的提示挪到这个点上：整颗图标染色会和"已展开"撞成同一个样子，分不出来 */}
      {props.state.running ? <span aria-hidden="true" className="ai-browser-toggle__dot"/> : null}
    </button>
  );
}
