import { writeClipboard as writeText } from "./host";

/**
 * 复制统一走原生剪贴板，不用 navigator.clipboard。
 *
 * 应用跑在 WKWebView 里，异步剪贴板 API 的授权比 Safari 严得多：无论是否在用户手势内，
 * writeText 一律抛 NotAllowedError；退回 document.execCommand("copy") 同样被拒。
 * 右键菜单里那三个"复制"因此从来没成功过。插件绕开 WebView，直接写 NSPasteboard。
 */
export async function writeClipboard(value: string): Promise<void> {
  await writeText(value);
}
