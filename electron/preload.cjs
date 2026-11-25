// eslint-disable-next-line @typescript-eslint/no-require-imports -- sandboxed preloads can't be ES modules
const { contextBridge, ipcRenderer } = require('electron')

// Same as deskconn-web-app's preload: how App.vue lays out its title bar.
contextBridge.exposeInMainWorld('deskconnTitleBar', process.platform === 'darwin' ? 'mac' : 'overlay')

// The deskconn CLI's login key (~/.deskconn/id_ed25519), shared with this app.
contextBridge.exposeInMainWorld('deskconnKey', {
  read: () => ipcRenderer.invoke('key:read'),
  write: (key) => ipcRenderer.invoke('key:write', key),
})
