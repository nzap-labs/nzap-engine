import { createRootRouteWithContext, Link } from '@tanstack/react-router'
import { Toaster } from 'sonner'
import { LogoMark } from '@/components/logo'
import { useTheme } from '@/components/theme-toggle'
import { DialogsProvider } from '@/components/dialogs'
import { DashboardShell } from '@/features/shell/dashboard-shell'
import type { RouterContext } from '@/router'

export const Route = createRootRouteWithContext<RouterContext>()({
  component: RootDocument,
  notFoundComponent: NotFound,
  errorComponent: RootError,
})

function RootDocument() {
  const theme = useTheme()
  return (
    <DialogsProvider>
      <DashboardShell />
      <Toaster position="top-center" theme={theme} />
    </DialogsProvider>
  )
}

function NotFound() {
  return (
    <div className="flex min-h-full flex-1 flex-col items-center justify-center gap-5 p-6">
      <LogoMark className="size-12" />
      <p className="text-2xl font-medium tracking-tight">Page not found</p>
      <p className="text-graphite">That page does not exist in NZAP Engine.</p>
      <Link
        to="/colab"
        className="rounded-3xl border border-ink px-6 py-3 font-medium transition-colors hover:bg-paper-soft"
      >
        Back to Colab
      </Link>
    </div>
  )
}

function RootError({ error }: { error: unknown }) {
  return (
    <div className="flex min-h-dvh flex-col items-center justify-center gap-5 px-6 text-center">
      <LogoMark className="size-12" />
      <p className="text-2xl font-medium tracking-tight">Something went wrong</p>
      <p className="max-w-md text-graphite">
        {error instanceof Error ? error.message : 'An unexpected error occurred.'}
      </p>
      <Link
        to="/colab"
        className="rounded-3xl border border-ink px-6 py-3 font-medium transition-colors hover:bg-paper-soft"
      >
        Back to Colab
      </Link>
    </div>
  )
}
