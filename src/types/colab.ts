/**
 * Shapes of the engine's IPC payloads (the Rust structs in `nzap-core` are
 * serialised camelCase; streamed runtime events keep Jupyter's snake_case).
 */

export type ColabConnectionReason = 'not_connected' | 'revoked' | 'token_expired'
export type SecretStorage = 'keychain' | 'file' | 'memory'

export interface GoogleUser {
  sub: string
  email: string
  name?: string | null
  picture?: string | null
}

/** `auth_status` — the Google Auth card's state. */
export interface ColabStatus {
  connected: boolean
  reason: ColabConnectionReason | null
  email: string | null
  user: GoogleUser | null
  warning: string | null
  storage: SecretStorage
  customClient: boolean
}

/** `account_get`: the Google identity plus Colab's own account block. */
export interface ColabAccount {
  user?: GoogleUser | null
  colab?: Record<string, unknown> | null
  error?: string | null
}

export interface ColabSession {
  name: string
  endpoint: string
  /** `CPU`, `T4`, `L4`, `A100`, `H100`, `V5E1`, `V6E1`… */
  accelerator: string
  variant: string
  /** `Standard` | `High-RAM` */
  shape: string
  kernelId: string | null
  sessionId: string | null
  createdAt: number
  lastActivity: number
  lastKeepalive: number | null
  keepaliveError: string | null
  drivePendingUri: string | null
  driveAuthorized: boolean
  connected: boolean
  /** Last kernel `execution_state` (`idle` / `busy` / `starting`) while connected. */
  kernelState: string | null
  /** Opens Colab's web UI attached to this VM (the `colab url` format). */
  colabUrl: string | null
  uptimeSeconds: number
  idleSeconds: number
  lifetimeRemainingSeconds: number
  idleRemainingSeconds: number
}

export interface ColabAssignment {
  endpoint: string
  accelerator: string
  variant: string
  shape: string
  /** True when the engine already tracks the VM as a runtime. */
  managed: boolean
}

export interface ColabConfig {
  gpus: string[]
  tpus: string[]
  highMemOnly: string[]
  keepAliveInterval: number
}

export interface ColabFileEntry {
  name: string
  path: string
  type: 'directory' | 'file' | 'notebook'
  size: number | null
  lastModified: string | null
  mimetype: string | null
}

export interface ColabFileListing {
  path: string
  type: string
  entries: ColabFileEntry[]
}

/** A Jupyter contents model (`files_read`). */
export interface ColabFileModel {
  name: string
  path: string
  type?: string
  format?: string | null
  content?: unknown
  size?: number | null
  mimetype?: string | null
}

export interface ColabMeter {
  usage: number | null
  limit: number | null
  percent: number | null
  name?: string
}

/** Telemetry from `{proxy}/api/colab/resources`, normalised by the engine. */
export interface ColabResources {
  ram: ColabMeter | null
  disk: ColabMeter | null
  gpu: ColabMeter | null
}

export interface CreateSessionRequest {
  name?: string
  gpu?: string
  tpu?: string
  highMem?: boolean
}

/** One event streamed while a cell runs. */
export type ColabExecuteEvent =
  | { type: 'stream'; name?: string; text?: string }
  | { type: 'result'; data?: Record<string, string | string[]>; execution_count?: number }
  | { type: 'display' | 'update_display'; data?: Record<string, string | string[]> }
  | { type: 'error'; ename?: string; evalue?: string; traceback?: string[] }
  | { type: 'status'; state?: string }
  | {
      type: 'colab_request' | 'drive_auth_required'
      message?: string
      uri?: string
      /** `dfs_ephemeral` (Drive) or `auth_user_ephemeral` (Google Cloud). */
      auth_type?: string
    }
  | { type: 'automation'; op?: string; state: 'started' | 'finished'; status?: string }
  | { type: 'input'; execution_count?: number }
  | { type: 'input_request'; prompt?: string; password?: boolean }
  | { type: 'clear_output' }
  | { type: 'execute_reply'; status?: string; execution_count?: number }

/** Compute-unit consumption (VS Code `ConsumptionStatusBar` parity). */
export interface ColabQuota {
  source: 'user-info' | 'ccu-info'
  fetchedAt: number
  tier: 'NONE' | 'PRO' | 'PRO_PLUS'
  paidComputeUnits: number
  consumptionRateHourly: number
  assignmentsCount: number
  freeCcuRemaining: number | null
  freeMinutesRemaining: number | null
  paidMinutesRemaining: number | null
  /** Paid + free minutes at the current burn rate (null while nothing burns). */
  minutesRemaining: number | null
  nextFreeRefillAt: number | null
  severity: 'ok' | 'low' | 'depleted'
  signupAction: string
  eligibleAccelerators: string[]
  ineligibleAccelerators: string[]
  /** `X.XX/hr`, the status-bar text. */
  statusText: string
  /** The extension's tooltip text, verbatim. */
  tooltip: string
  warnBelowMinutes: number
  snoozeMinutes: number
  errors: string[]
}

/** One entry of a runtime's history log (`colab log` event vocabulary). */
export interface ColabHistoryEvent {
  timestamp: string
  event_type: string
  code?: string
  op?: string
  path?: string
  status?: string
  value?: unknown
  endpoint?: string
  accelerator?: string
  reason?: string
  outputs?: unknown[]
  [key: string]: unknown
}

export type ColabHistoryFormat = 'ipynb' | 'md' | 'txt' | 'jsonl'

export type ColabAutomationOp = 'install' | 'drivemount' | 'gcp-auth'

export interface ColabAutomationRequest {
  packages?: string[]
  requirements?: { filename: string; content: string }
  path?: string
}

/** A notebook or script fetched by `import_notebook_url`. */
export interface ColabImportedFile {
  filename: string
  kind: 'ipynb' | 'py'
  content: string
  size: number
}

export interface ColabRunFileRequest {
  filename: string
  content: string
  env?: string[]
  stopOnError?: boolean
}

/** Extra events a file run adds around the ordinary cell events. */
export type ColabRunFileEvent =
  | ColabExecuteEvent
  | { type: 'cell'; index: number; total: number; state: 'started' | 'finished'; status?: string }
  | {
      type: 'run_complete'
      status: 'ok' | 'error'
      failed_cells: number
      total_cells: number
      filename?: string
      notebook?: unknown
    }

/** `job_run` — the `colab run` options. */
export interface ColabJobRequest {
  filename: string
  script: string
  args?: string[]
  env?: string[]
  artifacts?: string[]
  gpu?: string
  tpu?: string
  highMem?: boolean
  keep?: boolean
  name?: string
}

export type ColabJobEvent =
  | ColabExecuteEvent
  | {
      type: 'job'
      phase: 'assigning' | 'connecting' | 'running' | 'collecting' | 'released'
      session: string
      hardware?: string
      endpoint?: string
    }
  | { type: 'artifact'; path: string; size: number; savedTo?: string; skipped?: string }
  | {
      type: 'job_done'
      exit_code: number
      released: boolean
      kept?: boolean
      session: string
      error?: string
    }

/** `session_stop`. */
export interface StopOutcome {
  released: boolean
  warning: string | null
}
