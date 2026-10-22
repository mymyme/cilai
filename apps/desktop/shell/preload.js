const { contextBridge, ipcRenderer } = require("electron");

/**
 * 界面调用宿主的通道。
 *
 * 界面侧的对口是它的宿主接口模块，两边只经这一个对象往来。此前这里叫的是前一代宿主框架
 * 的内部名字——那是为了让当时未经改造的界面原样跑起来，前一代宿主壳退役后该理由不再成立。
 */
let nextCallbackId = 0;
const callbacks = new Map();

ipcRenderer.on("host:callback", (_event, message) => {
  const entry = callbacks.get(message.id);
  if (!entry) return;
  entry.callback(message.payload);
  if (entry.once) callbacks.delete(message.id);
});

// 客户端更新使用独立、固定的桥接面。界面不能传入更新源、下载地址或本地路径；这些值只能
// 来自正式安装包内的发布配置与 electron-updater 校验结果。
contextBridge.exposeInMainWorld("__CILAI_UPDATER__", {
  getState: () => ipcRenderer.invoke("updater:get-state"),
  check: () => ipcRenderer.invoke("updater:check"),
  checkSelectedChannel: () => ipcRenderer.invoke('updater:check-selected-channel'),
  selectChannel: (channel) => ipcRenderer.invoke('updater:select-channel', channel),
  download: () => ipcRenderer.invoke("updater:download"),
  pause: () => ipcRenderer.invoke('updater:pause'),
  setAutoDownload: (enabled) => ipcRenderer.invoke('updater:set-auto-download', enabled),
  install: () => ipcRenderer.invoke("updater:install"),
  onState: (handler) => {
    const listener = (_event, state) => handler(state);
    ipcRenderer.on("updater:state", listener);
    return () => ipcRenderer.removeListener("updater:state", listener);
  },
});

window.addEventListener('online', () => {
  void ipcRenderer.invoke('updater:network-restored').catch(error => console.error('网络恢复后检查更新失败', error));
});

contextBridge.exposeInMainWorld("__CILAI_HOST__", {
  invoke: (command, args) =>
    ipcRenderer.invoke("host:invoke", { command, args: args ?? {}, options: null }),
  // 注销监听时界面先清渲染侧的回调表再通知宿主。缺了这一步，注销的第一步就会抛错，
  // 监听既撤不掉也报不出原因。此前它单独占一个全局对象，那是前一代框架的形状。
  unregisterListener: (eventId) => {
    callbacks.delete(eventId);
  },
  transformCallback: (callback, once = false) => {
    nextCallbackId += 1;
    callbacks.set(nextCallbackId, { callback, once });
    return nextCallbackId;
  },
  // 本地文件协议未启用。返回一个形似可用、实则取不到内容的地址，只会把失败推迟到加载时
  // 且不给原因，因此直接拒绝。
  convertFileSrc: (filePath) => {
    throw new Error(`本地文件协议未启用，无法转换：${filePath}`);
  },
});
