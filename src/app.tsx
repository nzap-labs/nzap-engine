import { LogoWordmark } from '@/components/logo'

/**
 * Foundation placeholder. The NZAP shell and the Colab workspace are ported
 * from legacy-nzap in Phase 6 (see PLAN.md).
 */
export function App() {
  return (
    <main className="flex min-h-dvh flex-col items-center justify-center gap-4 bg-paper px-6 text-center">
      <LogoWordmark />
      <h1 className="text-3xl font-medium tracking-tight">NZAP Engine</h1>
      <p className="max-w-md text-graphite">
        Your own Google Colab runtimes, from a native window.
      </p>
    </main>
  )
}
