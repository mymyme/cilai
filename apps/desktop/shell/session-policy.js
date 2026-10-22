// 离屏会话的能力策略。容器与 AI 浏览器共用同一套口径。

/**
 * 摄像头、麦克风、定位、通知、剪贴板读取、设备访问默认拒绝。
 * 下载只交给明确注册的应用文件保存处理器；AI浏览器不注册该能力。
 *
 * 换到 Chromium 后这些必须显式声明——它对一部分权限请求的默认行为是放行，而原先的
 * 系统内核把这类请求交给操作系统处理。容器与 AI 浏览器都是离屏的，权限弹窗无处呈现，
 * 静默放行等于让页面在员工看不见的地方取得这些能力。需要时按应用逐项放开，不留默认口子。
 */
const policies = new WeakMap();

function denyAmbientCapabilities(target, label, downloadHandler = null) {
  const previous = policies.get(target);
  if (previous) {
    previous.downloadHandler = downloadHandler;
    return;
  }
  const policy = { downloadHandler };
  policies.set(target, policy);
  const refuse = (kind) => console.warn(`[${label}] 环境能力请求已拒绝：${kind}`);
  target.setPermissionRequestHandler((_contents, permission, callback) => {
    refuse(permission);
    callback(false);
  });
  // 状态查询不留痕：它由浏览器例行发起，每次页面加载都有若干次，打印会淹没真正的请求
  target.setPermissionCheckHandler(() => false);
  target.setDevicePermissionHandler(() => false);
  target.on("will-download", (event, item, contents) => {
    try {
      if (policy.downloadHandler?.(item, contents) === true) return;
    } catch (error) {
      console.warn(`[${label}] 文件保存未能开始：${error.message}`);
    }
    event.preventDefault();
    refuse(`下载 ${item.getFilename()}`);
  });
}

module.exports = { denyAmbientCapabilities };
