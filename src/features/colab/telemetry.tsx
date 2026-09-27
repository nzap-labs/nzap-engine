import { HardDrive, MemoryStick, Microchip } from 'lucide-react'
import { useQuery } from '@tanstack/react-query'
import { colabResourcesQuery } from '@/api/colab'
import type { ColabResources } from '@/types/colab'

interface Meter {
  label: string
  icon: typeof MemoryStick
  usage?: number
  limit?: number
  percent?: number
}

/** RAM / disk / GPU bars for the active runtime. */
export function Telemetry({ sessionName }: { sessionName: string | null }) {
  const { data } = useQuery(colabResourcesQuery(sessionName))
  const resources = data?.resources as ColabResources | undefined
  if (!sessionName || !resources) return null

  const meters: Meter[] = [
    { label: 'RAM', icon: MemoryStick, ...pick(resources.ram) },
    { label: 'Disk', icon: HardDrive, ...pick(resources.disk) },
    { label: 'GPU', icon: Microchip, ...pick(resources.gpu) },
  ].filter((meter) => meter.percent !== undefined || meter.usage !== undefined)

  if (!meters.length) return null

  return (
    <div className="grid gap-3 sm:grid-cols-3">
      {meters.map((meter) => (
        <div key={meter.label} className="rounded-2xl border border-line bg-paper-soft p-3">
          <div className="flex items-center justify-between text-xs text-graphite">
            <span className="flex items-center gap-1.5">
              <meter.icon className="size-3.5" />
              {meter.label}
            </span>
            <span className="font-mono">{formatPercent(meter)}</span>
          </div>
          <div
            className="mt-2 h-1.5 overflow-hidden rounded-full bg-paper"
            role="progressbar"
            aria-valuenow={percentOf(meter) ?? 0}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-label={meter.label}
          >
            <div
              className="h-full rounded-full bg-ink"
              style={{ width: `${Math.min(100, percentOf(meter) ?? 0)}%` }}
            />
          </div>
          <p className="mt-1.5 text-[11px] text-graphite">{formatBytes(meter)}</p>
        </div>
      ))}
    </div>
  )
}

function pick(value: unknown): Pick<Meter, 'usage' | 'limit' | 'percent'> {
  if (!value || typeof value !== 'object') return {}
  const record = value as Record<string, unknown>
  const num = (key: string) =>
    typeof record[key] === 'number' ? (record[key] as number) : undefined
  return { usage: num('usage'), limit: num('limit'), percent: num('percent') }
}

function percentOf(meter: Meter): number | undefined {
  if (typeof meter.percent === 'number') return meter.percent
  if (typeof meter.usage === 'number' && typeof meter.limit === 'number' && meter.limit > 0) {
    return (meter.usage / meter.limit) * 100
  }
  return undefined
}

function formatPercent(meter: Meter): string {
  const percent = percentOf(meter)
  return percent === undefined ? '—' : `${percent.toFixed(0)}%`
}

function formatBytes(meter: Meter): string {
  if (typeof meter.usage !== 'number') return '—'
  const used = formatSize(meter.usage)
  return typeof meter.limit === 'number' ? `${used} of ${formatSize(meter.limit)}` : used
}

function formatSize(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`
}
