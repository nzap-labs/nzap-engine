import { cn } from '@/lib/cn'

function initialsOf(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean)
  const initials = parts
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? '')
    .join('')
  return initials || '?'
}

/** Avatar with ink-ring styling and an initials fallback. */
export function Avatar({
  src,
  name,
  className,
}: {
  src?: string
  name: string
  className?: string
}) {
  if (src) {
    return (
      <img
        src={src}
        alt={name}
        referrerPolicy="no-referrer"
        className={cn('shrink-0 rounded-full border border-ink object-cover', className)}
      />
    )
  }
  return (
    <span
      aria-label={name}
      className={cn(
        'grid shrink-0 place-items-center rounded-full bg-ink font-medium text-paper',
        className,
      )}
    >
      {initialsOf(name)}
    </span>
  )
}
