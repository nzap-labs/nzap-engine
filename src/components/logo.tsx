import { cn } from '@/lib/cn'

export function SparkleIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 32 32" className={className} aria-hidden="true" fill="currentColor">
      <path d="M16 6c.9 6.4 3.7 9.2 10 10-6.3.8-9.1 3.6-10 10-.9-6.4-3.7-9.2-10-10 6.3-.8 9.1-3.6 10-10z" />
    </svg>
  )
}

export function LogoMark({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        'inline-grid size-9 shrink-0 place-items-center rounded-[10px] bg-ink',
        className,
      )}
    >
      <SparkleIcon className="size-5 text-sunshine" />
    </span>
  )
}

export function LogoWordmark({ className }: { className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-2.5', className)}>
      <LogoMark />
      <span className="text-lg font-medium tracking-tight">NZAP</span>
    </span>
  )
}
