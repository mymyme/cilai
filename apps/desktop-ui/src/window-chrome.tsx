/**
 * macOS 用系统标题栏叠加窗口内容，红绿灯由系统绘制；Windows 关闭了系统装饰
 * （打包配置里关闭了系统装饰），标题栏完全交给这里自绘。
 * 是否需要自绘由后端 host_platform 命令在启动时告知一次，不在前端猜测平台。
 */
import { hostWindow } from "./host";
import { useCallback, useEffect, useState } from "react";
import { invoke } from "./host";
import { WindowCloseIcon, WindowMaximizeIcon, WindowMinimizeIcon, WindowRestoreIcon } from "./icons";

export function useHostPlatform() {
  const [hostPlatform, setHostPlatform] = useState<string | null>(null);
  useEffect(() => {
    void invoke<string>("host_platform")
      .then(setHostPlatform)
      .catch(() => setHostPlatform("other"));
  }, []);
  return hostPlatform;
}

export function useWindowMaximized(active: boolean) {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    if (!active) return;
    let disposed = false;
    const syncMaximized = () => void hostWindow.isMaximized().then((value) => { if (!disposed) setMaximized(value); });
    syncMaximized();
    let unlisten: (() => void) | undefined;
    void hostWindow.onResized(syncMaximized).then((dispose) => { if (disposed) dispose(); else unlisten = dispose; });
    return () => { disposed = true; unlisten?.(); };
  }, [active]);
  return maximized;
}

export function WindowControls({ maximized }: { maximized: boolean }) {
  const minimize = useCallback(() => { void hostWindow.minimize(); }, []);
  const toggleMaximize = useCallback(() => { void hostWindow.toggleMaximize(); }, []);
  const close = useCallback(() => { void hostWindow.close(); }, []);
  return <div className="window-controls">
    <button aria-label="最小化" className="window-control" onClick={minimize} title="最小化" type="button"><WindowMinimizeIcon/></button>
    <button aria-label={maximized ? "还原" : "最大化"} className="window-control" onClick={toggleMaximize} title={maximized ? "还原" : "最大化"} type="button">{maximized ? <WindowRestoreIcon/> : <WindowMaximizeIcon/>}</button>
    <button aria-label="关闭" className="window-control window-control-close" onClick={close} title="关闭" type="button"><WindowCloseIcon/></button>
  </div>;
}
