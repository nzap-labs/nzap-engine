/**
 * NZAP apps: a notebook plus an `app.json` spec (`nzap-app/1`), described in
 * the catalog's APPS.md. The engine passes the spec through untouched; this
 * file is its typed shape on the UI side.
 */

export const APP_FORMAT = 'nzap-app/1'
export const APP_EVENT_MIME = 'application/vnd.nzap.app+json'

export type AppCategory = 'audio' | 'image' | 'video' | 'text' | 'vision' | 'data' | 'utility'

export type AppWidget =
  | 'input'
  | 'textarea'
  | 'file'
  | 'select'
  | 'segmented'
  | 'radio'
  | 'number'
  | 'slider'
  | 'switch'
  | 'checkbox'

export interface AppInput {
  param: string
  widget: AppWidget
  label?: string
  placeholder?: string
  rows?: number
  maxLength?: number
  min?: number
  max?: number
  step?: number
  unit?: string
  /** Option value → display text. */
  labels?: Record<string, string>
  /** Only show options that start with another parameter's current value. */
  filter?: { param: string; prefix: true }
  accept?: string
  maxMb?: number
  section?: string
}

export type AppOutputKind =
  'audio' | 'image' | 'video' | 'text' | 'markdown' | 'json' | 'table' | 'file'

export interface AppOutputSlot {
  id: string
  kind: AppOutputKind
  label?: string
}

export interface AppSpec {
  format: typeof APP_FORMAT
  category: AppCategory
  icon?: string
  tagline?: string
  runtime: {
    accelerator: string
    supported?: string[]
    highMem?: boolean
    minVramGb?: number
  }
  estimates: {
    setup: number
    run: number
    measuredOn?: string
    runNote?: string
  }
  runLabel?: string
  inputs?: AppInput[]
  outputs: AppOutputSlot[]
  examples?: { label: string; values: Record<string, string | number | boolean> }[]
  links?: { label: string; url: string }[]
  license?: string
}

/** Events a running app reports through `application/vnd.nzap.app+json`. */
export type AppEvent =
  | { event: 'stage'; id: string; label: string; progress?: number | null }
  | { event: 'ready'; warm: boolean; setupSeconds?: number; device?: string }
  | ({ event: 'output' } & AppOutputPayload)
  | { event: 'done'; warm?: boolean; seconds?: { setup?: number; run?: number } }

export interface AppOutputPayload {
  id: string
  kind: AppOutputKind
  /** Media on the runtime (absolute, e.g. `/content/nzap/outputs/…`). */
  path?: string
  mime?: string
  filename?: string
  text?: string
  value?: unknown
  columns?: string[]
  rows?: (string | number | boolean | null)[][]
  meta?: Record<string, unknown>
}
