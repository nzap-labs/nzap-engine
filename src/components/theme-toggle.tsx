import { useSyncExternalStore } from 'react'
import { Moon, Sun } from 'lucide-react'
import { cn } from '@/lib/cn'
import { getThemeSnapshot, subscribeTheme, toggleTheme } from '@/lib/theme'

export function useTheme() {
  return useSyncExternalStore(subscribeTheme, getThemeSnapshot)
}

/** Light/dark switch. Pure icon button — wrap in a Tooltip where available. */
export function ThemeToggle({ className }: { className?: string }) {
  const theme = useTheme()
  const isDark = theme === 'dark'
  return (
    <button
      type="button"
      aria-label={isDark ? 'Switch to light mode' : 'Switch to dark mode'}
      onClick={toggleTheme}
      className={cn(
        'cursor-pointer rounded-lg p-2 text-graphite transition-colors hover:bg-paper-soft hover:text-ink',
        className,
      )}
    >
      {isDark ? <Sun className="size-4" /> : <Moon className="size-4" />}
    </button>
  )
}
