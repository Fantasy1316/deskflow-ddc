import { contextBridge, ipcRenderer } from 'electron'
contextBridge.exposeInMainWorld('screenbridge', {
  snapshot: () => ipcRenderer.invoke('screenbridge:snapshot'),
  act: (action: unknown) => ipcRenderer.invoke('screenbridge:action', action),
  reportError: (error: unknown) => ipcRenderer.invoke('screenbridge:renderer-error', error),
  subscribe: (listener: (state: unknown) => void) => {
    const handler = (_: Electron.IpcRendererEvent, state: unknown) => listener(state)
    ipcRenderer.on('screenbridge:state', handler)
    return () => ipcRenderer.removeListener('screenbridge:state', handler)
  },
})
