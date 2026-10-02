/**
 * A simulated NZAP engine behind Tauri's IPC mock.
 *
 * `npm run dev` in a plain browser and the Playwright web E2E suite run the
 * real UI against this instead of Rust. It answers every command the app
 * uses with the same payload shapes as `src-tauri`, streams events through
 * channels, and imitates a Colab kernel closely enough to exercise every
 * flow: prints, errors, `input()` prompts, Drive consent pause/resume,
 * interrupts, a terminal, files, notebooks, file runs and jobs.
 *
 * Tests drive it through `window.__NZAP_FAKE__` (see `FakeControls`).
 * Production builds never include it (dynamic import behind DEV /
 * VITE_FAKE_ENGINE).
 */
import { mockIPC } from '@tauri-apps/api/mocks'
import type { Channel } from '@tauri-apps/api/core'
import bundledCatalog from '../../crates/nzap-core/catalog/bundled.json'

type Json = Record<string, unknown>
type Emit = (event: Json) => void

interface FakeUser {
  sub: string
  email: string
  name: string
  picture: string | null
}

interface FakeSession {
  name: string
  endpoint: string
  accelerator: string
  variant: string
  shape: string
  kernelId: string | null
  createdAt: number
  lastActivity: number
  lastKeepalive: number | null
  driveAuthorized: boolean
  drivePendingUri: string | null
  connected: boolean
  kernelState: string | null
  count: number
  files: Map<string, FakeFile>
  /** App slugs whose model is loaded in this kernel. */
  warmApps: Set<string>
}

interface FakeFile {
  dir: boolean
  content: string
  /** Binary files keep base64 content, like Jupyter's contents API. */
  format?: 'base64'
}

interface FakeNotebook {
  id: string
  slug: string
  title: string
  description: string
  source: string
  params: Json[]
  tags: string[]
  author: string | null
  visibility: 'public' | 'private'
  createdAt: string | null
  updatedAt: string | null
  forkedFrom: string | null
  app?: Json
}

export interface FakeState {
  /** Milliseconds between streamed events. */
  delay: number
  connected: boolean
  /** Whether the user already granted Drive / Cloud consent. */
  driveConsent: boolean
  /** Fail the next `session_create` with this error. */
  failNextCreate: { code: string; message: string } | null
  user: FakeUser
  sessions: Map<string, FakeSession>
  external: { endpoint: string; accelerator: string; shape: string }[]
  history: Map<string, Json[]>
  notebooks: FakeNotebook[]
  settings: Json
  customClient: string | null
  /** URLs the app asked to open in the browser. */
  opened: string[]
  /** Commands invoked, in order (for assertions). */
  calls: string[]
  saved: { filename: string; content: string }[]
  /** What the update feed offers (`plugin:updater|check`); null = up to date. */
  update: { version: string; body: string; date: string } | null
  /** Set when the app asked to relaunch after installing an update. */
  restarted: boolean
}

export interface FakeControls {
  state: FakeState
  reset: (overrides?: Partial<Omit<FakeState, 'sessions' | 'history'>>) => void
}

declare global {
  interface Window {
    __NZAP_FAKE__?: FakeControls
  }
}

const SIGNED_IN: FakeUser = {
  sub: '1',
  email: 'ada@example.com',
  name: 'Ada Lovelace',
  picture: null,
}

const DEFAULT_CATALOG = 'https://raw.githubusercontent.com/nzap-labs/nzap-notebooks/main/'

function publicNotebooks(): FakeNotebook[] {
  return (bundledCatalog as { notebooks: Json[] }).notebooks.map((entry) => ({
    id: `public:${String(entry.slug)}`,
    slug: String(entry.slug),
    title: String(entry.title),
    description: String(entry.description ?? ''),
    source: String(entry.sourceText ?? ''),
    params: (entry.params as Json[]) ?? [],
    tags: (entry.tags as string[]) ?? [],
    author: (entry.author as string | undefined) ?? null,
    visibility: 'public',
    createdAt: null,
    updatedAt: null,
    forkedFrom: null,
    ...(entry.app ? { app: entry.app as Json } : {}),
  }))
}

function initialState(): FakeState {
  return {
    delay: 120,
    connected: false,
    driveConsent: true,
    failNextCreate: null,
    user: SIGNED_IN,
    sessions: new Map(),
    external: [{ endpoint: 'm-s-t4-webui', accelerator: 'T4', shape: 'Standard' }],
    history: new Map(),
    notebooks: publicNotebooks(),
    settings: {
      catalogUrl: DEFAULT_CATALOG,
      keepAlive: true,
      keepAliveIntervalSeconds: 60,
      closeToTray: false,
      artifactsDir: null,
    },
    customClient: null,
    opened: [],
    calls: [],
    saved: [],
    update: null,
    restarted: false,
  }
}

class Failure {
  constructor(
    readonly code: string,
    readonly message: string,
  ) {}
}

function fail(code: string, message: string): never {
  throw { code, message }
}

const now = () => Date.now() / 1000

/** Test presets set before the page loads (`sessionStorage['nzap-fake-preset']`). */
function preset(): Partial<FakeState> {
  try {
    const raw = sessionStorage.getItem('nzap-fake-preset')
    return raw ? (JSON.parse(raw) as Partial<FakeState>) : {}
  } catch {
    return {}
  }
}

export function installFakeEngine(): FakeControls {
  const controls: FakeControls = {
    state: { ...initialState(), ...preset() },
    reset(overrides = {}) {
      controls.state = { ...initialState(), ...overrides }
    },
  }
  window.__NZAP_FAKE__ = controls
  const s = () => controls.state

  // ------------------------------------------------------------ plumbing

  const cancels = new Map<string, () => void>()
  const stdinWaiters = new Map<string, (value: string) => void>()
  const consentWaiters = new Map<string, () => void>()
  const interruptWaiters = new Map<string, () => void>()
  let terminals = 0
  const terminalLines = new Map<number, { channel: Channel<Json>; line: string }>()

  const sleep = (ms = s().delay) => new Promise((resolve) => setTimeout(resolve, ms))

  /** Race `promise` against the stream being cancelled. */
  function cancellable<T>(streamId: string | undefined, promise: Promise<T>): Promise<T> {
    if (!streamId) return promise
    return Promise.race([
      promise,
      new Promise<T>((_, reject) =>
        cancels.set(streamId, () => reject(new Failure('cancelled', 'Cancelled.'))),
      ),
    ])
  }

  function emitter(channel: unknown): Emit {
    const target = channel as Channel<Json>
    return (event) => target.onmessage(event)
  }

  function log(name: string, eventType: string, data: Json = {}) {
    const events = s().history.get(name) ?? []
    events.push({ timestamp: new Date().toISOString(), event_type: eventType, ...data })
    s().history.set(name, events)
  }

  function requireConnected() {
    if (!s().connected) fail('not_connected', 'Connect your Google account first.')
  }

  function session(name: unknown): FakeSession {
    const found = s().sessions.get(String(name))
    if (!found) fail('not_found', `No such runtime: ${String(name)}`)
    return found
  }

  function view(session: FakeSession): Json {
    const uptime = Math.max(0, now() - session.createdAt)
    const idle = Math.max(0, now() - session.lastActivity)
    return {
      name: session.name,
      endpoint: session.endpoint,
      accelerator: session.accelerator,
      variant: session.variant,
      shape: session.shape,
      kernelId: session.kernelId,
      sessionId: session.kernelId ? `session-${session.name}` : null,
      createdAt: session.createdAt,
      lastActivity: session.lastActivity,
      lastKeepalive: session.lastKeepalive,
      keepaliveError: null,
      drivePendingUri: session.drivePendingUri,
      driveAuthorized: session.driveAuthorized,
      connected: session.connected,
      kernelState: session.connected ? session.kernelState : null,
      colabUrl: `https://colab.research.google.com/notebooks/empty.ipynb?dbu=%2Ftun%2Fm%2F${session.endpoint}`,
      uptimeSeconds: Math.floor(uptime),
      idleSeconds: Math.floor(idle),
      lifetimeRemainingSeconds: Math.max(0, Math.floor(12 * 3600 - uptime)),
      idleRemainingSeconds: Math.max(0, Math.floor(90 * 60 - idle)),
    }
  }

  function newSession(name: string, gpu?: string, tpu?: string, highMem?: boolean): FakeSession {
    const accelerator = tpu ? tpu.toUpperCase() : gpu ? gpu.toUpperCase() : 'CPU'
    const files = new Map<string, FakeFile>([
      ['content', { dir: true, content: '' }],
      ['content/sample_data', { dir: true, content: '' }],
      ['content/sample_data/README.md', { dir: false, content: 'Sample datasets.\n' }],
    ])
    return {
      name,
      endpoint: `m-s-${accelerator.toLowerCase()}-${Math.random().toString(36).slice(2, 8)}`,
      accelerator,
      variant: tpu ? '2' : gpu ? '1' : '0',
      shape: highMem ? 'High-RAM' : 'Standard',
      kernelId: null,
      createdAt: now(),
      lastActivity: now(),
      lastKeepalive: null,
      driveAuthorized: false,
      drivePendingUri: null,
      connected: false,
      kernelState: null,
      count: 0,
      files,
      warmApps: new Set(),
    }
  }

  // ------------------------------------------------------------ the kernel

  function printed(line: string, params: Json): string | null {
    const match = /^\s*print\((.*)\)\s*$/.exec(line)
    if (!match) return null
    const inner = match[1].trim()
    const quoted = /^(['"])(.*)\1$/.exec(inner)
    if (quoted) return quoted[2]
    const lookup = /^params\[['"](.+)['"]\]$/.exec(inner)
    if (lookup) {
      const value = params[lookup[1]]
      return typeof value === 'string' ? value : JSON.stringify(value)
    }
    return inner
  }

  /** Run one cell on `target`, emitting UI events; returns the execute_reply. */
  async function runCell(
    target: FakeSession,
    code: string,
    emit: Emit,
    streamId?: string,
    record = true,
  ): Promise<Json> {
    target.connected = true
    target.kernelId ??= `kernel-${target.name}`
    target.lastActivity = now()
    target.count += 1
    const count = target.count
    const seen: Json[] = []
    const out = (event: Json) => {
      seen.push(event)
      emit(event)
    }
    let status = 'ok'
    target.kernelState = 'busy'
    out({ type: 'status', state: 'busy' })
    out({ type: 'input', execution_count: count })
    await cancellable(streamId, sleep())

    try {
      if (code.includes('drive.mount(') || code.includes('authenticate_user(')) {
        const drive = code.includes('drive.mount(')
        const what = drive ? 'Google Drive' : 'Google Cloud'
        const authType = drive ? 'dfs_ephemeral' : 'auth_user_ephemeral'
        emit({
          type: 'colab_request',
          auth_type: authType,
          message: `${what} authorization requested by the VM…`,
        })
        if (!s().driveConsent) {
          const uri = 'https://accounts.google.com/o/oauth2/consent?fake=1'
          target.drivePendingUri = uri
          emit({
            type: 'drive_auth_required',
            auth_type: authType,
            uri,
            message: `${what} access has not been granted yet. Open the link, approve access, then click Continue — the cell resumes where it paused.`,
          })
          await cancellable(
            streamId,
            new Promise<void>((resolve) => consentWaiters.set(target.name, resolve)),
          )
        }
        target.drivePendingUri = null
        if (drive) target.driveAuthorized = true
        emit({
          type: 'colab_request',
          auth_type: authType,
          message: `${what} credentials propagated. Resuming…`,
        })
        out({
          type: 'stream',
          name: 'stdout',
          text: drive ? 'Mounted at /content/drive\n' : 'Authenticated with Google Cloud.\n',
        })
      } else if (code.includes('input(')) {
        out({ type: 'input_request', prompt: 'Name? ', password: false })
        const value = await cancellable(
          streamId,
          new Promise<string>((resolve) => stdinWaiters.set(target.name, resolve)),
        )
        out({ type: 'stream', name: 'stdout', text: `Hello, ${value}!\n` })
      } else if (code.includes('wait_for_interrupt') || code.includes('time.sleep')) {
        await cancellable(
          streamId,
          new Promise<void>((resolve) => interruptWaiters.set(target.name, resolve)),
        )
        out({
          type: 'error',
          ename: 'KeyboardInterrupt',
          evalue: '',
          traceback: ['KeyboardInterrupt'],
        })
        status = 'error'
      } else {
        let params: Json = {}
        for (const line of code.split('\n')) {
          const injected = /^params = _nzap_json\.loads\((.*)\)$/.exec(line.trim())
          if (injected) {
            params = JSON.parse(JSON.parse(injected[1]) as string) as Json
            continue
          }
          const text = printed(line, params)
          if (text !== null) {
            out({ type: 'stream', name: 'stdout', text: `${text}\n` })
            await sleep(s().delay / 3)
          }
        }
        if (code.includes("uv', 'pip', 'install'"))
          out({ type: 'stream', name: 'stdout', text: 'Installation Complete (via uv)!\n' })
        if (code.includes('display_image'))
          out({ type: 'display', data: { 'image/png': SAMPLE_PNG, 'text/plain': '<Figure>' } })
        if (code.includes('answer'))
          out({ type: 'result', execution_count: count, data: { 'text/plain': '42' } })
        const exit = /sys\.exit\((\d+)\)/.exec(code)
        if (exit) {
          out({
            type: 'error',
            ename: 'SystemExit',
            evalue: exit[1],
            traceback: [`SystemExit: ${exit[1]}`],
          })
          status = 'error'
        } else if (code.includes('raise') || code.includes('fail')) {
          out({
            type: 'error',
            ename: 'ValueError',
            evalue: 'boom',
            traceback: ['Traceback (most recent call last):', 'ValueError: boom'],
          })
          status = 'error'
        }
      }
    } catch (error) {
      if (record) log(target.name, 'execution', { code, status: 'interrupted', outputs: seen })
      throw error
    } finally {
      stdinWaiters.delete(target.name)
      consentWaiters.delete(target.name)
      interruptWaiters.delete(target.name)
      target.kernelState = 'idle'
    }
    const reply = { type: 'execute_reply', status, execution_count: count }
    out({ type: 'status', state: 'idle' })
    out(reply)
    if (record)
      log(target.name, 'execution', {
        code,
        status,
        execution_count: count,
        outputs: seen.filter((e) =>
          ['stream', 'result', 'display', 'error'].includes(String(e.type)),
        ),
      })
    return reply
  }

  // ------------------------------------------------------------ notebooks

  function notebookView(notebook: FakeNotebook, withSource: boolean): Json {
    return {
      id: notebook.id,
      slug: notebook.slug,
      title: notebook.title,
      description: notebook.description,
      ...(withSource ? { source: notebook.source } : {}),
      params: notebook.params,
      visibility: notebook.visibility,
      isMine: notebook.visibility === 'private',
      tags: notebook.tags,
      author: notebook.author,
      createdAt: notebook.createdAt,
      updatedAt: notebook.updatedAt,
      forkedFrom: notebook.forkedFrom,
      ...(notebook.app ? { app: notebook.app } : {}),
    }
  }

  function findNotebook(id: unknown): FakeNotebook {
    const notebook = s().notebooks.find((entry) => entry.id === id)
    if (!notebook) fail('not_found', 'Notebook not found.')
    return notebook
  }

  function saveNotebook(draft: Json, existing?: FakeNotebook): FakeNotebook {
    const slug = String(draft.slug ?? existing?.slug ?? '').trim()
    if (!/^[a-z0-9][a-z0-9-]{1,62}$/.test(slug))
      fail('invalid_input', 'Slug must be lowercase letters, numbers and dashes.')
    if (
      s().notebooks.some(
        (entry) => entry.visibility === 'private' && entry.slug === slug && entry !== existing,
      )
    ) {
      fail('invalid_input', `You already have a notebook called '${slug}'.`)
    }
    const title = String(draft.title ?? existing?.title ?? '').trim()
    if (!title) fail('invalid_input', 'A title is required.')
    const source = String(draft.source ?? existing?.source ?? '')
    if (!source.trim()) fail('invalid_input', 'Notebook source is required.')
    const stamp = new Date().toISOString()
    const notebook: FakeNotebook = {
      id: existing?.id ?? `local:${crypto.randomUUID().replaceAll('-', '')}`,
      slug,
      title,
      description: String(draft.description ?? existing?.description ?? ''),
      source,
      params: (draft.params as Json[] | undefined) ?? existing?.params ?? [],
      tags: [],
      author: null,
      visibility: 'private',
      createdAt: existing?.createdAt ?? stamp,
      updatedAt: stamp,
      forkedFrom: (draft.forkedFrom as string | null | undefined) ?? existing?.forkedFrom ?? null,
      app: (draft.app as Json | undefined) ?? existing?.app,
    }
    if (existing) Object.assign(existing, notebook)
    else s().notebooks.push(notebook)
    return existing ?? notebook
  }

  function resolveParams(declared: Json[], values: Json): Json {
    const resolved: Json = {}
    for (const param of declared) {
      const raw = values[String(param.key)]
      const value = raw === undefined || raw === null || raw === '' ? param.default : raw
      if (value === undefined || value === null) {
        if (param.required) fail('invalid_input', `${String(param.label)} is required.`)
        continue
      }
      resolved[String(param.key)] =
        param.type === 'integer' || param.type === 'number' ? Number(value) : value
    }
    return resolved
  }

  // ------------------------------------------------------------ commands

  async function handle(
    cmd: string,
    args: Json,
    options?: { headers?: Record<string, string> },
  ): Promise<unknown> {
    s().calls.push(cmd)
    const streamId = args.streamId as string | undefined
    try {
      return await dispatch(cmd, args, options, streamId)
    } catch (error) {
      if (error instanceof Failure) throw { code: error.code, message: error.message }
      throw error
    } finally {
      if (streamId) cancels.delete(streamId)
    }
  }

  async function dispatch(
    cmd: string,
    args: Json,
    options: { headers?: Record<string, string> } | undefined,
    streamId: string | undefined,
  ): Promise<unknown> {
    switch (cmd) {
      // -- app
      case 'app_info':
        return { version: '0.1.0-preview', os: 'browser', arch: 'fake' }
      case 'open_url':
        s().opened.push(String(args.url))
        return null
      case 'reveal_path':
      case 'open_log_dir':
        return null
      // -- updater + process plugins
      case 'plugin:updater|check':
        await sleep()
        return s().update
          ? {
              rid: 1,
              currentVersion: '0.1.0',
              version: s().update!.version,
              date: s().update!.date,
              body: s().update!.body,
              rawJson: {},
            }
          : null
      case 'plugin:updater|download_and_install': {
        const emit = emitter(args.onEvent)
        const total = 12 * 1024 * 1024
        emit({ event: 'Started', data: { contentLength: total } })
        for (let chunk = 0; chunk < 6; chunk += 1) {
          await sleep()
          emit({ event: 'Progress', data: { chunkLength: total / 6 } })
        }
        emit({ event: 'Finished' })
        return null
      }
      case 'plugin:process|restart':
        s().restarted = true
        return null
      case 'plugin:resources|close':
        return null
      case 'stream_cancel': {
        const cancel = cancels.get(String(args.streamId))
        cancel?.()
        return Boolean(cancel)
      }
      case 'config_get':
        return {
          gpus: ['t4', 'l4', 'g4', 'a100', 'h100'],
          tpus: ['v5e1', 'v6e1'],
          highMemOnly: ['l4', 'v5e1', 'v6e1'],
          keepAliveInterval: s().settings.keepAliveIntervalSeconds,
        }
      case 'settings_get':
        return settingsView()
      case 'settings_update': {
        const patch = args.patch as Json
        if (patch.catalogUrl !== undefined && !/^https:\/\//.test(String(patch.catalogUrl))) {
          fail('invalid_input', 'The catalog URL must be an https:// address.')
        }
        Object.assign(s().settings, patch)
        if (patch.artifactsDir === '') s().settings.artifactsDir = null
        return settingsView()
      }
      case 'settings_set_oauth_client': {
        if (s().connected)
          fail('invalid_input', 'Disconnect Google before changing the OAuth client.')
        const json = args.json as string | null
        if (json) {
          const parsed = JSON.parse(json) as Json
          const inner = (parsed.installed ?? parsed.web ?? parsed) as Json
          if (!inner.client_id) fail('invalid_input', 'OAuth client JSON has no client_id.')
          s().customClient = String(inner.client_id)
        } else {
          s().customClient = null
        }
        return settingsView()
      }

      // -- auth
      case 'auth_status':
        return {
          connected: s().connected,
          reason: s().connected ? null : 'not_connected',
          email: s().connected ? s().user.email : null,
          user: s().connected ? s().user : null,
          warning: null,
          storage: 'keychain',
          customClient: Boolean(s().customClient),
        }
      case 'auth_connect':
        s().opened.push('https://accounts.google.com/o/oauth2/v2/auth?fake=1')
        await sleep(s().delay * 4)
        s().connected = true
        return s().user
      case 'auth_cancel':
        return null
      case 'auth_begin_remote':
        s().opened.push('https://accounts.google.com/o/oauth2/v2/auth?token_usage=remote')
        return 'https://accounts.google.com/o/oauth2/v2/auth?token_usage=remote'
      case 'auth_complete_remote':
        if (!String(args.code).trim()) fail('invalid_input', 'Paste the code Google showed you.')
        s().connected = true
        return s().user
      case 'auth_disconnect':
        for (const name of s().sessions.keys())
          log(name, 'session_terminated', { reason: 'user_requested' })
        s().sessions.clear()
        s().connected = false
        return null
      case 'account_get':
        requireConnected()
        return {
          user: s().user,
          colab: { currentBalance: 0, consumptionRateHourly: 0.07 },
          error: null,
        }
      case 'quota_get': {
        requireConnected()
        const burning = [...s().sessions.values()].length * 0.07
        return {
          source: 'ccu-info',
          fetchedAt: now(),
          tier: 'NONE',
          paidComputeUnits: 0,
          consumptionRateHourly: burning,
          assignmentsCount: s().sessions.size,
          freeCcuRemaining: 36,
          freeMinutesRemaining: burning > 0 ? 600 : null,
          paidMinutesRemaining: burning > 0 ? 0 : null,
          minutesRemaining: burning > 0 ? 600 : null,
          nextFreeRefillAt: now() + 86_400,
          severity: 'ok',
          signupAction: 'Sign Up for Colab',
          eligibleAccelerators: ['T4', 'V5E1'],
          ineligibleAccelerators: ['A100', 'L4', 'H100', 'G4'],
          statusText: `${burning.toFixed(2)}/hr`,
          tooltip:
            'You are not subscribed.\n\nYou currently have zero compute units available. Resources offered free of charge are not guaranteed.',
          warnBelowMinutes: 30,
          snoozeMinutes: 10,
          errors: [],
        }
      }

      // -- runtimes
      case 'sessions_list':
        return [...s().sessions.values()].map(view)
      case 'session_create': {
        requireConnected()
        const request = args.request as Json
        if (s().failNextCreate) {
          const failure = s().failNextCreate!
          s().failNextCreate = null
          fail(failure.code, failure.message)
        }
        const name = String(request.name || `session-${Math.floor(now())}`)
        if (!/^[A-Za-z0-9._-]{1,48}$/.test(name))
          fail(
            'invalid_input',
            'Runtime names use 1–48 letters, digits, dots, dashes or underscores.',
          )
        if (s().sessions.has(name))
          fail('invalid_input', `A runtime named '${name}' already exists.`)
        await sleep(s().delay * 3)
        const created = newSession(
          name,
          request.gpu as string | undefined,
          request.tpu as string | undefined,
          Boolean(request.highMem),
        )
        created.kernelId = `kernel-${name}`
        created.connected = true
        created.kernelState = 'idle'
        s().sessions.set(name, created)
        log(name, 'session_created', {
          endpoint: created.endpoint,
          accelerator: created.accelerator,
          how: 'assigned',
        })
        return { session: view(created), connected: true }
      }
      case 'session_get':
        return view(session(args.name))
      case 'session_connect': {
        const target = session(args.name)
        target.connected = true
        target.kernelId ??= `kernel-${target.name}`
        target.kernelState = 'idle'
        return { connected: true, kernelId: target.kernelId, kernelInfo: { status: 'ok' } }
      }
      case 'session_disconnect':
        session(args.name).connected = false
        return null
      case 'session_keepalive': {
        const target = session(args.name)
        target.lastKeepalive = now()
        return { ok: true, at: target.lastKeepalive }
      }
      case 'session_restart': {
        s().sessions.get(String(args.name))?.warmApps.clear()
        const target = session(args.name)
        target.connected = false
        target.count = 0
        return { id: target.kernelId, restarted: true }
      }
      case 'session_interrupt': {
        const target = session(args.name)
        interruptWaiters.get(target.name)?.()
        consentWaiters.get(target.name)?.()
        return null
      }
      case 'session_stdin': {
        const target = session(args.name)
        const waiter = stdinWaiters.get(target.name)
        if (!waiter) fail('runtime', 'The kernel is not waiting for input.')
        log(target.name, 'input_reply', { value: args.value })
        waiter(String(args.value))
        return null
      }
      case 'session_drive_authorize': {
        const target = session(args.name)
        if (!s().driveConsent)
          return { success: false, unauthorizedRedirectUri: target.drivePendingUri }
        const waiter = consentWaiters.get(target.name)
        waiter?.()
        return { success: true, resumed: Boolean(waiter) }
      }
      case 'session_stop': {
        const target = session(args.name)
        s().sessions.delete(target.name)
        log(target.name, 'session_terminated', { reason: 'user_requested' })
        return { released: true, warning: null }
      }
      case 'session_resources': {
        const target = session(args.name)
        const gib = 1024 ** 3
        return {
          ram: { usage: 2.1 * gib, limit: 12.7 * gib, percent: 16.5 },
          disk: { usage: 38 * gib, limit: 107 * gib, percent: 35.5 },
          gpu:
            target.accelerator === 'CPU'
              ? null
              : {
                  usage: 0.4 * gib,
                  limit: 15 * gib,
                  percent: 2.7,
                  name: `Tesla ${target.accelerator}`,
                },
        }
      }
      case 'session_execute': {
        const target = session(args.name)
        if (!String(args.code).trim()) fail('invalid_input', 'There is no code to run.')
        return runCell(target, String(args.code), emitter(args.onEvent), streamId)
      }
      case 'session_automation': {
        const target = session(args.name)
        const emit = emitter(args.onEvent)
        const op = String(args.op)
        const request = args.request as Json
        const code =
          op === 'install'
            ? `subprocess.check_call(['uv', 'pip', 'install', '--system'] + ${JSON.stringify(request.packages ?? [])})`
            : op === 'drivemount'
              ? `from google.colab import drive\ndrive.mount(${JSON.stringify(request.path ?? '/content/drive')})`
              : 'from google.colab import auth\nauth.authenticate_user()'
        if (
          op === 'install' &&
          !(request.packages as string[] | undefined)?.length &&
          !request.requirements
        ) {
          fail('invalid_input', 'No packages or requirements specified.')
        }
        log(target.name, 'automation', { op, code })
        emit({ type: 'automation', op, state: 'started' })
        const reply = await runCell(target, code, emit, streamId, false)
        log(target.name, 'automation_result', { op, status: reply.status })
        emit({ type: 'automation', op, state: 'finished', status: reply.status })
        return reply.status
      }
      case 'session_run_file':
        return runFile(session(args.name), args.request as Json, emitter(args.onEvent), streamId)
      case 'job_run':
        return runJob(args.request as Json, emitter(args.onEvent), streamId)
      case 'import_notebook_url': {
        const url = String(args.url)
        if (!/^https:\/\//.test(url)) fail('invalid_input', 'Only https:// links can be imported.')
        await sleep()
        return {
          filename: url.split('/').pop() || 'imported.ipynb',
          kind: url.endsWith('.py') ? 'py' : 'ipynb',
          content: url.endsWith('.py')
            ? "print('imported script')\n"
            : JSON.stringify({
                cells: [{ cell_type: 'code', source: "print('imported notebook')" }],
                nbformat: 4,
              }),
          size: 120,
        }
      }
      case 'assignments_list':
        requireConnected()
        return [
          ...[...s().sessions.values()].map((entry) => ({
            endpoint: entry.endpoint,
            accelerator: entry.accelerator,
            variant: entry.variant,
            shape: entry.shape,
            managed: true,
          })),
          ...s().external.map((entry) => ({ ...entry, variant: '1', managed: false })),
        ]
      case 'assignment_release': {
        s().external = s().external.filter((entry) => entry.endpoint !== args.endpoint)
        for (const [name, entry] of s().sessions)
          if (entry.endpoint === args.endpoint) s().sessions.delete(name)
        return null
      }
      case 'assignment_adopt': {
        const external = s().external.find((entry) => entry.endpoint === args.endpoint)
        if (!external) fail('not_found', 'Assignment not found.')
        const name = String(args.name || `imported-${external.endpoint.slice(0, 8)}`)
        const adopted = newSession(
          name,
          external.accelerator === 'CPU' ? undefined : external.accelerator.toLowerCase(),
        )
        adopted.endpoint = external.endpoint
        s().sessions.set(name, adopted)
        s().external = s().external.filter((entry) => entry !== external)
        log(name, 'session_created', { endpoint: adopted.endpoint, how: 'adopted' })
        return view(adopted)
      }

      // -- terminal
      case 'terminal_open': {
        const target = session(args.name)
        const id = ++terminals
        const channel = args.onFrame as Channel<Json>
        terminalLines.set(id, { channel, line: '' })
        log(target.name, 'console_started')
        setTimeout(
          () =>
            channel.onmessage({
              type: 'frame',
              data: JSON.stringify({ data: 'Welcome to the fake runtime\r\nroot@fake:/content# ' }),
            }),
          10,
        )
        return id
      }
      case 'terminal_send': {
        const terminal = terminalLines.get(Number(args.id))
        if (!terminal) fail('not_found', 'That terminal is closed.')
        const frame = JSON.parse(String(args.frame)) as { data?: string }
        if (typeof frame.data !== 'string') return null
        let output = ''
        for (const char of frame.data) {
          if (char === '\r' || char === '\n') {
            const command = terminal.line.trim()
            output += `\r\n${command === 'whoami' ? 'root\r\n' : command === 'pwd' ? '/content\r\n' : command ? `sh: 1: ${command}: not found\r\n` : ''}root@fake:/content# `
            terminal.line = ''
          } else if (char === '\u007f') {
            if (terminal.line) {
              terminal.line = terminal.line.slice(0, -1)
              output += '\b \b'
            }
          } else {
            terminal.line += char
            output += char
          }
        }
        terminal.channel.onmessage({ type: 'frame', data: JSON.stringify({ data: output }) })
        return null
      }
      case 'terminal_close': {
        const terminal = terminalLines.get(Number(args.id))
        terminal?.channel.onmessage({ type: 'closed', reason: null })
        terminalLines.delete(Number(args.id))
        return null
      }

      // -- history
      case 'history_get': {
        const events = s().history.get(String(args.name)) ?? []
        const limit = Number(args.limit ?? 0)
        return limit ? events.slice(-limit) : events
      }
      case 'history_export':
        s().saved.push({
          filename: `${String(args.name)}.${String(args.format)}`,
          content: JSON.stringify(s().history.get(String(args.name)) ?? []),
        })
        return `/Users/you/Downloads/${String(args.name)}.${String(args.format)}`
      case 'history_clear':
        s().history.delete(String(args.name))
        return null

      // -- files
      case 'files_list': {
        const target = session(args.name)
        const dir = String(args.path ?? '').replace(/^\/+|\/+$/g, '')
        const prefix = dir ? `${dir}/` : ''
        const entries = [...target.files.entries()]
          .filter(
            ([path]) =>
              path.startsWith(prefix) &&
              path.length > prefix.length &&
              !path.slice(prefix.length).includes('/'),
          )
          .map(([path, file]) => ({
            name: path.split('/').pop(),
            path,
            type: file.dir ? 'directory' : 'file',
            size: file.dir ? null : file.content.length,
            lastModified: '2026-01-01T00:00:00Z',
            mimetype: file.dir ? null : 'text/plain',
          }))
          .sort(
            (a, b) =>
              Number(a.type !== 'directory') - Number(b.type !== 'directory') ||
              String(a.name).localeCompare(String(b.name)),
          )
        return { path: dir, type: 'directory', entries }
      }
      case 'files_read': {
        const file = session(args.name).files.get(String(args.path))
        if (!file) fail('runtime', `Not found: ${String(args.path)}`)
        return {
          name: String(args.path).split('/').pop(),
          path: args.path,
          type: 'file',
          format: file.format ?? 'text',
          content: file.content,
          mimetype: file.format === 'base64' ? 'application/octet-stream' : 'text/plain',
        }
      }
      case 'files_write':
        writeFile(session(args.name), String(args.path), String(args.content ?? ''))
        return { path: args.path }
      case 'files_upload_bytes': {
        const name = decodeURIComponent(options?.headers?.['x-nzap-session'] ?? '')
        const path = decodeURIComponent(options?.headers?.['x-nzap-path'] ?? '')
        const bytes = args as unknown as Uint8Array
        writeFile(session(name), path, new TextDecoder().decode(bytes))
        return { path }
      }
      case 'files_mkdir':
        session(args.name).files.set(String(args.path), { dir: true, content: '' })
        return { path: args.path }
      case 'files_rename': {
        const target = session(args.name)
        const from = String(args.path)
        const to = String(args.newPath).replace(/^\/+|\/+$/g, '')
        for (const [path, file] of [...target.files.entries()]) {
          if (path === from || path.startsWith(`${from}/`)) {
            target.files.delete(path)
            target.files.set(to + path.slice(from.length), file)
          }
        }
        return { path: to }
      }
      case 'files_delete': {
        const target = session(args.name)
        const path = String(args.path)
        for (const key of [...target.files.keys()])
          if (key === path || key.startsWith(`${path}/`)) target.files.delete(key)
        return null
      }
      case 'files_download':
        return `/Users/you/Downloads/${String(args.path).split('/').pop()}`
      case 'save_text_file':
        s().saved.push({ filename: String(args.filename), content: String(args.content) })
        return `/Users/you/Downloads/${String(args.filename)}`

      // -- notebooks
      case 'notebooks_list':
        return {
          notebooks: [...s().notebooks]
            .sort(
              (a, b) =>
                Number(a.visibility === 'private') - Number(b.visibility === 'private') ||
                a.title.localeCompare(b.title),
            )
            .map((notebook) => notebookView(notebook, false)),
          catalog: {
            origin: 'bundled',
            url: String(s().settings.catalogUrl),
            fetchedAt: null,
            error: null,
            count: publicNotebooks().length,
          },
        }
      case 'notebooks_refresh':
        await sleep()
        return {
          origin: 'remote',
          url: String(s().settings.catalogUrl),
          fetchedAt: new Date().toISOString(),
          error: null,
          count: publicNotebooks().length,
        }
      case 'notebook_get':
        return notebookView(findNotebook(args.id), true)
      case 'notebook_create':
        return notebookView(saveNotebook(args.draft as Json), true)
      case 'notebook_update': {
        const existing = findNotebook(args.id)
        if (existing.visibility === 'public')
          fail(
            'invalid_input',
            'Public notebooks are read-only. Fork it into your notebooks to change it.',
          )
        return notebookView(saveNotebook(args.patch as Json, existing), true)
      }
      case 'notebook_delete': {
        const existing = findNotebook(args.id)
        if (existing.visibility === 'public')
          fail('invalid_input', 'Public notebooks are read-only.')
        s().notebooks = s().notebooks.filter((entry) => entry !== existing)
        return null
      }
      case 'notebook_fork': {
        const source = findNotebook(args.id)
        return notebookView(
          saveNotebook({ ...source, slug: `${source.slug}-copy`, forkedFrom: source.slug }),
          true,
        )
      }
      case 'notebook_export':
        return `/Users/you/Downloads/${findNotebook(args.id).slug}.nzap.json`
      case 'notebook_import':
        return null
      case 'notebook_run': {
        const notebook = findNotebook(args.id)
        const target = session(args.session)
        const resolved = resolveParams(notebook.params, (args.params as Json) ?? {})
        if (notebook.app) return runApp(target, notebook, resolved, emitter(args.onEvent), streamId)
        const code = `# Injected by NZAP Engine — do not edit.\nimport json as _nzap_json\nparams = _nzap_json.loads(${JSON.stringify(JSON.stringify(resolved))})\ndel _nzap_json\n\n${notebook.source}`
        return runCell(target, code, emitter(args.onEvent), streamId)
      }
      default:
        fail('internal', `The fake engine does not implement ${cmd}.`)
    }
  }

  function settingsView(): Json {
    return {
      settings: s().settings,
      oauthClientId:
        s().customClient ??
        '764086051850-6qr4p6gpi6hn506pt8ejuq83di341hur.apps.googleusercontent.com',
      customOauthClient: Boolean(s().customClient),
      defaultCatalogUrl: DEFAULT_CATALOG,
    }
  }

  /**
   * An NZAP app run: the stages a real app reports (install, load, run), a
   * warm second run, and real output files (a synthesized WAV for audio
   * apps) that the UI fetches with `files_read`, like on Colab.
   */
  async function runApp(
    target: FakeSession,
    notebook: FakeNotebook,
    params: Json,
    emit: Emit,
    streamId?: string,
  ): Promise<Json> {
    const app = notebook.app as Json
    const slug = notebook.slug
    target.connected = true
    target.kernelId ??= `kernel-${target.name}`
    target.lastActivity = now()
    target.count += 1
    const count = target.count
    const pace = s().delay / 120
    const wait = (ms: number) => cancellable(streamId, sleep(ms * pace))
    const event = (payload: Json, text: string) =>
      emit({
        type: 'display',
        data: {
          'application/vnd.nzap.app+json': { v: 1, app: slug, ...payload },
          'text/plain': text,
        },
      })
    const stage = (id: string, label: string, progress: number | null = null) =>
      event({ event: 'stage', id, label, progress }, `[nzap] ${label}…`)

    target.kernelState = 'busy'
    emit({ type: 'status', state: 'busy' })
    emit({ type: 'input', execution_count: count })
    const started = Date.now()
    const warm = target.warmApps.has(slug)
    try {
      if (!warm) {
        stage('install', `Installing ${notebook.title.split(' (')[0]}`)
        emit({ type: 'stream', name: 'stdout', text: 'Collecting packages…\n' })
        await wait(1100)
        stage('download', 'Downloading model weights')
        await wait(900)
        stage('load', 'Loading the model')
        await wait(700)
      }
      const setup = (Date.now() - started) / 1000
      event(
        {
          event: 'ready',
          warm,
          setupSeconds: setup,
          device: target.accelerator === 'CPU' ? 'cpu' : `Tesla ${target.accelerator}`,
        },
        '[nzap] Model ready.',
      )
      target.warmApps.add(slug)

      const runStarted = Date.now()
      const outputs = (app.outputs as Json[] | undefined) ?? []
      const text = String(params.text ?? '')
      const paragraphs = text.split(/\n\s*\n/).filter((part) => part.trim())
      for (let index = 0; index < Math.max(1, paragraphs.length); index += 1) {
        stage(
          'run',
          paragraphs.length > 1
            ? `Generating paragraph ${index + 1} of ${paragraphs.length}`
            : 'Generating',
          paragraphs.length > 1 ? index / paragraphs.length : null,
        )
        await wait(900 / Math.max(1, paragraphs.length))
      }
      const stamp = new Date().toISOString().replace(/\D/g, '').slice(0, 14)
      for (const slot of outputs) {
        const id = String(slot.id)
        if (slot.kind === 'audio') {
          const audio = synthesizeSpeech(text || notebook.title)
          const path = `content/nzap/outputs/${slug}/${stamp}.wav`
          writeFile(target, path, audio.base64)
          target.files.get(path)!.format = 'base64'
          event(
            {
              event: 'output',
              id,
              kind: 'audio',
              path: `/${path}`,
              mime: 'audio/wav',
              meta: { duration: audio.duration, sampleRate: 24000, segments: audio.segments },
            },
            `[nzap] Saved /${path}`,
          )
        } else if (slot.kind === 'table') {
          const rows = text
            .split('\n')
            .filter((line) => line.trim())
            .map((line) => {
              const negative = /\b(not|died|bad|waste|worst|broke|hate|no)\b/i.test(line)
              return [line.trim(), negative ? 'Negative' : 'Positive', negative ? 0.9871 : 0.9993]
            })
          event(
            {
              event: 'output',
              id,
              kind: 'table',
              columns: ['Text', 'Sentiment', 'Confidence'],
              rows,
            },
            `[nzap] Scored ${rows.length} lines.`,
          )
        } else if (slot.kind === 'image') {
          const path = `content/nzap/outputs/${slug}/${stamp}.png`
          writeFile(target, path, SAMPLE_PNG)
          target.files.get(path)!.format = 'base64'
          event(
            { event: 'output', id, kind: 'image', path: `/${path}`, mime: 'image/png' },
            `[nzap] Saved /${path}`,
          )
        } else {
          event({ event: 'output', id, kind: 'text', text }, text)
        }
      }
      event(
        {
          event: 'done',
          warm,
          seconds: { setup, run: (Date.now() - runStarted) / 1000 },
        },
        '[nzap] Done.',
      )
    } finally {
      target.kernelState = 'idle'
    }
    const reply = { type: 'execute_reply', status: 'ok', execution_count: count }
    emit({ type: 'status', state: 'idle' })
    emit(reply)
    log(target.name, 'automation', { op: 'notebook', notebook: notebook.id, title: notebook.title })
    return reply
  }

  function writeFile(target: FakeSession, path: string, content: string) {
    const clean = path.replace(/^\/+|\/+$/g, '')
    if (!clean) fail('invalid_input', 'A file path is required.')
    const parts = clean.split('/')
    for (let index = 1; index < parts.length; index += 1) {
      const parent = parts.slice(0, index).join('/')
      if (!target.files.has(parent)) target.files.set(parent, { dir: true, content: '' })
    }
    target.files.set(clean, { dir: false, content })
    target.lastActivity = now()
  }

  async function runFile(
    target: FakeSession,
    request: Json,
    emit: Emit,
    streamId?: string,
  ): Promise<Json> {
    const filename = String(request.filename)
    const isNotebook = filename.toLowerCase().endsWith('.ipynb')
    const notebook = isNotebook ? (JSON.parse(String(request.content)) as { cells: Json[] }) : null
    const cells = notebook
      ? notebook.cells
          .map((cell, index) => ({ index, cell }))
          .filter(({ cell }) => cell.cell_type === 'code')
      : [{ index: 0, cell: { source: String(request.content) } as Json }]
    let failed = 0
    for (const [position, { cell }] of cells.entries()) {
      emit({ type: 'cell', index: position, total: cells.length, state: 'started' })
      const source = Array.isArray(cell.source)
        ? (cell.source as string[]).join('')
        : String(cell.source)
      const outputs: Json[] = []
      const reply = await runCell(
        target,
        source,
        (event) => {
          if (['stream', 'error', 'result', 'display'].includes(String(event.type)))
            outputs.push(event)
          emit(event)
        },
        streamId,
      )
      if (notebook)
        cell.outputs = outputs.map((event) =>
          event.type === 'stream'
            ? { output_type: 'stream', name: 'stdout', text: event.text }
            : {
                output_type: 'error',
                ename: event.ename,
                evalue: event.evalue,
                traceback: event.traceback ?? [],
              },
        )
      emit({
        type: 'cell',
        index: position,
        total: cells.length,
        state: 'finished',
        status: reply.status,
      })
      if (reply.status !== 'ok') {
        failed += 1
        if (request.stopOnError) break
      }
    }
    const done: Json = {
      type: 'run_complete',
      status: failed ? 'error' : 'ok',
      failed_cells: failed,
      total_cells: cells.length,
    }
    if (notebook) {
      done.filename = `${filename.replace(/\.ipynb$/i, '')}_output.ipynb`
      done.notebook = notebook
    }
    emit(done)
    return done
  }

  async function runJob(request: Json, emit: Emit, streamId?: string): Promise<Json> {
    requireConnected()
    const name = String(request.name || `run-${Math.random().toString(36).slice(2, 8)}`)
    const hardware = request.tpu
      ? String(request.tpu).toUpperCase()
      : request.gpu
        ? String(request.gpu).toUpperCase()
        : 'CPU'
    emit({ type: 'job', phase: 'assigning', session: name, hardware })
    await cancellable(streamId, sleep(s().delay * 2))
    const target = newSession(
      name,
      request.gpu as string | undefined,
      request.tpu as string | undefined,
    )
    s().sessions.set(name, target)
    emit({ type: 'job', phase: 'connecting', session: name, endpoint: target.endpoint, hardware })
    await cancellable(streamId, sleep())
    emit({ type: 'job', phase: 'running', session: name })
    let exitCode = 0
    await runCell(
      target,
      String(request.script),
      (event) => {
        if (event.type === 'error' && event.ename === 'SystemExit') {
          exitCode = Number(event.evalue) || 0
          return
        }
        if (event.type === 'error') exitCode = 1
        emit(event)
      },
      streamId,
    )
    const artifacts = (request.artifacts as string[] | undefined) ?? []
    if (artifacts.length) {
      emit({ type: 'job', phase: 'collecting', session: name })
      await sleep()
      emit({
        type: 'artifact',
        path: '/content/out/result.txt',
        size: 12,
        savedTo: `/Users/you/Downloads/NZAP Engine/${name}/out/result.txt`,
      })
    }
    const kept = Boolean(request.keep)
    if (!kept) {
      s().sessions.delete(name)
      emit({ type: 'job', phase: 'released', session: name })
    }
    const done = { type: 'job_done', exit_code: exitCode, released: !kept, kept, session: name }
    emit(done)
    return done
  }

  mockIPC((cmd, args) => handle(cmd, (args ?? {}) as Json))
  // The mock drops invoke options; the upload command needs its headers.
  const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: unknown } })
    .__TAURI_INTERNALS__
  internals.invoke = (cmd: string, args: unknown, options?: { headers?: Record<string, string> }) =>
    handle(cmd, (args ?? {}) as Json, options)
  console.info('[nzap] Running against the simulated engine (window.__NZAP_FAKE__).')
  return controls
}

/**
 * A WAV that looks and sounds like speech: voiced syllables with formants,
 * word gaps and pauses between paragraphs, seeded by the text so the same
 * input gives the same clip. Returns base64, the duration, and per-paragraph
 * segments like the real TTS apps report.
 */
function synthesizeSpeech(text: string) {
  const rate = 24_000
  let seed = 7
  for (const char of text) seed = (seed * 31 + char.charCodeAt(0)) % 2_147_483_647
  const random = () => {
    seed = (seed * 48_271) % 2_147_483_647
    return seed / 2_147_483_647
  }
  const paragraphs = text
    .split(/\n\s*\n/)
    .map((part) => part.trim())
    .filter(Boolean)
  const samples: number[] = []
  const segments: { start: number; end: number; text: string }[] = []
  for (const paragraph of paragraphs.length ? paragraphs : ['…']) {
    const start = samples.length / rate
    for (const word of paragraph.split(/\s+/).slice(0, 60)) {
      const syllables = Math.max(1, Math.round(word.length / 3))
      for (let syllable = 0; syllable < syllables; syllable += 1) {
        const length = Math.floor(rate * (0.12 + random() * 0.1))
        const pitch = 105 + random() * 70
        const formant = 500 + random() * 1400
        for (let index = 0; index < length; index += 1) {
          const t = index / rate
          const envelope = Math.sin((Math.PI * index) / length) ** 1.5
          const voiced =
            Math.sin(2 * Math.PI * pitch * t) * 0.55 +
            Math.sin(2 * Math.PI * pitch * 2 * t) * 0.25 +
            Math.sin(2 * Math.PI * formant * t) * 0.12
          samples.push(envelope * (voiced + (random() - 0.5) * 0.08) * 0.6)
        }
      }
      const gap = Math.floor(rate * (/[.,!?]$/.test(word) ? 0.28 : 0.07))
      for (let index = 0; index < gap; index += 1) samples.push(0)
      if (samples.length > rate * 30) break
    }
    segments.push({ start, end: samples.length / rate, text: paragraph })
    for (let index = 0; index < rate * 0.45; index += 1) samples.push(0)
  }
  const bytes = new Uint8Array(44 + samples.length * 2)
  const view = new DataView(bytes.buffer)
  const ascii = (offset: number, value: string) =>
    [...value].forEach((char, index) => view.setUint8(offset + index, char.charCodeAt(0)))
  ascii(0, 'RIFF')
  view.setUint32(4, 36 + samples.length * 2, true)
  ascii(8, 'WAVE')
  ascii(12, 'fmt ')
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true)
  view.setUint16(22, 1, true)
  view.setUint32(24, rate, true)
  view.setUint32(28, rate * 2, true)
  view.setUint16(32, 2, true)
  view.setUint16(34, 16, true)
  ascii(36, 'data')
  view.setUint32(40, samples.length * 2, true)
  samples.forEach((sample, index) =>
    view.setInt16(44 + index * 2, Math.max(-1, Math.min(1, sample)) * 32_767, true),
  )
  let binary = ''
  for (let offset = 0; offset < bytes.length; offset += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000))
  return {
    base64: btoa(binary),
    duration: Math.round((samples.length / rate) * 100) / 100,
    segments: segments.map((segment) => ({
      start: Math.round(segment.start * 1000) / 1000,
      end: Math.round(segment.end * 1000) / 1000,
      text: segment.text,
    })),
  }
}

/** A 1×1 PNG. */
const SAMPLE_PNG =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=='
