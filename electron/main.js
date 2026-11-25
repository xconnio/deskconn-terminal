// Electron shell for the DeskConn terminal. Serves the Vite build from
// app://deskconn (a secure origin, as WebTransport needs) and gives the page
// the deskconn CLI's login key, so `deskconn auth login` and this app share one sign-in.
import { app, BrowserWindow, ipcMain, net, protocol, shell } from 'electron'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

const ORIGIN = 'app://deskconn'
const DIST = path.join(import.meta.dirname, '..', 'dist')
const DEV_URL = process.argv.find((a) => a.startsWith('--dev-url='))?.slice('--dev-url='.length)
// Pages this app serves; by origin, so http://localhost:51740 doesn't pass for :5174.
const APP_PREFIXES = [`${ORIGIN}/`, ...(DEV_URL ? [`${new URL(DEV_URL).origin}/`] : [])]
const isAppUrl = (url) => APP_PREFIXES.some((p) => url.startsWith(p))
// Same directory and file format as the CLI (deskconn internal/deskconn/helpers.go Login):
//   id_ed25519      "<private key hex> <authid> [<expires at RFC3339>]"
//   id_ed25519.pub  "<public key hex> <authid> <name>"
const CFG_DIR = path.join(os.homedir(), '.deskconn')
const PRIV = path.join(CFG_DIR, 'id_ed25519')
const PUB = path.join(CFG_DIR, 'id_ed25519.pub')

// Vite serves node_modules (deskconn-web-app's source) as immutable; without this, the
// disk cache keeps serving a replaced copy (a new pinned commit) across restarts.
if (DEV_URL) app.commandLine.appendSwitch('disable-http-cache')

protocol.registerSchemesAsPrivileged([
  { scheme: 'app', privileges: { standard: true, secure: true, supportFetchAPI: true, corsEnabled: true, stream: true } },
])

async function serve(request) {
  let pathname
  try { pathname = decodeURIComponent(new URL(request.url).pathname) } catch { return new Response('Bad request', { status: 400 }) }
  const file = path.normalize(path.join(DIST, pathname))
  if (!file.startsWith(DIST + path.sep)) return new Response('Forbidden', { status: 403 })
  const res = await net.fetch(pathToFileURL(file).toString()).catch(() => null)
  return res?.ok ? res : net.fetch(pathToFileURL(path.join(DIST, 'index.html')).toString())
}

function checkSender(event) {
  if (!isAppUrl(event.senderFrame?.url ?? '')) throw new Error('not allowed')
}

// Null when there's no key or it has expired: the page then asks to sign in.
async function readKey() {
  let line
  try { line = (await readFile(PRIV, 'utf8')).trim() } catch (err) {
    if (err.code === 'ENOENT') return null
    throw err
  }
  const [privateKey, authId, expiresAt] = line.split(' ')
  if (!privateKey || !authId) return null
  if (expiresAt && new Date(expiresAt).getTime() <= Date.now()) return null
  return { authId, privateKey }
}

// The values land in a space-separated file the CLI parses, so none may break its format.
async function writeKey({ privateKey, publicKey, authId, name, expiresAt }) {
  const hex = /^[0-9a-f]{64}$/
  const word = /^\S+$/
  if (!hex.test(privateKey) || !hex.test(publicKey) || !word.test(authId)) throw new Error('invalid key')
  if (expiresAt && !word.test(expiresAt)) throw new Error('invalid expiry')
  const safeName = String(name ?? '').replace(/[\r\n]/g, ' ')
  await mkdir(CFG_DIR, { recursive: true, mode: 0o700 })
  await writeFile(PRIV, `${privateKey} ${authId}${expiresAt ? ` ${expiresAt}` : ''}\n`, { mode: 0o600 })
  await writeFile(PUB, `${publicKey} ${authId} ${safeName}\n`, { mode: 0o600 })
}

ipcMain.handle('key:read', (event) => { checkSender(event); return readKey() })
ipcMain.handle('key:write', (event, key) => { checkSender(event); return writeKey(key) })

function createWindow() {
  const win = new BrowserWindow({
    width: 1100,
    height: 700,
    backgroundColor: '#1e1e1e',
    autoHideMenuBar: true,
    // The page's header is the title bar (tabs in it); the OS keeps only its window buttons.
    titleBarStyle: 'hidden',
    ...(process.platform === 'darwin'
      ? { trafficLightPosition: { x: 12, y: 12 } }
      : { titleBarOverlay: { color: '#141416', symbolColor: '#e2e8f0', height: 38 } }),
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      preload: path.join(import.meta.dirname, 'preload.cjs'),
    },
  })
  // Links clicked in the terminal open in the system browser, never in the app window.
  win.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:/.test(url)) void shell.openExternal(url)
    return { action: 'deny' }
  })
  win.webContents.on('will-navigate', (e, url) => {
    if (!isAppUrl(url)) e.preventDefault()
  })
  if (DEV_URL) {
    // Vite may still be starting. -3 (ERR_ABORTED) is a superseded navigation, not a failure.
    win.webContents.on('did-fail-load', (_e, code, _d, url, main) => {
      if (main && code !== -3) setTimeout(() => win.loadURL(url), 500)
    })
  }
  win.loadURL(DEV_URL ?? `${ORIGIN}/`)
}

app.whenReady().then(() => {
  protocol.handle('app', serve)
  createWindow()
  app.on('activate', () => { if (BrowserWindow.getAllWindows().length === 0) createWindow() })
})

app.on('window-all-closed', () => { if (process.platform !== 'darwin') app.quit() })
