import { useState } from 'react'
import { Check, ChevronDown } from 'lucide-react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'

interface Model {
  id: string
  name: string
  available: boolean
}

const MODELS: Model[] = [
  { id: 'nzap-1-flash', name: 'NZAP-1 Flash', available: true },
  { id: 'nzap-1-pro', name: 'NZAP-1 Pro', available: false },
  { id: 'nzap-1-max', name: 'NZAP-1 Max', available: false },
]

/** Model selector shown top-left of the workspace. */
export function ModelPicker() {
  const [selected, setSelected] = useState(MODELS[0]!)
  return (
    <DropdownMenu>
      <DropdownMenuTrigger className="flex cursor-pointer items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[15px] font-medium transition-colors hover:bg-paper-soft">
        {selected.name}
        <ChevronDown className="size-4 text-graphite" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        <DropdownMenuLabel>Model</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {MODELS.map((model) => (
          <DropdownMenuItem
            key={model.id}
            disabled={!model.available}
            onSelect={() => setSelected(model)}
            className="justify-between"
          >
            <span>{model.name}</span>
            {model.available ? (
              selected.id === model.id && <Check className="size-4" />
            ) : (
              <span className="rounded-full border border-line px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-graphite">
                Soon
              </span>
            )}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
