import type { PiniaPluginContext } from 'pinia'
import { authService } from '@/services/authService'
import { CryptoSignAuthenticator, type Session } from '@/services/xconn'
import type { useAuthStore } from '@/stores/auth'

// The deskconn CLI's login key (~/.deskconn/id_ed25519), handed over by electron/preload.cjs.
export type Key = { authId: string; privateKey: string }

let key: Key | null = null

export function useCliKey(k: Key) {
  key = k
}

// deskconn-web-app's auth store opens machine sessions with the browser's own device
// key. Its session cache (so TerminalPanel) only goes through these two, so here they
// use the CLI's key instead.
export function cliKeyPlugin({ store }: PiniaPluginContext) {
  if (store.$id !== 'auth') return
  const auth = store as unknown as ReturnType<typeof useAuthStore>
  auth.shellWamp = async (realm: string) =>
    key ? authService.shellDesktop(key.authId, key.privateKey, realm) : false
  auth.shellWebRTC = async (realm: string, routed: Session) =>
    key
      ? authService.shellWebRTCDesktop(new CryptoSignAuthenticator(key.authId, key.privateKey, {}), realm, routed)
      : false
}
