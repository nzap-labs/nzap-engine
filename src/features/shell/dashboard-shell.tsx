import { createContext, useContext, useState } from 'react'
import type { ReactNode } from 'react'
import { Outlet } from '@tanstack/react-router'
import { TooltipProvider } from '@/components/ui/tooltip'
import { Sidebar } from './sidebar'
import { cn } from '@/lib/cn'

interface SidebarContextValue {
  open: boolean
  toggle: () => void
}

const SidebarContext = createContext<SidebarContextValue>({ open: true, toggle: () => {} })

export function useSidebar() {
  return useContext(SidebarContext)
}

/**
 * App shell: sidebar + scrollable main area. Pages render
 * their own header row (model picker / page title) inside the main area.
 *
 * The sidebar animates instead of unmounting: on desktop the wrapper's width
 * transitions (300px ↔ 0, clipping the aside); on mobile the aside is a fixed
 * drawer that slides via translate-x while the scrim fades.
 */
export function DashboardShell() {
  const [open, setOpen] = useState(true)
  const toggle = () => setOpen((current) => !current)

  return (
    <TooltipProvider>
      <SidebarContext.Provider value={{ open, toggle }}>
        <div className="flex h-dvh overflow-hidden bg-paper">
          <div
            aria-hidden
            onClick={() => setOpen(false)}
            className={cn(
              'fixed inset-0 z-30 bg-black/50 transition-opacity duration-300 lg:hidden',
              open ? 'opacity-100' : 'pointer-events-none opacity-0',
            )}
          />
          <div
            className={cn(
              'z-40 h-full shrink-0 overflow-hidden transition-[width] duration-300 ease-out',
              open ? 'w-0 lg:w-[300px]' : 'w-0',
            )}
          >
            <aside
              className={cn(
                'h-full w-[300px] transition-transform duration-300 ease-out',
                'fixed inset-y-0 left-0 lg:static',
                open ? 'translate-x-0' : '-translate-x-full lg:translate-x-0',
              )}
            >
              <Sidebar onClose={() => setOpen(false)} />
            </aside>
          </div>
          <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
            <Outlet />
          </main>
        </div>
      </SidebarContext.Provider>
    </TooltipProvider>
  )
}

/** Icon button that reopens the sidebar (rendered by pages when it's hidden). */
export function SidebarRevealButton({ children }: { children: ReactNode }) {
  const { open, toggle } = useSidebar()
  if (open) return null
  return (
    <button
      type="button"
      aria-label="Open sidebar"
      onClick={toggle}
      className="cursor-pointer rounded-lg p-2 text-graphite transition-colors hover:bg-paper-soft hover:text-ink"
    >
      {children}
    </button>
  )
}
