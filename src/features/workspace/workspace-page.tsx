import { ArrowUpRight, Menu } from 'lucide-react'
import { Composer } from './composer'
import { ModelPicker } from './model-picker'
import { SidebarRevealButton } from '@/features/shell/dashboard-shell'
import { ExternalLink } from '@/components/external-link'
import { TaskChips } from './task-chips'
import { TemplateCards } from './template-cards'
import { SparkleIcon } from '@/components/logo'

function TopLink({ label, href }: { label: string; href: string }) {
  return (
    <ExternalLink
      href={href}
      className="inline-flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-sm font-medium text-ink transition-colors hover:bg-paper-soft"
    >
      {label}
      <ArrowUpRight className="size-3.5 text-graphite" />
    </ExternalLink>
  )
}

/** The chat workspace — NZAP's layout, ready for the agent features on the roadmap. */
export function WorkspacePage() {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-center justify-between gap-2 px-4 py-3">
        <div className="flex min-w-0 items-center gap-1">
          <SidebarRevealButton>
            <Menu className="size-4" />
          </SidebarRevealButton>
          <ModelPicker />
        </div>
        <div className="flex items-center gap-1">
          <TopLink label="Docs" href="https://github.com/nzap-labs/nzap-engine#readme" />
          <TopLink label="GitHub" href="https://github.com/nzap-labs/nzap-engine" />
        </div>
      </header>

      <div className="scrollbar-thin flex min-h-0 flex-1 flex-col items-center overflow-y-auto px-4 pb-8 md:px-6">
        <div className="relative flex w-full max-w-3xl flex-1 flex-col items-center justify-center py-8 min-h-[min-content]">
          <SparkleIcon
            aria-hidden
            className="pointer-events-none absolute -top-4 size-[300px] text-ink opacity-[0.05] md:size-[380px]"
          />
          <h1 className="relative text-center text-4xl font-medium tracking-tight text-ink md:text-6xl">
            What can I build for you?
          </h1>
          <p className="relative mt-4 text-center text-graphite">
            Talk to NZAP and explore the boundless creative world
          </p>
          <Composer className="relative z-10 mt-8 w-full" />
        </div>

        <TaskChips className="mt-4" />
        <TemplateCards className="mx-auto mt-7 max-w-6xl" />
      </div>
    </div>
  )
}
