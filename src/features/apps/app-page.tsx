import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import {
  ArrowLeft,
  CircleAlert,
  Cpu,
  ExternalLink,
  Flame,
  Loader2,
  Play,
  RotateCcw,
  Square,
  Timer,
} from 'lucide-react'
import { toast } from 'sonner'
import { colabSessionsQuery, colabStatusQuery, useCreateSession } from '@/api/colab'
import { notebooksQuery } from '@/api/notebooks'
import { Button } from '@/components/ui/button'
import { PageHeader } from '@/features/shell/page-header'
import { cn } from '@/lib/cn'
import { errorMessage, openExternal } from '@/lib/ipc'
import type { ColabSession } from '@/types/colab'
import type { AppSpec } from '@/types/app'
import type { Notebook, NotebookParamValues } from '@/types/notebook'
import { AppForm, fileProblems } from './app-form'
import { AppIcon } from './app-icon'
import { AppOutputView } from './app-outputs'
import { RunProgress, type RuntimeStart } from './run-progress'
import {
  aboutDuration,
  appSpecOf,
  formatDuration,
  formSections,
  initialValues,
  reconcile,
  runsOn,
  runtimeNameFor,
  runtimeRequest,
} from './spec'
import { clearResults, estimateFor, isWarm, pruneWarm, useAppStore, type AppResult } from './store'
import { useAppRun } from './use-app-run'

// A stable empty list: store selectors must not return a new array each time.
const NO_RESULTS: AppResult[] = []

/** Wall-clock time, read in event handlers. */
const clock = () => Date.now()

/** How long Google usually takes to hand out a runtime, by kind. */
function runtimeStartEstimate(accelerator: string): number {
  const upper = accelerator.toUpperCase()
  if (upper === 'CPU') return 20
  if (upper.startsWith('V')) return 75
  return 45
}

export function AppPage({ appId }: { appId: string }) {
  const { data, isPending } = useQuery(notebooksQuery)
  const notebook = data?.notebooks.find((entry) => entry.id === appId)
  const spec = notebook ? appSpecOf(notebook) : null

  if (isPending)
    return (
      <Shell title="App">
        <p className="flex items-center gap-2 text-sm text-graphite">
          <Loader2 className="size-4 animate-spin" /> Loading…
        </p>
      </Shell>
    )
  if (!notebook || !spec)
    return (
      <Shell title="App">
        <div className="rounded-[24px] border border-line p-6">
          <p className="font-medium">This app is not available.</p>
          <p className="mt-1 text-sm text-graphite">
            It may have been removed from the collection, or it is a plain notebook.
          </p>
          <Link to="/apps" className="mt-4 inline-flex text-sm font-medium underline">
            Back to Apps
          </Link>
        </div>
      </Shell>
    )
  // Keyed so switching apps starts with a fresh form.
  return <AppView key={notebook.id} notebook={notebook} spec={spec} />
}

function Shell({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader title={title} />
      <div className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-4 pb-10 md:px-6">
        <div className="mx-auto w-full max-w-6xl">{children}</div>
      </div>
    </div>
  )
}

function AppView({ notebook, spec }: { notebook: Notebook; spec: AppSpec }) {
  const { data: status } = useQuery(colabStatusQuery)
  const { data: sessionData } = useQuery({
    ...colabSessionsQuery,
    enabled: Boolean(status?.connected),
  })
  const sessions = useMemo(() => sessionData?.sessions ?? [], [sessionData])
  const timings = useAppStore((state) => state.timings)
  const results = useAppStore((state) => state.results[notebook.id] ?? NO_RESULTS)
  // Re-render when warm state changes.
  useAppStore((state) => state.warm)

  const sections = useMemo(() => formSections(spec, notebook.params), [spec, notebook.params])
  const [values, setValues] = useState<NotebookParamValues>(() => initialValues(sections))
  const [files, setFiles] = useState<Record<string, File>>({})
  const [chosen, setChosen] = useState<string | null>(null)
  const [runtimeStart, setRuntimeStart] = useState<RuntimeStart | null>(null)
  const create = useCreateSession()
  const { state, run, cancel } = useAppRun()

  useEffect(() => {
    if (sessionData) pruneWarm(sessionData.sessions)
  }, [sessionData])

  const usable = sessions.filter((session) => runsOn(spec, session.accelerator))
  // Prefer the runtime you picked, then one that already has the app loaded,
  // then the recommended accelerator.
  const runtime: ColabSession | null =
    sessions.find((session) => session.name === chosen) ??
    usable.find((session) => isWarm(notebook.slug, session)) ??
    usable.find((session) => session.accelerator === spec.runtime.accelerator) ??
    usable[0] ??
    null
  const warm = runtime ? isWarm(notebook.slug, runtime) : false
  const accelerator = runtime?.accelerator ?? spec.runtime.accelerator
  const estimate = estimateFor(spec, notebook.slug, accelerator, timings, warm)
  const busy = state.status === 'uploading' || state.status === 'running' || create.isPending
  const problems = fileProblems(sections, files)
  const unsupported = runtime && !runsOn(spec, runtime.accelerator)
  const total =
    (runtime ? 0 : runtimeStartEstimate(spec.runtime.accelerator)) + estimate.setup + estimate.run

  function change(key: string, value: string | number | boolean) {
    setValues((current) => reconcile(sections, { ...current, [key]: value }))
  }

  function setFile(key: string, file: File | null) {
    setFiles((current) => {
      const next = { ...current }
      if (file) next[key] = file
      else delete next[key]
      return next
    })
    if (!file) change(key, '')
  }

  async function startRuntime(): Promise<ColabSession | null> {
    const startedAt = clock()
    const accelerator = spec.runtime.accelerator
    setRuntimeStart({ startedAt, finishedAt: null, expected: runtimeStartEstimate(accelerator) })
    try {
      const name = runtimeNameFor(
        notebook.slug,
        sessions.map((session) => session.name),
      )
      const result = await create.mutateAsync(
        runtimeRequest(accelerator, name, spec.runtime.highMem),
      )
      setRuntimeStart({
        startedAt,
        finishedAt: clock(),
        expected: runtimeStartEstimate(accelerator),
      })
      setChosen(result.session.name)
      toast.success(`Runtime ${result.session.name} is ready (${result.session.accelerator}).`)
      return result.session
    } catch (error) {
      setRuntimeStart(null)
      toast.error(errorMessage(error, 'Could not start a runtime.'))
      return null
    }
  }

  async function generate() {
    if (busy || problems.length) return
    let target = runtime
    if (!target) {
      target = await startRuntime()
      if (!target) return
    } else {
      setRuntimeStart(null)
    }
    await run({ notebook, runtime: target, values, files })
  }

  const latest: AppResult | null = results[0] ?? null
  const liveOutputs = state.status === 'running' || state.status === 'uploading'
  const showProgress = runtimeStart !== null || state.status !== 'idle'

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title="Apps"
        actions={
          <Link
            to="/apps"
            className="inline-flex items-center gap-1.5 rounded-3xl border border-ink px-3 py-1.5 text-xs font-medium hover:bg-paper-soft"
          >
            <ArrowLeft className="size-3.5" /> All apps
          </Link>
        }
      />
      <div className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-4 pb-10 md:px-6">
        <div className="mx-auto w-full max-w-6xl space-y-6">
          <header className="flex flex-col gap-5 rounded-[24px] border border-ink bg-paper p-6 md:flex-row md:items-center">
            <AppIcon icon={spec.icon} category={spec.category} className="size-16 rounded-[20px]" />
            <div className="min-w-0 flex-1">
              <h2 className="text-2xl font-medium tracking-tight">{notebook.title}</h2>
              <p className="mt-1 max-w-2xl text-sm leading-relaxed text-graphite">
                {spec.tagline ?? notebook.description}
              </p>
              <div className="mt-3 flex flex-wrap gap-2 text-xs">
                <Chip icon={<Cpu className="size-3.5" />}>
                  Best on {spec.runtime.accelerator}
                  {spec.runtime.minVramGb ? ` · ${spec.runtime.minVramGb} GB+ VRAM` : ''}
                </Chip>
                <Chip icon={<Timer className="size-3.5" />}>
                  Setup {aboutDuration(spec.estimates.setup)} · each run{' '}
                  {aboutDuration(spec.estimates.run)}
                </Chip>
                {spec.license && <Chip>{spec.license}</Chip>}
                {spec.links?.map((link) => (
                  <button
                    key={link.url}
                    type="button"
                    onClick={() => void openExternal(link.url).catch(() => undefined)}
                    className="inline-flex cursor-pointer items-center gap-1 rounded-full border border-line px-2.5 py-1 font-medium hover:border-ink"
                  >
                    {link.label} <ExternalLink className="size-3" />
                  </button>
                ))}
              </div>
            </div>
          </header>

          <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
            <section
              aria-label="Inputs"
              className="space-y-5 rounded-[24px] border border-ink bg-paper p-6"
            >
              <RuntimeChooser
                spec={spec}
                slug={notebook.slug}
                connected={Boolean(status?.connected)}
                sessions={sessions}
                runtime={runtime}
                disabled={busy}
                onChoose={setChosen}
              />

              {spec.examples && spec.examples.length > 0 && (
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
                    Try
                  </span>
                  {spec.examples.map((example) => (
                    <button
                      key={example.label}
                      type="button"
                      disabled={busy}
                      onClick={() =>
                        setValues(initialValues(sections, { ...values, ...example.values }))
                      }
                      className="h-8 cursor-pointer rounded-3xl border border-line px-3 text-xs font-medium hover:border-ink disabled:opacity-60"
                    >
                      {example.label}
                    </button>
                  ))}
                </div>
              )}

              <AppForm
                sections={sections}
                values={values}
                files={files}
                disabled={busy}
                onChange={change}
                onFile={setFile}
              />

              {problems.map((problem) => (
                <p key={problem} className="text-sm text-coral">
                  {problem}
                </p>
              ))}
              {unsupported && (
                <p className="flex items-start gap-2 text-sm text-coral">
                  <CircleAlert className="mt-0.5 size-4 shrink-0" />
                  This app does not run on {runtime.accelerator}. Pick or start a{' '}
                  {spec.runtime.accelerator} runtime.
                </p>
              )}

              <div className="flex flex-wrap items-center gap-3 border-t border-line pt-5">
                {state.status === 'running' || state.status === 'uploading' ? (
                  <Button variant="secondary" onClick={cancel}>
                    <Square className="size-4" /> Stop
                  </Button>
                ) : (
                  <Button
                    size="lg"
                    onClick={() => void generate()}
                    disabled={
                      busy || !status?.connected || Boolean(unsupported) || problems.length > 0
                    }
                  >
                    {create.isPending ? (
                      <Loader2 className="size-4 animate-spin" />
                    ) : (
                      <Play className="size-4" />
                    )}
                    {create.isPending ? 'Starting runtime…' : (spec.runLabel ?? 'Run')}
                  </Button>
                )}
                <p className="text-xs leading-relaxed text-graphite">
                  {warm ? (
                    <>
                      <Flame className="mr-1 inline size-3.5 text-coral" />
                      Warm on {runtime?.name} · each run {aboutDuration(estimate.run)}
                    </>
                  ) : (
                    <>
                      {aboutDuration(total)} on {accelerator}
                      {estimate.source === 'yours' ? ' (from your last run)' : ''}:{' '}
                      {!runtime &&
                        `start ${aboutDuration(runtimeStartEstimate(spec.runtime.accelerator))} · `}
                      setup {aboutDuration(estimate.setup)} · run {aboutDuration(estimate.run)}
                      {spec.estimates.runNote ? ` ${spec.estimates.runNote}` : ''}
                    </>
                  )}
                </p>
              </div>
            </section>

            <section
              aria-label="Results"
              className="space-y-5 self-start rounded-[24px] border border-ink bg-paper p-6 lg:sticky lg:top-0"
            >
              <div className="flex items-center justify-between gap-3">
                <p className="font-medium">Result</p>
                {results.length > 0 && !busy && (
                  <button
                    type="button"
                    onClick={() => clearResults(notebook.id)}
                    className="inline-flex cursor-pointer items-center gap-1 text-xs text-graphite hover:text-ink"
                  >
                    <RotateCcw className="size-3.5" /> Clear
                  </button>
                )}
              </div>

              {showProgress && (
                <RunProgress state={state} estimate={estimate} runtimeStart={runtimeStart} />
              )}

              {state.status === 'error' && (
                <div className="rounded-2xl border border-coral p-4 text-sm">
                  <p className="flex items-center gap-2 font-medium text-coral">
                    <CircleAlert className="size-4" /> The app stopped
                  </p>
                  <p className="mt-1 whitespace-pre-wrap break-words text-graphite">
                    {state.error}
                  </p>
                </div>
              )}

              {liveOutputs
                ? state.outputs.map((output) => (
                    <AppOutputView
                      key={`${output.id}-${output.path ?? ''}`}
                      output={output}
                      slot={spec.outputs.find((slot) => slot.id === output.id)}
                      runtime={runtime?.name ?? ''}
                    />
                  ))
                : latest && <ResultView result={latest} spec={spec} />}

              {!latest && !showProgress && (
                <div className="grid place-items-center rounded-2xl border border-dashed border-line px-6 py-14 text-center">
                  <AppIcon icon={spec.icon} category={spec.category} className="opacity-80" />
                  <p className="mt-4 text-sm font-medium">Nothing generated yet</p>
                  <p className="mt-1 max-w-xs text-xs leading-relaxed text-graphite">
                    Fill in the form and press {spec.runLabel ?? 'Run'}. Everything runs on your own
                    Colab runtime.
                  </p>
                </div>
              )}

              {results.length > 1 && (
                <details className="group">
                  <summary className="cursor-pointer text-sm font-medium text-graphite hover:text-ink">
                    Earlier results ({results.length - 1})
                  </summary>
                  <div className="mt-4 space-y-6">
                    {results.slice(1).map((result) => (
                      <ResultView key={result.id} result={result} spec={spec} compact />
                    ))}
                  </div>
                </details>
              )}

              {state.log && (
                <details>
                  <summary className="cursor-pointer text-xs font-medium text-graphite hover:text-ink">
                    Runtime log
                  </summary>
                  <pre className="scrollbar-thin mt-2 max-h-64 overflow-auto rounded-2xl bg-paper-soft p-3 font-mono text-[11px] leading-relaxed text-graphite">
                    {state.log}
                  </pre>
                </details>
              )}
            </section>
          </div>
        </div>
      </div>
    </div>
  )
}

function Chip({ icon, children }: { icon?: ReactNode; children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-line px-2.5 py-1 font-medium">
      {icon}
      {children}
    </span>
  )
}

function ResultView({
  result,
  spec,
  compact = false,
}: {
  result: AppResult
  spec: AppSpec
  compact?: boolean
}) {
  return (
    <div className={cn('space-y-3', compact && 'border-t border-line pt-4')}>
      <p className="text-xs text-graphite">
        {new Date(result.startedAt).toLocaleTimeString()} · {formatDuration(result.seconds)} on{' '}
        {result.accelerator}
        {result.warm ? ' (warm)' : ''}
      </p>
      {result.outputs.map((output) => (
        <AppOutputView
          key={`${output.id}-${output.path ?? ''}`}
          output={output}
          slot={spec.outputs.find((slot) => slot.id === output.id)}
          runtime={result.runtime}
        />
      ))}
    </div>
  )
}

function RuntimeChooser({
  spec,
  slug,
  connected,
  sessions,
  runtime,
  disabled,
  onChoose,
}: {
  spec: AppSpec
  slug: string
  connected: boolean
  sessions: ColabSession[]
  runtime: ColabSession | null
  disabled: boolean
  onChoose: (name: string) => void
}) {
  if (!connected)
    return (
      <div className="rounded-2xl border border-line bg-paper-soft p-4 text-sm">
        <p className="font-medium">Connect Google to run apps</p>
        <p className="mt-1 text-graphite">
          Apps run on your own Colab runtimes.{' '}
          <Link to="/colab" className="font-medium text-ink underline">
            Connect on the Colab page
          </Link>
          .
        </p>
      </div>
    )
  if (sessions.length === 0)
    return (
      <div className="flex items-center gap-3 rounded-2xl border border-line bg-paper-soft p-4 text-sm">
        <Cpu className="size-4 shrink-0 text-graphite" />
        <p className="text-graphite">
          No runtime yet. Pressing {spec.runLabel ?? 'Run'} starts a{' '}
          <span className="font-medium text-ink">{spec.runtime.accelerator}</span> runtime for you
          (about {aboutDuration(runtimeStartEstimate(spec.runtime.accelerator))}).
        </p>
      </div>
    )
  return (
    <label className="block">
      <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">Runtime</span>
      <select
        value={runtime?.name ?? ''}
        disabled={disabled}
        onChange={(event) => onChoose(event.target.value)}
        className="mt-1.5 h-11 w-full rounded-2xl border border-ink bg-transparent px-3 text-sm outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
      >
        {sessions.map((session) => (
          <option
            key={session.name}
            value={session.name}
            disabled={!runsOn(spec, session.accelerator)}
          >
            {session.name} · {session.accelerator}
            {isWarm(slug, session) ? ' · warm' : ''}
            {runsOn(spec, session.accelerator) ? '' : ' · not supported'}
          </option>
        ))}
      </select>
    </label>
  )
}
