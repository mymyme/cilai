/**
 * 宿主接口。
 *
 * 界面与宿主壳之间只经这一个模块往来。此前界面直接引用某一个宿主框架的客户端包，
 * 于是"界面依赖哪个壳"这件事散落在十六个文件里；换壳就要改十六处，两个壳并存期间
 * 也说不清哪些调用是共通的、哪些是某一端特有的。
 *
 * 底层是宿主壳在预加载脚本里暴露的一个对象，命令名与事件名由本文件与它对齐。宿主能力
 * 有增减时，改动只落在这两处。
 */

type HostChannel = {
  invoke: <T>(command: string, args: object) => Promise<T>;
  transformCallback: (callback: (payload: unknown) => void, once?: boolean) => number;
  unregisterListener: (eventId: number) => void;
  metadata?: { currentWindow?: { label?: string } };
};

function channel(): HostChannel {
  const value = (globalThis as { __CILAI_HOST__?: HostChannel }).__CILAI_HOST__;
  if (!value) throw new Error("宿主接口不可用：界面未运行在工作台宿主壳中");
  return value;
}

/** 调用宿主命令。失败以异常回到调用方，不返回空值掩盖失败。 */
export async function invoke<T>(command: string, args: object = {}): Promise<T> {
  return channel().invoke<T>(command, args);
}

/**
 * 订阅宿主事件，返回解绑函数。
 *
 * 处理函数直接收到负载本身。宿主协议送来的是一个带事件名与回调号的信封，那两项是
 * 投递用的，调用方每次都要从信封里取一层，取错也不会有人发现。
 */
export async function listen<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  const envelope = (value: unknown) => handler((value as { payload: T }).payload);
  const eventId = await invoke<number>("plugin:event|listen", {
    event,
    target: { kind: "Any" },
    handler: channel().transformCallback(envelope),
  });
  return async () => {
    // 先清渲染侧的回调表再通知宿主：缺了这一步，解绑的第一步就会抛错，
    // 事件既撤不掉也报不出原因。
    channel().unregisterListener(eventId);
    await invoke("plugin:event|unlisten", { event, eventId });
  };
}

/**
 * 当前窗口的标识。
 *
 * 窗口寻址是宿主特有的概念：现有壳按标签区分多个窗口，新壳由宿主按发出调用的页面直接
 * 认定，不看这一项。取不到时传空而不是编一个出来——编出来的标签在现有壳里会指向不存在
 * 的窗口，那种失败比当场传空难查得多。
 */
function currentWindowLabel(): string | null {
  return channel().metadata?.currentWindow?.label ?? null;
}

/** 窗口控制。界面自绘标题栏经此调用；macOS 的系统窗口按钮不走这里。 */
export const hostWindow = {
  async isMaximized(): Promise<boolean> {
    return invoke<boolean>("plugin:window|is_maximized", { label: currentWindowLabel() });
  },
  async minimize(): Promise<void> {
    await invoke("plugin:window|minimize", { label: currentWindowLabel() });
  },
  async toggleMaximize(): Promise<void> {
    await invoke("plugin:window|toggle_maximize", { label: currentWindowLabel() });
  },
  async close(): Promise<void> {
    await invoke("plugin:window|close", { label: currentWindowLabel() });
  },
  /** 窗口尺寸变化。最大化与还原都会触发，界面据此同步按钮状态。 */
  async onResized(handler: () => void): Promise<() => void> {
    return listen("tauri://resize", handler);
  },
};

/**
 * 写入系统剪贴板。
 *
 * 文本走 `text`；协议里另有一个可选的 `label`，那是窗口标签而不是内容，不要混用。
 */
export async function writeClipboard(value: string): Promise<void> {
  await invoke("plugin:clipboard-manager|write_text", { text: value });
}
