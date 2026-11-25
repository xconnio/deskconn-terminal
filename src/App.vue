<script setup lang="ts">
import { computed, onMounted, provide, ref } from 'vue'
import { authService } from '@/services/authService'
import { wampService } from '@/services/wamp'
import { useSessionCacheStore } from '@/stores/sessionCache'
import { generateKeys } from '@/utils/crypto'
import { errorMessage } from '@/utils/errorMessage'
import { DESKTOP_OFFLINE_MESSAGE } from '@/utils/desktopError'
import { floatingWindowToolbarKey } from '@/composables/floatingWindowToolbar'
import TerminalPanel from '@/components/TerminalPanel.vue'
import TabStrip from '@/components/TabStrip.vue'
import type { Desktop } from '@/types'
import { useCliKey, type Key } from './cliKey'

declare global {
  interface Window {
    deskconnKey: {
      read(): Promise<Key | null>
      write(key: Key & { publicKey: string; name: string; expiresAt: string }): Promise<void>
    }
  }
}

const sessionCache = useSessionCacheStore()
const titleBar = window.deskconnTitleBar
const view = ref<'loading' | 'error' | 'login' | 'otp' | 'machines'>('loading')
const error = ref('')
const busy = ref(false)
const machines = ref<Desktop[]>([])
const email = ref('')
const password = ref('')
const code = ref('')

// Each tab is one TerminalPanel on one machine. The app's own tab strip (in the title
// bar) switches between them; each panel's built-in tab strip teleports into this
// hidden element, as it would into a FloatingWindow titlebar.
interface Tab {
  id: number
  machine: Desktop
  state: 'connecting' | 'ready' | 'offline'
  // The shell's live title from TerminalPanel, '' until it sets one.
  title: string
  // The tab you were on when this one was opened; Back returns there.
  openedFrom: number | null
}
const tabs = ref<Tab[]>([])
const activeId = ref<number | null>(null)
const activeTab = computed(() => tabs.value.find((t) => t.id === activeId.value))
const stripTabs = computed(() =>
  tabs.value.map((t) => ({ id: t.id, label: t.title ? `${t.machine.name} · ${t.title}` : t.machine.name })),
)
const panels = new Map<number, InstanceType<typeof TerminalPanel>>()
let nextId = 0
const hiddenToolbar = ref<HTMLElement | null>(null)
provide(floatingWindowToolbarKey, hiddenToolbar)

async function openTab(machine: Desktop) {
  tabs.value.push({ id: nextId++, machine, state: 'connecting', title: '', openedFrom: activeId.value })
  const tab = tabs.value[tabs.value.length - 1]!
  activeId.value = tab.id
  await connectTab(tab)
}

// A session connects even when the machine is offline, so ping deskconnd first,
// as deskconn-web-app's DesktopSessionHost does before showing a machine.
async function connectTab(tab: Tab) {
  tab.state = 'connecting'
  try {
    const session = await sessionCache.acquire(tab.machine.realm)
    if (!session) throw new Error('no session')
    await session.call('com.deskconn.ping')
    tab.state = 'ready'
  } catch {
    sessionCache.invalidate(tab.machine.realm)
    tab.state = 'offline'
  }
}

// Leaves an offline tab for the one it was opened from (or a neighbour, or the machine list).
function goBack(tab: Tab) {
  removeTab(tab.id)
  if (tabs.value.some((t) => t.id === tab.openedFrom)) activeId.value = tab.openedFrom
}

function removeTab(id: number) {
  const i = tabs.value.findIndex((t) => t.id === id)
  if (i === -1) return
  tabs.value.splice(i, 1)
  if (activeId.value === id) activeId.value = tabs.value[Math.min(i, tabs.value.length - 1)]?.id ?? null
}

// Asks first when a shell is still running something (TerminalPanel's own check).
async function closeTab(id: number) {
  if ((await panels.get(id)?.requestClose()) === false) return
  removeTab(id)
}

function setPanel(id: number, el: unknown) {
  if (el) panels.set(id, el as InstanceType<typeof TerminalPanel>)
  else panels.delete(id)
}

// "+" opens on the machine of the tab you're on; ▾ picks another.
const machineMenu = ref<{ x: number; y: number } | null>(null)

function openMachineMenu(e: MouseEvent) {
  const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
  machineMenu.value = { x: Math.min(r.left, window.innerWidth - 220), y: r.bottom + 4 }
}

function pickMachine(m: Desktop) {
  machineMenu.value = null
  void openTab(m)
}

// Same check as the CLI (deskconn ErrAuthenticationFailed): the key was revoked or rejected.
const isAuthError = (e: unknown) => errorMessage(e).includes('authentication_failed')

async function connect(key: Key) {
  const { session } = await authService.autoLogin(key.authId, key.privateKey)
  try {
    machines.value = (await authService.listDesktops(session)).args ?? []
  } finally {
    session.leave().catch(() => {})
  }
  useCliKey(key)
  view.value = 'machines'
}

async function start() {
  view.value = 'loading'
  error.value = ''
  try {
    const key = await window.deskconnKey.read()
    if (!key) {
      view.value = 'login'
      return
    }
    email.value = key.authId
    await connect(key)
  } catch (e) {
    error.value = isAuthError(e) ? 'Your session has ended. Please sign in again.' : errorMessage(e)
    view.value = isAuthError(e) ? 'login' : 'error'
  }
}

async function run(step: () => Promise<void>) {
  busy.value = true
  error.value = ''
  try {
    await step()
  } catch (e) {
    error.value = errorMessage(e)
  } finally {
    busy.value = false
  }
}

// Password is checked server-side, which then emails a one-time code.
const requestCode = () =>
  run(async () => {
    const { session } = await authService.requestLoginOtp(email.value, password.value)
    session.leave().catch(() => {})
    code.value = ''
    view.value = 'otp'
  })

// Same as `deskconn auth login`: register a fresh key, then save it where the CLI keeps its own.
const verifyCode = () =>
  run(async () => {
    const { privateKey, publicKey } = await generateKeys()
    const cra = await wampService.connectWithCRA(email.value, password.value)
    let verified
    try {
      verified = await authService.verifyLoginOtp(cra, email.value, password.value, code.value, publicKey)
    } finally {
      cra.leave().catch(() => {})
    }
    const { session, result } = await authService.autoLogin(email.value, privateKey)
    session.leave().catch(() => {})
    const key = { authId: email.value, privateKey }
    await window.deskconnKey.write({
      ...key,
      publicKey,
      name: result.args?.[0]?.name ?? '',
      expiresAt: verified?.args?.[0]?.expires_at ?? '',
    })
    password.value = ''
    await connect(key)
  })

onMounted(start)
</script>

<template>
  <div class="app">
    <!-- No drag region while the menu is open: Electron hands clicks on one to the window
         manager, so the backdrop would never see them and the menu wouldn't close. -->
    <header
      class="bar"
      :class="{ 'bar--overlay': titleBar === 'overlay', 'bar--mac': titleBar === 'mac', 'bar--menu-open': machineMenu }"
    >
      <span v-if="!tabs.length" class="title">DeskConn Terminal</span>
      <template v-else>
        <TabStrip
          class="strip"
          :tabs="stripTabs"
          :active-id="activeId"
          new-tab-title="New terminal"
          @switch="activeId = $event"
          @close="closeTab"
          @add="activeTab && openTab(activeTab.machine)"
        />
        <button class="menu-btn" title="New terminal on…" @click="openMachineMenu">
          <i class="bi bi-chevron-down"></i>
        </button>
      </template>
    </header>
    <div ref="hiddenToolbar" hidden></div>

    <!-- Closing the last tab comes back to the machine list. -->
    <div v-for="tab in tabs" v-show="tab.id === activeId" :key="tab.id" class="terminal">
      <TerminalPanel
        v-if="tab.state === 'ready'"
        :ref="(el) => setPanel(tab.id, el)"
        :realm="tab.machine.realm"
        :desktop-name="tab.machine.name"
        :focused="tab.id === activeId"
        @close="removeTab(tab.id)"
        @title="tab.title = $event"
      />
      <div v-else class="page">
        <button class="back" @click="goBack(tab)">
          <i class="bi bi-arrow-left"></i>{{ tabs.length > 1 ? 'Back' : 'Machines' }}
        </button>
        <div class="status">
          <i class="bi" :class="tab.state === 'connecting' ? 'bi-hourglass-split' : 'bi-wifi-off'"></i>
          <h2>{{ tab.machine.name }}</h2>
          <p v-if="tab.state === 'connecting'" class="muted">Connecting…</p>
          <template v-else>
            <p class="err">{{ DESKTOP_OFFLINE_MESSAGE }}</p>
            <button class="retry" @click="connectTab(tab)">Retry</button>
          </template>
        </div>
      </div>
    </div>

    <template v-if="machineMenu">
      <div class="menu-backdrop" @pointerdown="machineMenu = null"></div>
      <div class="menu" :style="{ top: machineMenu.y + 'px', left: machineMenu.x + 'px' }">
        <button v-for="m in machines" :key="m.realm" @click="pickMachine(m)">
          <i class="bi bi-terminal"></i>
          {{ m.name }}
        </button>
      </div>
    </template>

    <main v-if="!tabs.length" class="page">
      <p v-if="view === 'loading'" class="muted">Connecting…</p>

      <div v-else-if="view === 'error'" class="card">
        <p class="err">{{ error }}</p>
        <button @click="start">Retry</button>
        <button class="link" @click="view = 'login'">Sign in as someone else</button>
      </div>

      <form v-else-if="view === 'login'" class="card" @submit.prevent="requestCode">
        <h2>Sign in</h2>
        <p class="muted">Signs in the deskconn CLI on this computer too.</p>
        <input v-model="email" type="email" placeholder="Email" autocomplete="username" required />
        <input v-model="password" type="password" placeholder="Password" autocomplete="current-password" required />
        <p v-if="error" class="err">{{ error }}</p>
        <button :disabled="busy">{{ busy ? 'Signing in…' : 'Continue' }}</button>
      </form>

      <form v-else-if="view === 'otp'" class="card" @submit.prevent="verifyCode">
        <h2>Check your email</h2>
        <p class="muted">Enter the code sent to {{ email }}.</p>
        <input v-model="code" inputmode="numeric" autocomplete="one-time-code" placeholder="Code" required />
        <p v-if="error" class="err">{{ error }}</p>
        <button :disabled="busy">{{ busy ? 'Verifying…' : 'Verify' }}</button>
        <button type="button" class="link" @click="view = 'login'">Back</button>
      </form>

      <div v-else class="card wide">
        <h2>Your machines</h2>
        <p v-if="!machines.length" class="muted">No machines yet. Attach one with <code>deskconn</code>.</p>
        <button v-for="m in machines" :key="m.realm" class="machine" @click="openTab(m)">
          <i class="bi bi-terminal"></i>
          <span>{{ m.name }}</span>
          <small v-if="m.role" class="muted">{{ m.role }}</small>
        </button>
        <button class="link" @click="start">Refresh</button>
      </div>
    </main>
  </div>
</template>

<style>
html, body, #app { height: 100%; margin: 0; }
body {
  background: #1e1e1e;
  color: #e2e8f0;
  font-family: system-ui, -apple-system, 'Segoe UI', sans-serif;
}
</style>

<style scoped>
.app { height: 100%; display: flex; flex-direction: column; }
.bar {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  flex-shrink: 0;
  height: 38px;
  box-sizing: border-box;
  padding: 0.25rem 0.6rem;
  background: #141416;
  box-shadow: inset 0 -1px rgba(255, 255, 255, 0.08);
  user-select: none;
  /* Drag the window by the empty title bar; tabs and buttons stay clickable. */
  -webkit-app-region: drag;
}
.bar :deep(button) { -webkit-app-region: no-drag; }
.bar--menu-open { -webkit-app-region: no-drag; }
/* Windows/Linux: stop short of the window buttons drawn over the right end. */
.bar--overlay { width: env(titlebar-area-width, 100%); }
/* macOS: clear the traffic lights on the left. */
.bar--mac { padding-left: 80px; }
.title { font-size: 0.85rem; font-weight: 600; color: #cbd5e1; }
/* TabStrip normally sits in a FloatingWindow titlebar and bleeds into its padding. */
.strip { margin: -0.25rem 0 -0.25rem -0.6rem; flex: 0 1 auto; }
/* Shrink the list to its tabs so ▾ sits right after "+", not at the far end. */
.strip :deep(.tabs-list) { flex: 0 1 auto; }
.menu-btn {
  padding: 0.2rem 0.4rem;
  background: transparent;
  color: #a9a9a9;
  font-size: 0.7rem;
  border-radius: 3px;
}
.menu-btn:hover { background: rgba(255, 255, 255, 0.1); color: #e2e8f0; }
.terminal { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.menu-backdrop { position: fixed; inset: 0; z-index: 9998; }
.menu {
  position: fixed;
  z-index: 9999;
  min-width: 180px;
  padding: 0.3rem;
  background: #252526;
  border: 1px solid #3a3a3a;
  border-radius: 8px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
}
.menu button {
  display: flex;
  align-items: center;
  gap: 0.55rem;
  width: 100%;
  padding: 0.45rem 0.7rem;
  background: none;
  color: #e2e8f0;
  font-size: 0.85rem;
  text-align: left;
  border-radius: 5px;
}
.menu button:hover { background: rgba(255, 255, 255, 0.08); }
.menu .bi { color: #60a5fa; }
.page { position: relative; flex: 1; display: grid; place-items: center; overflow: auto; padding: 2rem; }
.status { display: flex; flex-direction: column; align-items: center; gap: 0.5rem; text-align: center; }
.status .bi { font-size: 2.5rem; color: #64748b; }
.status h2 { margin: 0; font-size: 1.4rem; }
.status p { font-size: 1.05rem; }
.status .retry { margin-top: 0.75rem; min-width: 140px; }
.back {
  position: absolute;
  top: 0.75rem;
  left: 0.75rem;
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  line-height: 1;
  padding: 0.4rem 0.6rem;
  background: none;
  color: #cbd5e1;
  font-size: 0.9rem;
}
.back:hover { background: rgba(255, 255, 255, 0.08); }
/* bootstrap-icons nudges glyphs down for inline text; centred by flex instead. */
.back .bi::before { vertical-align: 0; }
.card { width: 320px; display: flex; flex-direction: column; gap: 0.6rem; }
.card.wide { width: 420px; }
h2 { margin: 0 0 0.25rem; font-size: 1.25rem; }
.muted { color: #94a3b8; font-size: 0.85rem; margin: 0; }
.err { color: #f87171; font-size: 0.85rem; margin: 0; }
input {
  padding: 0.6rem 0.75rem;
  border: 1px solid #3a3a3a;
  border-radius: 8px;
  background: #141416;
  color: inherit;
  font: inherit;
}
input:focus { outline: 2px solid #2563eb; outline-offset: -1px; }
button {
  padding: 0.6rem 0.75rem;
  border: none;
  border-radius: 8px;
  background: #2563eb;
  color: #fff;
  font: inherit;
  cursor: pointer;
}
button:disabled { opacity: 0.6; cursor: default; }
button.link { background: none; color: #93c5fd; padding: 0.25rem; }
.machine {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  background: #2a2a2a;
  color: inherit;
  text-align: left;
}
.machine:hover { background: #333; }
.machine span { flex: 1; }
</style>
