import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Cpu, FileUp, FolderOpen, NotebookPen, SquareTerminal, Terminal } from 'lucide-react'
import { colabSessionsQuery, colabStatusQuery } from '@/api/colab'
import type { Notebook } from '@/types/notebook'
import { PageHeader } from '@/features/shell/page-header'
import { ConnectionCard } from './connection-card'
import { NewRuntimeCard } from './new-runtime-card'
import { RuntimeList } from './runtime-list'
import { ConsolePanel } from './console-panel'
import { FilesPanel } from './files-panel'
import { NotebooksPanel } from './notebooks-panel'
import { NotebookRunDialog } from './notebook-run-dialog'
import { Telemetry } from './telemetry'
import { ConsumptionChip } from './consumption'
import { HistoryPanel } from './history-panel'
import { SetupCard } from './setup-card'
import { RunFilePanel } from './run-file-panel'
import { JobsPanel } from './jobs-panel'
import { TerminalPanel } from './terminal-panel'
import { cn } from '@/lib/cn'
import { Onboarding } from './onboarding'

type Tab = 'runtimes' | 'console' | 'terminal' | 'run' | 'files' | 'notebooks'

const TABS: { id: Tab; label: string; icon: typeof Cpu }[] = [
  { id: 'runtimes', label: 'Runtimes', icon: Cpu },
  { id: 'console', label: 'Console', icon: Terminal },
  { id: 'terminal', label: 'Terminal', icon: SquareTerminal },
  { id: 'run', label: 'Run', icon: FileUp },
  { id: 'notebooks', label: 'Notebooks', icon: NotebookPen },
  { id: 'files', label: 'Files', icon: FolderOpen },
]

/** Arrow keys, Home and End move between tabs (WAI-ARIA tabs pattern). */
function tabAfterKey(current: Tab, key: string): Tab | null {
  const index = TABS.findIndex((item) => item.id === current)
  const last = TABS.length - 1
  const target =
    key === 'ArrowRight'
      ? (index + 1) % TABS.length
      : key === 'ArrowLeft'
        ? (index - 1 + TABS.length) % TABS.length
        : key === 'Home'
          ? 0
          : key === 'End'
            ? last
            : null
  return target === null ? null : TABS[target].id
}

/**
 * The Colab workspace: connect Google, launch runtimes, run code, browse
 * files. Every action goes through the engine, which talks to Google with the
 * user's own token — no server in between.
 */
export function ColabWorkspace() {
  const [tab, setTab] = useState<Tab>('runtimes')
  const [activeName, setActiveName] = useState<string | null>(null)
  const [runNotebook, setRunNotebook] = useState<Notebook | null>(null)

  const { data: status } = useQuery(colabStatusQuery)
  const { data: sessions } = useQuery(colabSessionsQuery)

  const names = sessions?.sessions ?? []
  const active =
    activeName && names.some((session) => session.name === activeName) ? activeName : null

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader
        title="Colab"
        actions={
          <div className="flex flex-wrap items-center justify-end gap-2">
            <ConsumptionChip enabled={Boolean(status?.connected)} />
            <span className="flex items-center gap-2 rounded-full border border-ink px-3 py-1 text-xs font-medium">
              <span
                aria-hidden
                className={cn(
                  'size-2 rounded-full',
                  status?.connected ? 'bg-mint' : 'bg-graphite/40',
                )}
              />
              Google Auth {status?.connected ? 'connected' : 'off'}
            </span>
          </div>
        }
      />

      <div className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-4 pb-10 md:px-6">
        <div className="mx-auto w-full max-w-4xl space-y-6">
          <ConnectionCard />

          {status && !status.connected ? (
            <Onboarding />
          ) : (
            <>
              <div
                className="flex flex-wrap gap-2"
                role="tablist"
                aria-label="Colab workspace"
                onKeyDown={(event) => {
                  const next = tabAfterKey(tab, event.key)
                  if (!next) return
                  event.preventDefault()
                  setTab(next)
                  document.getElementById(`colab-tab-${next}`)?.focus()
                }}
              >
                {TABS.map((item) => (
                  <button
                    key={item.id}
                    id={`colab-tab-${item.id}`}
                    type="button"
                    role="tab"
                    aria-selected={tab === item.id}
                    aria-controls="colab-tabpanel"
                    tabIndex={tab === item.id ? 0 : -1}
                    onClick={() => setTab(item.id)}
                    className={cn(
                      'inline-flex h-9 cursor-pointer items-center gap-2 rounded-3xl border px-4 text-sm font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink',
                      tab === item.id
                        ? 'border-ink bg-sunshine text-on-sunshine'
                        : 'border-ink text-ink hover:bg-paper-soft',
                    )}
                  >
                    <item.icon className="size-4" />
                    {item.label}
                  </button>
                ))}
              </div>

              <div id="colab-tabpanel" role="tabpanel" aria-labelledby={`colab-tab-${tab}`}>
                {tab === 'runtimes' && (
                  <div className="space-y-6">
                    <NewRuntimeCard
                      onCreated={(name) => {
                        setActiveName(name)
                        setTab('console')
                      }}
                    />
                    <RuntimeList
                      activeName={active}
                      onSelect={(name) => {
                        setActiveName(name)
                        setTab('console')
                      }}
                    />
                  </div>
                )}

                {tab === 'console' && (
                  <div className="space-y-4">
                    <Telemetry sessionName={active} />
                    {/* Keyed by session so switching runtimes starts a clean transcript. */}
                    <ConsolePanel key={active ?? 'none'} sessionName={active} />
                    {active && <SetupCard key={`setup-${active}`} sessionName={active} />}
                    {active && <HistoryPanel key={`history-${active}`} sessionName={active} />}
                  </div>
                )}

                {tab === 'terminal' && (
                  <TerminalPanel key={active ?? 'none'} sessionName={active} />
                )}

                {tab === 'run' && (
                  <div className="space-y-6">
                    <JobsPanel />
                    <RunFilePanel key={active ?? 'none'} sessionName={active} />
                  </div>
                )}

                {tab === 'notebooks' && (
                  <NotebooksPanel
                    canRun={Boolean(active)}
                    runtimeName={active}
                    onRun={setRunNotebook}
                  />
                )}

                {tab === 'files' && <FilesPanel key={active ?? 'none'} sessionName={active} />}
              </div>
            </>
          )}

          {runNotebook && (
            <NotebookRunDialog
              notebook={runNotebook}
              sessions={(sessions?.sessions ?? []).map((session) => ({
                name: session.name,
                accelerator: session.accelerator,
              }))}
              defaultSession={active}
              onClose={() => setRunNotebook(null)}
            />
          )}
        </div>
      </div>
    </div>
  )
}
