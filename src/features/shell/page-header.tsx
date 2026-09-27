import { Menu } from 'lucide-react'
import type { ReactNode } from 'react'
import { SidebarRevealButton } from './dashboard-shell'

/** Consistent header for workspace sub-pages (profile, account, credits). */
export function PageHeader({ title, actions }: { title: string; actions?: ReactNode }) {
  return (
    <header className="flex items-center justify-between gap-2 px-4 py-3 md:px-6">
      <div className="flex min-w-0 items-center gap-1">
        <SidebarRevealButton>
          <Menu className="size-4" />
        </SidebarRevealButton>
        <h1 className="truncate px-1.5 text-lg font-medium tracking-tight">{title}</h1>
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </header>
  )
}
