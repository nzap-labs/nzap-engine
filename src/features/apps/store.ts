import { useSyncExternalStore } from 'react'
import type { AppOutputPayload, AppSpec } from '@/types/app'
import type { NotebookParamValues } from '@/types/notebook'

/**
 * What the Apps view remembers: how long each app really took on each
 * accelerator (kept across launches, so estimates become your own numbers),
 * which runtimes already have an app loaded (warm), and the results of this
 * launch's runs.
 */

export interface Timing {
  setup: number | null
  run: number
  at: number
}

export interface AppResult {
  id: string
  appId: string
  startedAt: number
  seconds: number
  warm: boolean
  runtime: string
  accelerator: string
  values: NotebookParamValues
  outputs: ResolvedOutput[]
}

export interface ResolvedOutput extends AppOutputPayload {
  /** A blob: URL for media fetched from the runtime. */
  url?: string
  /** Why the media could not be fetched. */
  fetchError?: string
}

interface State {
  timings: Record<string, Record<string, Timing>>
  /** `<runtime>@<createdAt>` → app slugs loaded in its kernel. */
  warm: Record<string, string[]>
  results: Record<string, AppResult[]>
}

const STORAGE_KEY = 'nzap.apps.v1'
const MAX_RESULTS = 12

function load(): State {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw) {
      const saved = JSON.parse(raw) as Partial<State>
      return { timings: saved.timings ?? {}, warm: saved.warm ?? {}, results: {} }
    }
  } catch {
    // Storage can be unavailable (private windows, previews); start empty.
  }
  return { timings: {}, warm: {}, results: {} }
}

let state: State = load()
const listeners = new Set<() => void>()

function persist() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ timings: state.timings, warm: state.warm }))
  } catch {
    // Not fatal: estimates fall back to the app's own numbers.
  }
}

function update(next: State) {
  state = next
  persist()
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useAppStore<T>(select: (state: State) => T): T {
  return useSyncExternalStore(
    subscribe,
    () => select(state),
    () => select(state),
  )
}

export const runtimeKey = (runtime: { name: string; createdAt: number }) =>
  `${runtime.name}@${Math.round(runtime.createdAt)}`

export function isWarm(slug: string, runtime: { name: string; createdAt: number }): boolean {
  return state.warm[runtimeKey(runtime)]?.includes(slug) ?? false
}

export function markWarm(
  slug: string,
  runtime: { name: string; createdAt: number },
  warm: boolean,
) {
  const key = runtimeKey(runtime)
  const current = state.warm[key] ?? []
  const next = warm
    ? current.includes(slug)
      ? current
      : [...current, slug]
    : current.filter((entry) => entry !== slug)
  if (next === current) return
  update({ ...state, warm: { ...state.warm, [key]: next } })
}

/** Forget runtimes that no longer exist. */
export function pruneWarm(live: { name: string; createdAt: number }[]) {
  const keep = new Set(live.map(runtimeKey))
  const entries = Object.entries(state.warm).filter(([key]) => keep.has(key))
  if (entries.length !== Object.keys(state.warm).length)
    update({ ...state, warm: Object.fromEntries(entries) })
}

export function recordTiming(
  slug: string,
  accelerator: string,
  seconds: { setup?: number; run?: number },
  warm: boolean,
) {
  if (typeof seconds.run !== 'number') return
  const previous = state.timings[slug]?.[accelerator]
  const timing: Timing = {
    // A warm run says nothing about setup; keep the last cold measurement.
    setup: warm ? (previous?.setup ?? null) : (seconds.setup ?? null),
    run: seconds.run,
    at: Date.now(),
  }
  update({
    ...state,
    timings: { ...state.timings, [slug]: { ...state.timings[slug], [accelerator]: timing } },
  })
}

export function addResult(result: AppResult) {
  const list = [result, ...(state.results[result.appId] ?? [])]
  for (const dropped of list.slice(MAX_RESULTS))
    for (const output of dropped.outputs) if (output.url) URL.revokeObjectURL(output.url)
  update({
    ...state,
    results: { ...state.results, [result.appId]: list.slice(0, MAX_RESULTS) },
  })
}

export function clearResults(appId: string) {
  for (const result of state.results[appId] ?? [])
    for (const output of result.outputs) if (output.url) URL.revokeObjectURL(output.url)
  update({ ...state, results: { ...state.results, [appId]: [] } })
}

export interface Estimate {
  setup: number
  run: number
  /** Where the numbers come from. */
  source: 'yours' | 'app'
  /** The accelerator they were measured on. */
  on: string
}

/**
 * How long the next run should take: your own last measurement on this
 * accelerator when there is one, else the app's published numbers. A warm
 * runtime skips setup.
 */
export function estimateFor(
  spec: AppSpec,
  slug: string,
  accelerator: string | null,
  timings: State['timings'],
  warm: boolean,
): Estimate {
  const accel = accelerator ?? spec.runtime.accelerator
  const mine = timings[slug]?.[accel]
  const setup = warm ? 0 : (mine?.setup ?? spec.estimates.setup)
  if (mine) return { setup, run: mine.run, source: 'yours', on: accel }
  return {
    setup,
    run: spec.estimates.run,
    source: 'app',
    on: spec.estimates.measuredOn ?? spec.runtime.accelerator,
  }
}

/** Test hook: start from a clean slate. */
export function resetAppStore() {
  state = { timings: {}, warm: {}, results: {} }
  persist()
  for (const listener of listeners) listener()
}
