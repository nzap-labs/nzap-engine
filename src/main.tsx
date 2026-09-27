import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { RouterProvider } from '@tanstack/react-router'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import '@fontsource-variable/dm-sans'
// must stay before any route imports: registers the CSS cascade-layer order
import './styles/app.css'
import { createAppRouter } from './router'
import { initTheme } from './lib/theme'
import { BrowserNotice } from './components/browser-notice'

initTheme()

function isTauri(): boolean {
  return '__TAURI_INTERNALS__' in window
}

async function start() {
  const root = createRoot(document.getElementById('root')!)

  if (!isTauri()) {
    // A plain browser has no engine. Development and the web E2E suite run
    // against a simulated one; anything else explains how to get the app.
    if (import.meta.env.DEV || import.meta.env.VITE_FAKE_ENGINE === '1') {
      const { installFakeEngine } = await import('./dev/fake-engine')
      installFakeEngine()
    } else {
      root.render(<BrowserNotice />)
      return
    }
  }

  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: 1,
        refetchOnWindowFocus: false,
        staleTime: 30_000,
      },
    },
  })
  const router = createAppRouter(queryClient)

  root.render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    </StrictMode>,
  )
}

void start()
