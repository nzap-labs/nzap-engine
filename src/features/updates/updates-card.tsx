import { useQuery } from '@tanstack/react-query'
import { CircleCheck, Download, ExternalLink, Loader2, RefreshCw, RotateCw } from 'lucide-react'
import { appInfoQuery } from '@/api/app'
import { Button } from '@/components/ui/button'
import { openExternal } from '@/lib/ipc'
import {
  autoCheckEnabled,
  checkForUpdates,
  installUpdate,
  RELEASES_URL,
  setAutoCheck,
  useUpdates,
} from './updates'

function megabytes(bytes: number) {
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

/** Settings → Updates: version, check, download progress, restart. */
export function UpdatesCard() {
  const { data: info } = useQuery(appInfoQuery)
  const updates = useUpdates()
  const busy = updates.phase === 'checking' || updates.phase === 'downloading'
  const fraction =
    updates.total && updates.total > 0 ? Math.min(1, updates.downloaded / updates.total) : null

  return (
    <section
      id="updates"
      aria-label="Updates"
      className="rounded-[24px] border border-ink bg-paper p-6"
    >
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <p className="font-medium">Updates</p>
          <p className="mt-1 text-sm leading-relaxed text-graphite">
            NZAP Engine {info?.version ?? ''} · updates are signed and verified before they install.
          </p>
        </div>
        <Button
          variant="secondary"
          size="sm"
          disabled={busy}
          onClick={() => void checkForUpdates()}
        >
          {updates.phase === 'checking' ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <RefreshCw className="size-4" />
          )}
          Check for updates
        </Button>
      </div>

      <div className="mt-4 space-y-3" aria-live="polite">
        {updates.phase === 'current' && (
          <p className="flex items-center gap-2 text-sm">
            <CircleCheck className="size-4 text-mint" /> You have the latest version.
          </p>
        )}

        {(updates.phase === 'available' ||
          updates.phase === 'downloading' ||
          updates.phase === 'installed') && (
          <div className="rounded-2xl border border-line bg-paper-soft p-4">
            <p className="font-medium">Version {updates.version} is available</p>
            {updates.date && (
              <p className="text-xs text-graphite">
                Released {new Date(updates.date).toLocaleDateString()}
              </p>
            )}
            {updates.notes && (
              <p className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap text-sm leading-relaxed text-graphite">
                {updates.notes}
              </p>
            )}
            {updates.phase === 'downloading' && (
              <div className="mt-3">
                <div className="h-1.5 overflow-hidden rounded-full bg-ink/10">
                  <div
                    className="h-full rounded-full bg-ink transition-[width]"
                    style={{ width: `${Math.round((fraction ?? 0.5) * 100)}%` }}
                  />
                </div>
                <p className="mt-1.5 text-xs tabular-nums text-graphite">
                  {megabytes(updates.downloaded)}
                  {updates.total ? ` of ${megabytes(updates.total)}` : ''}
                </p>
              </div>
            )}
            <div className="mt-4 flex flex-wrap gap-2">
              {updates.phase === 'installed' ? (
                <p className="flex items-center gap-2 text-sm">
                  <RotateCw className="size-4 animate-spin" /> Restarting into the new version…
                </p>
              ) : (
                <Button size="sm" disabled={busy} onClick={() => void installUpdate()}>
                  {updates.phase === 'downloading' ? (
                    <Loader2 className="size-4 animate-spin" />
                  ) : (
                    <Download className="size-4" />
                  )}
                  {updates.phase === 'downloading' ? 'Installing…' : 'Install and restart'}
                </Button>
              )}
            </div>
          </div>
        )}

        {updates.phase === 'error' && (
          <div className="rounded-2xl border border-line p-4 text-sm">
            <p>{updates.error}</p>
            <button
              type="button"
              onClick={() => void openExternal(RELEASES_URL).catch(() => undefined)}
              className="mt-2 inline-flex cursor-pointer items-center gap-1.5 font-medium underline"
            >
              Open the releases page <ExternalLink className="size-3.5" />
            </button>
          </div>
        )}

        <label className="flex items-center gap-2.5 text-sm">
          <input
            type="checkbox"
            defaultChecked={autoCheckEnabled()}
            onChange={(event) => setAutoCheck(event.target.checked)}
            className="size-4 accent-[var(--color-ink)]"
          />
          Check for updates when NZAP Engine starts
        </label>
      </div>
    </section>
  )
}
