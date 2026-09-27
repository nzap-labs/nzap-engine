import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Cpu, Loader2, Rocket } from 'lucide-react'
import { toast } from 'sonner'
import { colabConfigQuery, colabQuotaQuery, colabStatusQuery, useCreateSession } from '@/api/colab'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/cn'

type Hardware = 'cpu' | 'gpu' | 'tpu'

/**
 * Launch form for a new Colab runtime: CPU / GPU / TPU with the accelerator
 * picker colab-studio exposes (plus High-RAM where the shape applies).
 */
export function NewRuntimeCard({ onCreated }: { onCreated?: (name: string) => void }) {
  const { data: config } = useQuery(colabConfigQuery)
  const { data: status } = useQuery(colabStatusQuery)
  const { data: quota } = useQuery({ ...colabQuotaQuery, enabled: Boolean(status?.connected) })
  const create = useCreateSession()

  const [name, setName] = useState('')
  const [hardware, setHardware] = useState<Hardware>('cpu')
  const [accelerator, setAccelerator] = useState('')
  const [highMem, setHighMem] = useState(false)

  const accelerators =
    hardware === 'gpu' ? (config?.gpus ?? []) : hardware === 'tpu' ? (config?.tpus ?? []) : []
  const highMemOnly = config?.highMemOnly ?? []
  // Google reports which accelerators this account may request; the rest
  // would only fail at assignment time, so they are shown but disabled.
  const ineligible = new Set(quota?.ineligibleAccelerators ?? [])
  const isEligible = (option: string) => !ineligible.has(option.toUpperCase())
  const selectedAccelerator =
    accelerators.includes(accelerator) && isEligible(accelerator)
      ? accelerator
      : (accelerators.find(isEligible) ?? '')
  const shapeApplies =
    hardware !== 'cpu' && selectedAccelerator !== '' && !highMemOnly.includes(selectedAccelerator)

  const noEligibleAccelerator = hardware !== 'cpu' && selectedAccelerator === ''
  const disabled = !status?.connected || create.isPending || noEligibleAccelerator

  function launch() {
    create.mutate(
      {
        name: name.trim() || undefined,
        gpu: hardware === 'gpu' ? selectedAccelerator : undefined,
        tpu: hardware === 'tpu' ? selectedAccelerator : undefined,
        highMem: shapeApplies ? highMem : undefined,
      },
      {
        onSuccess: (result) => {
          toast.success(`Runtime ${result.session.name} is ready (${result.session.accelerator}).`)
          setName('')
          onCreated?.(result.session.name)
        },
        onError: (error) =>
          toast.error(error instanceof Error ? error.message : 'Could not launch a runtime.'),
      },
    )
  }

  return (
    <section aria-label="New runtime" className="rounded-[24px] border border-ink bg-paper p-6">
      <p className="font-medium">New runtime</p>
      <p className="mt-1 text-sm text-graphite">
        Allocating a free-tier GPU can take a minute — the request waits for Google.
      </p>

      <div className="mt-4 space-y-4">
        <label className="block">
          <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
            Name
          </span>
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="my-runtime"
            maxLength={48}
            className="mt-1.5 h-11 w-full rounded-2xl border border-ink bg-transparent px-4 text-sm outline-none placeholder:text-graphite/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
          />
        </label>

        <div>
          <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
            Hardware
          </span>
          <div className="mt-1.5 flex gap-2" role="radiogroup" aria-label="Hardware">
            {(['cpu', 'gpu', 'tpu'] as Hardware[]).map((option) => (
              <button
                key={option}
                type="button"
                role="radio"
                aria-checked={hardware === option}
                onClick={() => setHardware(option)}
                className={cn(
                  'h-9 cursor-pointer rounded-3xl border px-4 text-sm font-medium uppercase transition-colors',
                  hardware === option
                    ? 'border-ink bg-sunshine text-on-sunshine'
                    : 'border-ink text-ink hover:bg-paper-soft',
                )}
              >
                {option}
              </button>
            ))}
          </div>
        </div>

        {accelerators.length > 0 && (
          <label className="block">
            <span className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
              Accelerator
            </span>
            <select
              value={selectedAccelerator}
              onChange={(event) => setAccelerator(event.target.value)}
              className="mt-1.5 h-11 w-full rounded-2xl border border-ink bg-transparent px-3 text-sm outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
            >
              {accelerators.map((option) => (
                <option key={option} value={option} disabled={!isEligible(option)}>
                  {option.toUpperCase()}
                  {isEligible(option) ? '' : ' — not available on your plan'}
                </option>
              ))}
            </select>
          </label>
        )}

        {noEligibleAccelerator && (
          <p className="text-xs text-graphite">
            Your Colab plan has no {hardware.toUpperCase()} accelerators available right now.
          </p>
        )}

        {shapeApplies && (
          <label className="flex items-center gap-2.5 text-sm">
            <input
              type="checkbox"
              checked={highMem}
              onChange={(event) => setHighMem(event.target.checked)}
              className="size-4 accent-[var(--color-ink)]"
            />
            High-RAM shape
          </label>
        )}

        <Button onClick={launch} disabled={disabled} className="w-full">
          {create.isPending ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <Rocket className="size-4" />
          )}
          {create.isPending ? 'Allocating…' : 'Launch runtime'}
        </Button>
        {!status?.connected && (
          <p className="flex items-center gap-1.5 text-xs text-graphite">
            <Cpu className="size-3.5" /> Connect Google Auth first.
          </p>
        )}
      </div>
    </section>
  )
}
