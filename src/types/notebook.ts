/** Notebook library: the public collection (GitHub) plus your own notebooks. */

export const NOTEBOOK_PARAM_TYPES = [
  'string',
  'text',
  'integer',
  'number',
  'boolean',
  'select',
] as const

export type NotebookParamType = (typeof NOTEBOOK_PARAM_TYPES)[number]

export interface NotebookParam {
  key: string
  label: string
  type: NotebookParamType
  default?: string | number | boolean | null
  required?: boolean
  /** Only for `select`. */
  options?: string[]
  description?: string
}

export interface Notebook {
  /** `public:<slug>` or `local:<id>`. */
  id: string
  slug: string
  title: string
  description: string
  /** Absent from list responses until a single notebook is fetched. */
  source?: string
  params: NotebookParam[]
  visibility: 'public' | 'private'
  isMine: boolean
  tags: string[]
  author: string | null
  createdAt: string | null
  updatedAt: string | null
  forkedFrom: string | null
  /** The app spec (`nzap-app/1`) when the notebook is an app; see `appSpecOf`. */
  app?: unknown
}

/** Values collected from the run form; keys are the declared parameter keys. */
export type NotebookParamValues = Record<string, string | number | boolean>

export interface CatalogStatus {
  origin: 'remote' | 'cache' | 'bundled'
  url: string
  fetchedAt: string | null
  error: string | null
  count: number
}
