const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("bridge", {
  onFrame: (handler) => ipcRenderer.on("frame", (_event, dataUrl) => handler(dataUrl)),
  pointer: (payload) => ipcRenderer.send("pointer", payload),
  composition: (payload) => ipcRenderer.send("composition", payload),
});
