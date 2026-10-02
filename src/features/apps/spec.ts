import type { ColabExecuteEvent, CreateSessionRequest } from '@/types/colab'
import type { Notebook, NotebookParam, NotebookParamValues } from '@/types/notebook'
import {
  APP_EVENT_MIME,
  APP_FORMAT,
  type AppEvent,
  type AppInput,
  type AppSpec,
  type AppWidget,
} from '@/types/app'

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value)

const isSeconds = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value) && value >= 0

/**
 * The notebook's app spec, or null when it is a plain notebook (or the spec
 * is not one this version can render — it then still runs as a notebook).
 */
export function appSpecOf(notebook: Pick<Notebook, 'app'>): AppSpec | null {
  const app = notebook.app
  if (!isObject(app) || app.format !== APP_FORMAT) return null
  const runtime = app.runtime
  const estimates = app.estimates
  if (!isObject(runtime) || typeof runtime.accelerator !== 'string') return null
  if (!isObject(estimates) || !isSeconds(estimates.setup) || !isSeconds(estimates.run)) return null
  if (!Array.isArray(app.outputs) || app.outputs.length === 0) return null
  return app as unknown as AppSpec
}

export interface FormField {
  param: NotebookParam
  input: AppInput
}

export interface FormSection {
  /** null for the unnamed first section. */
  title: string | null
  collapsed: boolean
  fields: FormField[]
}

const DEFAULT_WIDGET: Record<NotebookParam['type'], AppWidget> = {
  string: 'input',
  text: 'textarea',
  integer: 'number',
  number: 'number',
  boolean: 'switch',
  select: 'select',
}

/** Which widgets can edit which parameter types (mirrors build_index.py). */
const WIDGET_TYPES: Record<AppWidget, NotebookParam['type'][]> = {
  input: ['string'],
  textarea: ['string', 'text'],
  file: ['string'],
  select: ['select'],
  segmented: ['select'],
  radio: ['select'],
  number: ['integer', 'number'],
  slider: ['integer', 'number'],
  switch: ['boolean'],
  checkbox: ['boolean'],
}

/**
 * The form, in display order: the spec's inputs first, then any parameter it
 * left out with a default widget, grouped by `section`. A section named
 * Advanced starts collapsed.
 */
export function formSections(spec: AppSpec | null, params: NotebookParam[]): FormSection[] {
  const byKey = new Map(params.map((param) => [param.key, param]))
  const fields: FormField[] = []
  const seen = new Set<string>()
  for (const input of spec?.inputs ?? []) {
    const param = byKey.get(input.param)
    if (!param || seen.has(param.key)) continue
    const widget = WIDGET_TYPES[input.widget]?.includes(param.type)
      ? input.widget
      : DEFAULT_WIDGET[param.type]
    fields.push({ param, input: { ...input, widget } })
    seen.add(param.key)
  }
  for (const param of params) {
    if (!seen.has(param.key))
      fields.push({ param, input: { param: param.key, widget: DEFAULT_WIDGET[param.type] } })
  }

  const sections: FormSection[] = []
  for (const field of fields) {
    const title = field.input.section?.trim() || null
    let section = sections.find((candidate) => candidate.title === title)
    if (!section) {
      section = { title, collapsed: title?.toLowerCase() === 'advanced', fields: [] }
      sections.push(section)
    }
    section.fields.push(field)
  }
  // The unnamed section always comes first.
  return sections.sort((a, b) => Number(a.title !== null) - Number(b.title !== null))
}

export function fieldLabel(field: FormField): string {
  return field.input.label ?? field.param.label
}

/** A select's options, narrowed by its `filter` against the current values. */
export function visibleOptions(field: FormField, values: NotebookParamValues): string[] {
  const options = field.param.options ?? []
  const filter = field.input.filter
  if (!filter) return options
  const parent = values[filter.param]
  if (parent === undefined || parent === '') return options
  const narrowed = options.filter((option) => option.startsWith(String(parent)))
  return narrowed.length ? narrowed : options
}

export function optionLabel(field: FormField, option: string): string {
  return field.input.labels?.[option] ?? option
}

/** Defaults for every parameter, made consistent with any filters. */
export function initialValues(
  sections: FormSection[],
  overrides: NotebookParamValues = {},
): NotebookParamValues {
  const values: NotebookParamValues = {}
  for (const section of sections) {
    for (const { param } of section.fields) {
      const value = overrides[param.key] ?? param.default
      if (value !== null && value !== undefined) values[param.key] = value
      else if (param.type === 'boolean') values[param.key] = false
    }
  }
  return reconcile(sections, values)
}

/**
 * After a change, move any filtered select whose value no longer matches its
 * parent onto the first option that does (picking UK English swaps the
 * voice to a UK voice).
 */
export function reconcile(
  sections: FormSection[],
  values: NotebookParamValues,
): NotebookParamValues {
  const next = { ...values }
  for (const section of sections) {
    for (const field of section.fields) {
      if (field.param.type !== 'select' || !field.input.filter) continue
      const options = visibleOptions(field, next)
      const current = next[field.param.key]
      if (options.length && !options.includes(String(current))) next[field.param.key] = options[0]
    }
  }
  return next
}

// ---------------------------------------------------------------- runtimes

const TPUS = new Set(['V5E1', 'V6E1'])

/** The `session_create` request for an accelerator name from a spec. */
export function runtimeRequest(
  accelerator: string,
  name: string,
  highMem = false,
): CreateSessionRequest {
  const upper = accelerator.toUpperCase()
  if (upper === 'CPU') return { name, highMem }
  if (TPUS.has(upper)) return { name, tpu: upper, highMem }
  return { name, gpu: upper, highMem }
}

export function supportedAccelerators(spec: AppSpec): string[] {
  return spec.runtime.supported?.length ? spec.runtime.supported : [spec.runtime.accelerator]
}

export function runsOn(spec: AppSpec, accelerator: string): boolean {
  return supportedAccelerators(spec).some(
    (candidate) => candidate.toUpperCase() === accelerator.toUpperCase(),
  )
}

/** A runtime name for an app: `app-<slug>`, within the engine's 48 characters. */
export function runtimeNameFor(slug: string, taken: string[]): string {
  const base = `app-${slug}`.slice(0, 44)
  if (!taken.includes(base)) return base
  for (let n = 2; ; n += 1) {
    const candidate = `${base}-${n}`
    if (!taken.includes(candidate)) return candidate
  }
}

// ---------------------------------------------------------------- events

/** The app event inside a kernel output, if it carries one. */
export function appEventOf(event: ColabExecuteEvent): AppEvent | null {
  if (event.type !== 'display' && event.type !== 'update_display' && event.type !== 'result')
    return null
  const raw = (event.data as Record<string, unknown> | undefined)?.[APP_EVENT_MIME]
  let payload: unknown = raw
  if (typeof raw === 'string') {
    try {
      payload = JSON.parse(raw)
    } catch {
      return null
    }
  }
  if (!isObject(payload) || typeof payload.event !== 'string') return null
  switch (payload.event) {
    case 'stage':
      return typeof payload.id === 'string' && typeof payload.label === 'string'
        ? (payload as unknown as AppEvent)
        : null
    case 'output':
      return typeof payload.id === 'string' && typeof payload.kind === 'string'
        ? (payload as unknown as AppEvent)
        : null
    case 'ready':
    case 'done':
      return payload as unknown as AppEvent
    default:
      return null
  }
}

/** Runtime paths are absolute; the contents API wants them without the slash. */
export function contentsPath(path: string): string {
  return path.replace(/^\/+/, '')
}

export function inputUploadPath(slug: string, filename: string): string {
  const safe = filename.replace(/[^A-Za-z0-9._-]+/g, '_').replace(/^\.+/, '') || 'upload'
  return `content/nzap/inputs/${slug}/${Date.now()}-${safe}`
}

// ---------------------------------------------------------------- display

export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '—'
  if (seconds < 1) return '<1s'
  if (seconds < 60) return `${Math.round(seconds)}s`
  const minutes = Math.floor(seconds / 60)
  const rest = Math.round(seconds % 60)
  if (minutes < 60) return rest ? `${minutes}m ${rest}s` : `${minutes}m`
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`
}

/** "~1m" style, rounded the way people say it. */
export function aboutDuration(seconds: number): string {
  if (seconds < 10) return `~${Math.max(1, Math.round(seconds))}s`
  if (seconds < 60) return `~${Math.round(seconds / 5) * 5}s`
  return `~${formatDuration(Math.round(seconds / 15) * 15)}`
}
