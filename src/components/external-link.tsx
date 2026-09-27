import type { AnchorHTMLAttributes, MouseEvent } from 'react'
import { toast } from 'sonner'
import { errorMessage, openExternal } from '@/lib/ipc'

/**
 * A link that opens in the user's browser. Desktop webviews do not open
 * `target="_blank"` links, so the click goes through the engine, which only
 * allows `https://` URLs.
 */
export function ExternalLink({
  href,
  onClick,
  children,
  ...props
}: AnchorHTMLAttributes<HTMLAnchorElement> & { href: string }) {
  function open(event: MouseEvent<HTMLAnchorElement>) {
    onClick?.(event)
    event.preventDefault()
    openExternal(href).catch((error: unknown) =>
      toast.error(errorMessage(error, 'Could not open the link.')),
    )
  }
  return (
    <a href={href} rel="noreferrer" onClick={open} {...props}>
      {children}
    </a>
  )
}
