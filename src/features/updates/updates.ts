import { useSyncExternalStore } from 'react'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { errorMessage } from '@/lib/ipc'

/**
 * In-app updates. The engine's updater plugin fetches `latest.json` from the
 * release feed (tauri.conf.json → plugins.updater.endpoints), verifies the
 * bundle's minisign signature against the public key compiled into the app,
 * installs it and relaunches. Startup checks and the Settings card share
 * this state.
 */

/** Where people can download releases by hand. */
export const RELEASES_URL = 'https://github.com/nzap-labs/nzap-engine-releases/releases/latest'

export type UpdatePhase =
  'idle' | 'checking' | 'current' | 'available' | 'downloading' | 'installed' | 'error'

export interface UpdateState {
  phase: UpdatePhase
  version: string | null
  notes: string | null
  date: string | null
  downloaded: number
  total: number | null
  error: string | null
  checkedAt: number | null
}

const AUTO_KEY = 'nzap.updates.auto'

let state: UpdateState = {
  phase: 'idle',
  version: null,
  notes: null,
  date: null,
  downloaded: 0,
  total: null,
  error: null,
  checkedAt: null,
}
let pending: Update | null = null
const listeners = new Set<() => void>()

function set(patch: Partial<UpdateState>) {
  state = { ...state, ...patch }
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useUpdates(): UpdateState {
  return useSyncExternalStore(
    subscribe,
    () => state,
    () => state,
  )
}

export function autoCheckEnabled(): boolean {
  try {
    return localStorage.getItem(AUTO_KEY) !== 'off'
  } catch {
    return true
  }
}

export function setAutoCheck(enabled: boolean) {
  try {
    localStorage.setItem(AUTO_KEY, enabled ? 'on' : 'off')
  } catch {
    // The preference just does not stick.
  }
  for (const listener of listeners) listener()
}

/** A feed that is not reachable yet reads as an error worth explaining. */
function describe(error: unknown): string {
  const message = errorMessage(error, 'Could not check for updates.')
  if (/valid release JSON|404|Not Found/i.test(message))
    return 'No update feed is published yet. You can still download releases from GitHub.'
  if (/network|dns|connect|timed out|timeout/i.test(message))
    return 'Could not reach the update server. Check your connection.'
  return message
}

/** Ask the feed for a newer version. Resolves with the new version, if any. */
export async function checkForUpdates(): Promise<string | null> {
  if (state.phase === 'checking' || state.phase === 'downloading') return state.version
  set({ phase: 'checking', error: null })
  try {
    const update = await check({ timeout: 20_000 })
    pending = update
    if (!update) {
      set({ phase: 'current', version: null, checkedAt: Date.now() })
      return null
    }
    set({
      phase: 'available',
      version: update.version,
      notes: update.body ?? null,
      date: update.date ?? null,
      checkedAt: Date.now(),
    })
    return update.version
  } catch (error) {
    set({ phase: 'error', error: describe(error), checkedAt: Date.now() })
    return null
  }
}

/** Download, verify and install the pending update, then relaunch. */
export async function installUpdate(): Promise<void> {
  if (!pending) return
  set({ phase: 'downloading', downloaded: 0, total: null, error: null })
  try {
    await pending.downloadAndInstall((event) => {
      if (event.event === 'Started') set({ total: event.data.contentLength ?? null })
      else if (event.event === 'Progress')
        set({ downloaded: state.downloaded + event.data.chunkLength })
    })
    set({ phase: 'installed' })
    // Windows installers close the app themselves; elsewhere restart into
    // the new version.
    await relaunch()
  } catch (error) {
    set({ phase: 'error', error: errorMessage(error, 'The update could not be installed.') })
  }
}

/** Test hook. */
export function resetUpdates() {
  pending = null
  set({
    phase: 'idle',
    version: null,
    notes: null,
    date: null,
    downloaded: 0,
    total: null,
    error: null,
    checkedAt: null,
  })
}
