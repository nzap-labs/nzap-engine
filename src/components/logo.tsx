import { cn } from '@/lib/cn'
import markLight from '@/assets/brand/nzap-mark-light-160.png'
import markDark from '@/assets/brand/nzap-mark-dark-160.png'
import wordMask from '@/assets/brand/nzap-word-mask.png'

export function SparkleIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 32 32" className={className} aria-hidden="true" fill="currentColor">
      <path d="M16 6c.9 6.4 3.7 9.2 10 10-6.3.8-9.1 3.6-10 10-.9-6.4-3.7-9.2-10-10 6.3-.8 9.1-3.6 10-10z" />
    </svg>
  )
}

/**
 * The NZAP Labs "NZ" ribbon: black chrome on light themes, silver chrome
 * (with its own soft glow) on dark ones.
 */
export function LogoMark({ className }: { className?: string }) {
  return (
    <span className={cn('relative inline-block size-9 shrink-0', className)}>
      <img
        src={markLight}
        alt=""
        aria-hidden
        draggable={false}
        className="size-full object-contain dark:hidden"
      />
      <img
        src={markDark}
        alt=""
        aria-hidden
        draggable={false}
        className="hidden size-full object-contain drop-shadow-[0_0_10px_rgba(255,255,255,0.12)] dark:block"
      />
    </span>
  )
}

/** "NZΛP" in the logo's own lettering, tinted with the current text colour. */
export function LogoWord({ className }: { className?: string }) {
  return (
    <span
      role="img"
      aria-label="NZAP"
      className={cn('inline-block aspect-[627/87] h-3 bg-current', className)}
      style={{
        maskImage: `url(${wordMask})`,
        WebkitMaskImage: `url(${wordMask})`,
        maskSize: 'contain',
        WebkitMaskSize: 'contain',
        maskRepeat: 'no-repeat',
        WebkitMaskRepeat: 'no-repeat',
      }}
    />
  )
}

export function LogoWordmark({ className }: { className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-2.5', className)}>
      <LogoMark className="size-9" />
      <span className="flex flex-col gap-1.5">
        <LogoWord />
        <span className="text-[10px] font-medium uppercase leading-none tracking-[0.42em] text-graphite">
          Engine
        </span>
      </span>
    </span>
  )
}
