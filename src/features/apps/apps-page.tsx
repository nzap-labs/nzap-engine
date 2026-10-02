import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { ArrowUpRight, Cpu, Flame, Loader2, RefreshCw, Search, Timer } from 'lucide-react'
import { toast } from 'sonner'
import { colabSessionsQuery, colabStatusQuery } from '@/api/colab'
import { notebooksQuery, useRefreshCatalog } from '@/api/notebooks'
import { PageHeader } from '@/features/shell/page-header'
import { cn } from '@/lib/cn'
import { errorMessage, openExternal } from '@/lib/ipc'
import type { AppCategory, AppSpec } from '@/types/app'
import type { Notebook } from '@/types/notebook'
import { AppIcon } from './app-icon'
import { aboutDuration, appSpecOf } from './spec'
import { isWarm, useAppStore } from './store'

const CATEGORY_LABELS: Record<AppCategory, string> = {
  audio: 'Audio',
  image: 'Image',
  video: 'Video',
  text: 'Text',
  vision: 'Vision',
  data: 'Data',
  utility: 'Utility',
}

const APPS_GUIDE = 'https://github.com/nzap-labs/nzap-notebooks/blob/main/APPS.md'

interface AppEntry {
  notebook: Notebook
  spec: AppSpec
}

/** The app gallery: every notebook in the collection that ships an app spec. */
export function AppsPage() {
  const { data, isPending, error } = useQuery(notebooksQuery)
  const { data: status } = useQuery(colabStatusQuery)
  const { data: sessionData } = useQuery({
    ...colabSessionsQuery,
    enabled: Boolean(status?.connected),
  })
  const refresh = useRefreshCatalog()
  useAppStore((state) => state.warm)
  const [query, setQuery] = useState('')
  const [category, setCategory] = useState<AppCategory | 'all'>('all')

  const apps = useMemo<AppEntry[]>(
    () =>
      (data?.notebooks ?? []).flatMap((notebook) => {
        const spec = appSpecOf(notebook)
        return spec ? [{ notebook, spec }] : []
      }),
    [data],
  )
  const categories = [...new Set(apps.map((app) => app.spec.category))]
  const needle = query.trim().toLowerCase()
  const shown = apps.filter(
    ({ notebook, spec }) =>
      (category === 'all' || spec.category === category) &&
      (!needle ||
        [notebook.title, notebook.description, spec.tagline ?? '', ...notebook.tags]
          .join(' ')
          .toLowerCase()
          .includes(needle)),
  )
  const sessions = sessionData?.sessions ?? []

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title="Apps"
        actions={
          <button
            type="button"
            onClick={() =>
              refresh.mutate(undefined, {
                onSuccess: (catalog) =>
                  catalog.error
                    ? toast.error(`Could not refresh: ${catalog.error}`)
                    : toast.success(`Collection up to date (${catalog.count} notebooks).`),
                onError: (failure) => toast.error(errorMessage(failure)),
              })
            }
            disabled={refresh.isPending}
            className="inline-flex cursor-pointer items-center gap-1.5 rounded-3xl border border-ink px-3 py-1.5 text-xs font-medium hover:bg-paper-soft disabled:opacity-60"
          >
            <RefreshCw className={cn('size-3.5', refresh.isPending && 'animate-spin')} />
            Refresh
          </button>
        }
      />
      <div className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-4 pb-10 md:px-6">
        <div className="mx-auto w-full max-w-6xl space-y-6">
          <section className="relative overflow-hidden rounded-[24px] border border-ink bg-paper p-6 md:p-8">
            <div className="pointer-events-none absolute -right-24 -top-24 size-80 rounded-full bg-ink/10 blur-3xl" />
            <p className="relative text-xs font-medium uppercase tracking-[0.18em] text-graphite">
              Run AI on your own Colab
            </p>
            <h2 className="chrome-text relative mt-2 max-w-2xl text-3xl font-medium tracking-tight md:text-4xl">
              One click from model to result.
            </h2>
            <p className="relative mt-3 max-w-2xl text-sm leading-relaxed text-graphite">
              Each app knows the runtime it needs and how long it takes. NZAP starts the runtime,
              sets the model up once, and keeps it warm for the next run, all on your own Google
              account.
            </p>
            <div className="relative mt-6 flex flex-col gap-3 sm:flex-row sm:items-center">
              <label className="flex h-11 flex-1 items-center gap-2 rounded-2xl border border-ink bg-paper px-4">
                <Search className="size-4 text-graphite" />
                <input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="Search apps"
                  aria-label="Search apps"
                  className="h-full min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-graphite/70"
                />
              </label>
              <div className="flex flex-wrap gap-2" role="group" aria-label="Category">
                {(['all', ...categories] as const).map((option) => (
                  <button
                    key={option}
                    type="button"
                    aria-pressed={category === option}
                    onClick={() => setCategory(option)}
                    className={cn(
                      'h-9 cursor-pointer rounded-3xl border border-ink px-4 text-sm font-medium transition-colors',
                      category === option
                        ? 'bg-sunshine text-on-sunshine'
                        : 'text-ink hover:bg-paper-soft',
                    )}
                  >
                    {option === 'all' ? 'All' : CATEGORY_LABELS[option]}
                  </button>
                ))}
              </div>
            </div>
          </section>

          {isPending ? (
            <p className="flex items-center gap-2 text-sm text-graphite">
              <Loader2 className="size-4 animate-spin" /> Loading the collection…
            </p>
          ) : error ? (
            <p className="text-sm text-coral">{errorMessage(error)}</p>
          ) : (
            <ul className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
              {shown.map(({ notebook, spec }) => {
                const warmOn = sessions.find((session) => isWarm(notebook.slug, session))
                return (
                  <li key={notebook.id}>
                    <Link
                      to="/apps/$appId"
                      params={{ appId: notebook.id }}
                      className="group flex h-full flex-col rounded-[24px] border border-ink bg-paper p-5 transition-all hover:-translate-y-0.5 hover:shadow-[0_12px_32px_-16px_rgba(17,17,15,0.45)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
                    >
                      <div className="flex items-start justify-between gap-3">
                        <AppIcon icon={spec.icon} category={spec.category} />
                        {warmOn ? (
                          <span className="inline-flex items-center gap-1 rounded-full bg-coral/15 px-2.5 py-1 text-xs font-medium text-coral">
                            <Flame className="size-3.5" /> Warm
                          </span>
                        ) : (
                          <ArrowUpRight className="size-5 text-graphite transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5 group-hover:text-ink" />
                        )}
                      </div>
                      <p className="mt-4 font-medium leading-snug">{notebook.title}</p>
                      <p className="mt-1 line-clamp-2 text-sm leading-relaxed text-graphite">
                        {spec.tagline ?? notebook.description}
                      </p>
                      <div className="mt-auto flex flex-wrap gap-2 pt-4 text-xs text-graphite">
                        <span className="inline-flex items-center gap-1">
                          <Cpu className="size-3.5" /> {spec.runtime.accelerator}
                        </span>
                        <span aria-hidden>·</span>
                        <span className="inline-flex items-center gap-1">
                          <Timer className="size-3.5" />
                          {warmOn ? 'instant' : `setup ${aboutDuration(spec.estimates.setup)}`}
                          {' · run '}
                          {aboutDuration(spec.estimates.run)}
                        </span>
                      </div>
                    </Link>
                  </li>
                )
              })}
              <li>
                <button
                  type="button"
                  onClick={() => void openExternal(APPS_GUIDE).catch(() => undefined)}
                  className="flex h-full w-full cursor-pointer flex-col items-start rounded-[24px] border border-dashed border-ink p-5 text-left transition-colors hover:bg-paper-soft"
                >
                  <span className="text-3xl leading-none" aria-hidden>
                    +
                  </span>
                  <p className="mt-4 font-medium">Build an app</p>
                  <p className="mt-1 text-sm leading-relaxed text-graphite">
                    Any notebook becomes an app with a small app.json. Publish it to the community
                    collection.
                  </p>
                  <span className="mt-auto inline-flex items-center gap-1 pt-4 text-xs font-medium">
                    Read the guide <ArrowUpRight className="size-3.5" />
                  </span>
                </button>
              </li>
            </ul>
          )}
          {!isPending && shown.length === 0 && apps.length > 0 && (
            <p className="text-sm text-graphite">No apps match “{query}”.</p>
          )}
        </div>
      </div>
    </div>
  )
}
