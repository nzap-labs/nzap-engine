import { useEffect } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link'

/**
 * `nzap://` links, e.g. the website's "Open in NZAP Engine":
 *
 *   nzap://apps/public:kokoro-tts   an app by notebook id
 *   nzap://app/kokoro-tts           a public app by slug
 *   nzap://apps                     the gallery
 *
 * Anything else is ignored: a link only ever navigates, it never runs code.
 */
export type DeepLinkTarget = { to: '/apps' } | { to: '/apps/$appId'; appId: string }

const APP_ID = /^(public:[a-z0-9][a-z0-9-]{1,62}|local:[0-9a-f]{32})$/
const SLUG = /^[a-z0-9][a-z0-9-]{1,62}$/

export function parseDeepLink(link: string): DeepLinkTarget | null {
  let url: URL
  try {
    url = new URL(link)
  } catch {
    return null
  }
  if (url.protocol !== 'nzap:') return null
  // `nzap://apps/x` parses with host "apps" and path "/x".
  const parts = [url.hostname, ...url.pathname.split('/')]
    .map((part) => decodeURIComponent(part))
    .filter(Boolean)
  const [kind, value] = parts
  if (kind === 'apps' && parts.length === 1) return { to: '/apps' }
  if (kind === 'apps' && parts.length === 2 && APP_ID.test(value))
    return { to: '/apps/$appId', appId: value }
  if (kind === 'app' && parts.length === 2 && SLUG.test(value))
    return { to: '/apps/$appId', appId: `public:${value}` }
  return null
}

/** Follows nzap:// links the app was opened with, at launch and later. */
export function DeepLinks() {
  const navigate = useNavigate()
  useEffect(() => {
    const follow = (links: string[] | null) => {
      const target = links?.map(parseDeepLink).find(Boolean)
      if (!target) return
      if (target.to === '/apps') void navigate({ to: '/apps' })
      else void navigate({ to: '/apps/$appId', params: { appId: target.appId } })
    }
    let stop: (() => void) | undefined
    let cancelled = false
    getCurrent()
      .then(follow)
      .catch(() => undefined)
    onOpenUrl(follow)
      .then((unlisten) => {
        if (cancelled) unlisten()
        else stop = unlisten
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
      stop?.()
    }
  }, [navigate])
  return null
}
