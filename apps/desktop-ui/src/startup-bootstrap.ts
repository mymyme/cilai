import { readTheme } from "./theme";

/** 可见窗口先给品牌页一次绘制机会；窗口转到后台时继续启动，不等待隐藏窗口的动画帧。 */
export function waitForWorkbenchPaint(): Promise<void> {
  if (document.visibilityState !== "visible") return Promise.resolve();
  return new Promise(resolve => {
    let firstFrame: number | null = null, secondFrame: number | null = null;
    let finished = false;
    const complete = () => {
      if (finished) return;
      finished = true;
      if (firstFrame !== null) window.cancelAnimationFrame(firstFrame);
      if (secondFrame !== null) window.cancelAnimationFrame(secondFrame);
      document.removeEventListener("visibilitychange", visibilityChanged);
      resolve();
    };
    const visibilityChanged = () => { if (document.visibilityState !== "visible") complete(); };
    document.addEventListener("visibilitychange", visibilityChanged);
    firstFrame = window.requestAnimationFrame(() => {
      if (!finished) secondFrame = window.requestAnimationFrame(complete);
    });
  });
}

/** 保留 HTML 中的启动反馈，直到真正的主界面接管；资源错误也有可操作的页面。 */
export async function bootstrapWorkbench({
  load = () => import("./main"),
  reload = () => window.location.reload(),
}: { load?: () => Promise<unknown>; reload?: () => void } = {}) {
  try {
    const theme = readTheme();
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
    await waitForWorkbenchPaint();
    await load();
  } catch (error) {
    console.error("startup_frontend_load_failed", error);
    const surface = document.querySelector<HTMLElement>("[data-workbench-bootstrap]");
    if (!surface) return;
    surface.setAttribute("role", "alert");
    surface.setAttribute("aria-busy", "false");
    surface.setAttribute("aria-label", "工作台界面加载失败");
    surface.querySelector("h2")!.textContent = "工作台界面加载失败";
    surface.querySelector("p")!.textContent = "请重新打开；如果仍无法打开，请重新下载客户端。";
    surface.querySelector(".workbench-loading-spinner")?.remove();
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = "重新打开";
    button.addEventListener("click", reload);
    surface.append(button);
  }
}
