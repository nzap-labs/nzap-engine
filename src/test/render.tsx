import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import {
  createMemoryHistory,
  createRootRoute,
  createRouter,
  RouterProvider,
} from '@tanstack/react-router'
import { render } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactElement } from 'react'
import { Toaster } from 'sonner'
import { DialogsProvider } from '@/components/dialogs'
import { TooltipProvider } from '@/components/ui/tooltip'
import { installFakeEngine, type FakeState } from '@/dev/fake-engine'

/**
 * Render a component the way the app does (query client, router, dialogs,
 * tooltips) against the simulated engine, which answers every IPC command.
 * The router is in memory, so links render; the component is its only page.
 */
export function renderWithEngine(
  ui: ReactElement,
  state: Partial<FakeState> = {},
  prepare?: (state: FakeState) => void,
) {
  const engine = installFakeEngine()
  engine.reset({ delay: 1, ...state })
  prepare?.(engine.state)
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 }, mutations: { retry: false } },
  })
  const user = userEvent.setup()
  const router = createRouter({
    routeTree: createRootRoute({ component: () => ui }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  const result = render(
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>
        <DialogsProvider>
          <RouterProvider router={router} />
          <Toaster />
        </DialogsProvider>
      </TooltipProvider>
    </QueryClientProvider>,
  )
  return { ...result, user, engine, queryClient }
}
