import { Cpu, FolderOpen, NotebookPen, SquareTerminal } from 'lucide-react'
import type { ReactNode } from 'react'
import { LogoMark } from '@/components/logo'

const STEPS: { icon: ReactNode; title: string; body: string }[] = [
  {
    icon: <Cpu className="size-4" />,
    title: 'Launch runtimes',
    body: 'CPU, T4, L4, A100, H100 or TPU — allocated on your own Colab account.',
  },
  {
    icon: <SquareTerminal className="size-4" />,
    title: 'Run code and shells',
    body: 'A streaming console, a real terminal, packages, Drive mounts and jobs.',
  },
  {
    icon: <NotebookPen className="size-4" />,
    title: 'Use notebooks',
    body: 'Run community notebooks from GitHub or your own, with typed parameters.',
  },
  {
    icon: <FolderOpen className="size-4" />,
    title: 'Stay in control',
    body: 'No NZAP account and no server: your token never leaves this computer.',
  },
]

/** First-launch explainer shown until a Google account is connected. */
export function Onboarding() {
  return (
    <section
      aria-label="Welcome"
      className="relative overflow-hidden rounded-[24px] border border-line bg-paper-soft p-6 md:p-8"
    >
      <LogoMark className="pointer-events-none absolute -right-12 -top-10 size-64 opacity-[0.07]" />
      <LogoMark className="size-12" />
      <p className="chrome-text mt-5 text-3xl font-medium tracking-tight">
        Your Colab runtimes, from your desktop.
      </p>
      <p className="mt-2 max-w-xl text-sm leading-relaxed text-graphite">
        Connect the Google account you use for Colab above. That is the only sign-in NZAP Engine
        needs.
      </p>
      <ul className="mt-6 grid gap-3 sm:grid-cols-2">
        {STEPS.map((step) => (
          <li key={step.title} className="rounded-2xl border border-line bg-paper p-4">
            <p className="flex items-center gap-2 text-sm font-medium">
              <span className="text-graphite">{step.icon}</span>
              {step.title}
            </p>
            <p className="mt-1 text-xs leading-relaxed text-graphite">{step.body}</p>
          </li>
        ))}
      </ul>
    </section>
  )
}
