import type { ReactNode } from 'react'
import { cn } from '@/lib/cn'

interface Template {
  name: string
  tag: string
  art: ReactNode
}

const TEMPLATES: Template[] = [
  {
    name: 'Product launch',
    tag: 'Landing page',
    art: (
      <div className="flex h-full flex-col justify-between bg-ink p-5 text-paper">
        <span className="text-[10px] font-medium uppercase tracking-[0.22em] text-paper/50">
          Landing page
        </span>
        <p className="text-[24px] font-medium leading-[1.15] tracking-tight">
          BUILT BY DISCIPLINE. <span className="text-sunshine">FORGED IN IRON.</span>
        </p>
      </div>
    ),
  },
  {
    name: 'Stillwater',
    tag: 'Personal blog',
    art: (
      <div className="flex h-full flex-col justify-between bg-paper-soft p-5">
        <span className="text-[10px] font-medium uppercase tracking-[0.22em] text-graphite">
          Personal blog
        </span>
        <div>
          <p className="text-[30px] font-medium leading-none tracking-tight text-ink">Stillwater</p>
          <p className="mt-2 text-xs text-graphite">A quiet place for deep work</p>
        </div>
      </div>
    ),
  },
  {
    name: 'Atelier portfolio',
    tag: 'Portfolio',
    art: (
      <div className="flex h-full flex-col items-start justify-between bg-paper p-5">
        <span className="text-[10px] font-medium uppercase tracking-[0.22em] text-graphite">
          Portfolio
        </span>
        <p className="text-[26px] font-medium leading-[1.1] tracking-tight text-ink">
          ATELIER
          <br />
          <span className="text-graphite">HALBE</span>
        </p>
      </div>
    ),
  },
  {
    name: 'Solstice',
    tag: 'Event site',
    art: (
      <div className="flex h-full flex-col justify-between bg-sunshine p-5 text-on-sunshine">
        <span className="text-[10px] font-medium uppercase tracking-[0.22em] text-ink/60">
          Event site
        </span>
        <div>
          <p className="text-[28px] font-medium leading-none tracking-tight">SOLSTICE ’26</p>
          <p className="mt-2 text-xs font-medium">One day. Twelve talks. Zero fluff.</p>
        </div>
      </div>
    ),
  },
  {
    name: 'Pulseboard',
    tag: 'SaaS landing',
    art: (
      <div className="flex h-full flex-col justify-between bg-ink p-5 text-paper">
        <span className="text-[10px] font-medium uppercase tracking-[0.22em] text-paper/50">
          SaaS landing
        </span>
        <div>
          <p className="text-[26px] font-medium leading-none tracking-tight">Pulseboard</p>
          <p className="mt-2 text-xs text-paper/60">Know your metrics before the standup</p>
        </div>
      </div>
    ),
  },
  {
    name: 'Field Notes',
    tag: 'Knowledge base',
    art: (
      <div className="flex h-full flex-col justify-between bg-paper-soft p-5">
        <span className="text-[10px] font-medium uppercase tracking-[0.22em] text-graphite">
          Knowledge base
        </span>
        <div>
          <p className="text-[26px] font-medium leading-none tracking-tight text-ink">
            Field Notes
          </p>
          <p className="mt-2 text-xs text-graphite">Everything the team knows, in one place</p>
        </div>
      </div>
    ),
  },
]

/** Template gallery pinned under the chips row. */
export function TemplateCards({ className }: { className?: string }) {
  return (
    <div className={cn('w-full', className)}>
      <div className="scrollbar-thin flex snap-x snap-mandatory gap-4 overflow-x-auto pb-2">
        {TEMPLATES.map((template) => (
          <article
            key={template.name}
            className="w-[290px] shrink-0 snap-start overflow-hidden rounded-xl border border-ink bg-paper"
          >
            <div className="h-[150px] border-b border-ink">{template.art}</div>
            <div className="flex items-center justify-between px-4 py-2.5">
              <span className="text-sm font-medium">{template.name}</span>
              <span className="text-xs text-graphite">{template.tag}</span>
            </div>
          </article>
        ))}
      </div>
    </div>
  )
}
