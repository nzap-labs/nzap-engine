import { useEffect, useRef, useState } from 'react'
import { ArrowUp, ChevronDown, Globe, Plus } from 'lucide-react'
import { toast } from 'sonner'
import { cn } from '@/lib/cn'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'

const EFFORT_MODES = ['Max', 'Balanced', 'Fast'] as const
type EffortMode = (typeof EFFORT_MODES)[number]

/** The main prompt composer: tools, Deep Think, effort mode, send. */
export function Composer({ className }: { className?: string }) {
  const [value, setValue] = useState('')
  const [deepThink, setDeepThink] = useState(false)
  const [webSearch, setWebSearch] = useState(false)
  const [mode, setMode] = useState<EffortMode>('Max')
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  // grow the textarea with its content, capped by max-h
  useEffect(() => {
    const el = textareaRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, 192)}px`
  }, [value])

  const canSend = value.trim().length > 0

  function handleSend() {
    if (!canSend) return
    toast.info(
      'AI runs land in a later phase — your workspace, credits, and auth are ready for them.',
    )
    setValue('')
  }

  return (
    <div className={cn('rounded-[24px] border border-ink bg-paper p-4 pb-3', className)}>
      <textarea
        ref={textareaRef}
        rows={2}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault()
            handleSend()
          }
        }}
        placeholder="How can I help you today?"
        aria-label="Prompt"
        className="block max-h-48 min-h-[52px] w-full resize-none bg-transparent text-[15px] leading-relaxed text-ink outline-none placeholder:text-graphite"
      />
      <div className="mt-2 flex items-center justify-between gap-2">
        <div className="flex items-center gap-1">
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                aria-label="Attach files"
                onClick={() => toast.info('Attachments arrive with the first AI runs.')}
                className="cursor-pointer rounded-full p-2 text-ink transition-colors hover:bg-paper-soft"
              >
                <Plus className="size-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent>Attach files — coming soon</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                aria-label="Web search"
                aria-pressed={webSearch}
                onClick={() => setWebSearch((on) => !on)}
                className={cn(
                  'cursor-pointer rounded-full p-2 transition-colors',
                  webSearch ? 'bg-ink text-paper' : 'text-ink hover:bg-paper-soft',
                )}
              >
                <Globe className="size-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent>Web search</TooltipContent>
          </Tooltip>
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            aria-pressed={deepThink}
            onClick={() => setDeepThink((on) => !on)}
            className={cn(
              'cursor-pointer rounded-full border px-3.5 py-1.5 text-sm font-medium transition-colors',
              deepThink
                ? 'border-ink bg-sunshine text-on-sunshine'
                : 'border-transparent text-ink hover:bg-paper-soft',
            )}
          >
            Deep Think
          </button>
          <DropdownMenu>
            <DropdownMenuTrigger className="flex cursor-pointer items-center gap-1 rounded-full border border-line bg-paper px-3 py-1.5 text-sm font-medium text-ink transition-colors hover:bg-paper-soft">
              {mode}
              <ChevronDown className="size-3.5 text-graphite" />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {EFFORT_MODES.map((effort) => (
                <DropdownMenuItem key={effort} onSelect={() => setMode(effort)}>
                  {effort}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          <button
            type="button"
            aria-label="Send"
            onClick={handleSend}
            disabled={!canSend}
            className={cn(
              'grid size-9 shrink-0 place-items-center rounded-full border transition-colors',
              canSend
                ? 'cursor-pointer border-ink bg-sunshine text-on-sunshine hover:brightness-95'
                : 'cursor-not-allowed border-transparent bg-ink/10 text-graphite',
            )}
          >
            <ArrowUp className="size-4" />
          </button>
        </div>
      </div>
    </div>
  )
}
