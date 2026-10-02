import { useEffect } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { toast } from 'sonner'
import { autoCheckEnabled, checkForUpdates } from './updates'

const STARTUP_DELAY_MS = 6_000

/**
 * Look for an update shortly after launch (unless turned off in Settings).
 * Only a found update says anything; an unreachable feed stays quiet.
 */
export function StartupUpdateCheck() {
  const navigate = useNavigate()
  useEffect(() => {
    if (!autoCheckEnabled()) return
    const timer = setTimeout(() => {
      void checkForUpdates().then((version) => {
        if (!version) return
        toast(`NZAP Engine ${version} is available`, {
          description: 'It installs in a few seconds and keeps your runtimes.',
          duration: 15_000,
          action: {
            label: 'Update',
            onClick: () => void navigate({ to: '/settings', hash: 'updates' }),
          },
        })
      })
    }, STARTUP_DELAY_MS)
    return () => clearTimeout(timer)
  }, [navigate])
  return null
}
