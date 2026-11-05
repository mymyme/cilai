const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('cilaiDocumentLayout', {
  take: () => ipcRenderer.invoke('document-layout:take'),
  complete: html => ipcRenderer.invoke('document-layout:complete', html),
  fail: () => ipcRenderer.invoke('document-layout:failed'),
});
