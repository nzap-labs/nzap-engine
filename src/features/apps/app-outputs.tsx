import { useEffect, useRef, useState } from 'react'
import { Download, FileDown, Loader2, Pause, Play } from 'lucide-react'
import { toast } from 'sonner'
import { useDownloadFile } from '@/api/colab'
import { cn } from '@/lib/cn'
import type { AppOutputSlot } from '@/types/app'
import type { ResolvedOutput } from './store'
import { contentsPath, formatDuration } from './spec'

interface Segment {
  start: number
  end: number
  text: string
}

function segmentsOf(output: ResolvedOutput): Segment[] {
  const raw = output.meta?.segments
  if (!Array.isArray(raw)) return []
  return raw.filter(
    (segment): segment is Segment =>
      typeof segment === 'object' &&
      segment !== null &&
      typeof (segment as Segment).start === 'number' &&
      typeof (segment as Segment).end === 'number' &&
      typeof (segment as Segment).text === 'string',
  )
}

/** One output, rendered for its kind. */
export function AppOutputView({
  output,
  slot,
  runtime,
}: {
  output: ResolvedOutput
  slot?: AppOutputSlot
  runtime: string
}) {
  const label = slot?.label ?? output.id
  return (
    <figure className="space-y-2">
      <figcaption className="text-xs font-medium uppercase tracking-[0.14em] text-graphite">
        {label}
      </figcaption>
      {output.fetchError ? (
        <p className="rounded-2xl border border-coral px-4 py-3 text-sm text-coral">
          {output.fetchError}
        </p>
      ) : (
        <Body output={output} runtime={runtime} />
      )}
    </figure>
  )
}

function Body({ output, runtime }: { output: ResolvedOutput; runtime: string }) {
  switch (output.kind) {
    case 'audio':
      return output.url ? (
        <AudioPlayer output={output} runtime={runtime} />
      ) : (
        <Pending path={output.path} />
      )
    case 'image':
      return output.url ? (
        <div className="space-y-2">
          <img
            src={output.url}
            alt={String(output.meta?.prompt ?? 'Generated image')}
            className="max-h-[520px] w-full rounded-2xl border border-line object-contain"
          />
          <DownloadButton runtime={runtime} path={output.path} />
        </div>
      ) : (
        <Pending path={output.path} />
      )
    case 'video':
      return output.url ? (
        <div className="space-y-2">
          <video
            src={output.url}
            controls
            className="max-h-[520px] w-full rounded-2xl border border-line"
          />
          <DownloadButton runtime={runtime} path={output.path} />
        </div>
      ) : (
        <Pending path={output.path} />
      )
    case 'file':
      return (
        <div className="flex items-center justify-between gap-3 rounded-2xl border border-ink px-4 py-3 text-sm">
          <span className="flex min-w-0 items-center gap-2">
            <FileDown className="size-4 shrink-0 text-graphite" />
            <span className="truncate">
              {output.filename ?? output.path?.split('/').pop() ?? 'file'}
            </span>
          </span>
          <DownloadButton runtime={runtime} path={output.path} />
        </div>
      )
    case 'table':
      return <TableView columns={output.columns ?? []} rows={output.rows ?? []} />
    case 'json':
      return (
        <pre className="scrollbar-thin max-h-96 overflow-auto rounded-2xl border border-line bg-paper-soft p-4 font-mono text-xs leading-relaxed">
          {JSON.stringify(output.value ?? null, null, 2)}
        </pre>
      )
    case 'text':
    case 'markdown':
    default:
      return (
        <p className="whitespace-pre-wrap rounded-2xl border border-line bg-paper-soft p-4 text-sm leading-relaxed">
          {output.text ?? ''}
        </p>
      )
  }
}

function Pending({ path }: { path?: string }) {
  return (
    <p className="flex items-center gap-2 rounded-2xl border border-line px-4 py-3 text-sm text-graphite">
      <Loader2 className="size-4 animate-spin" /> Fetching {path?.split('/').pop() ?? 'result'}…
    </p>
  )
}

function DownloadButton({ runtime, path }: { runtime: string; path?: string }) {
  const download = useDownloadFile()
  if (!path) return null
  return (
    <button
      type="button"
      disabled={download.isPending}
      onClick={() =>
        download.mutate(
          { name: runtime, path: contentsPath(path) },
          {
            onSuccess: (saved) => saved && toast.success(`Saved ${saved}`),
            onError: (error) =>
              toast.error(
                error instanceof Error
                  ? `${error.message} The runtime may have been released.`
                  : 'Download failed.',
              ),
          },
        )
      }
      className="inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-3xl border border-ink px-3 text-xs font-medium hover:bg-paper-soft disabled:opacity-60"
    >
      {download.isPending ? (
        <Loader2 className="size-3.5 animate-spin" />
      ) : (
        <Download className="size-3.5" />
      )}
      Save
    </button>
  )
}

function TableView({
  columns,
  rows,
}: {
  columns: string[]
  rows: (string | number | boolean | null)[][]
}) {
  return (
    <div className="scrollbar-thin max-h-96 overflow-auto rounded-2xl border border-line">
      <table className="w-full text-left text-sm">
        <thead className="sticky top-0 bg-paper-soft">
          <tr>
            {columns.map((column) => (
              <th
                key={column}
                className="px-4 py-2.5 text-xs font-medium uppercase tracking-[0.1em] text-graphite"
              >
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => (
            <tr key={index} className="border-t border-line">
              {row.map((cell, column) => (
                <td
                  key={column}
                  className={cn('px-4 py-2.5', typeof cell === 'number' && 'tabular-nums')}
                >
                  {cell === null ? '—' : String(cell)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

const BARS = 96

/** Peak levels across the clip, for the waveform. */
async function peaksOf(url: string): Promise<number[]> {
  const buffer = await (await fetch(url)).arrayBuffer()
  const context = new OfflineAudioContext(1, 1, 44_100)
  const audio = await context.decodeAudioData(buffer)
  const data = audio.getChannelData(0)
  const size = Math.max(1, Math.floor(data.length / BARS))
  const peaks: number[] = []
  for (let bar = 0; bar < BARS; bar += 1) {
    let peak = 0
    for (let index = bar * size; index < Math.min(data.length, (bar + 1) * size); index += 1)
      peak = Math.max(peak, Math.abs(data[index]))
    peaks.push(peak)
  }
  const loudest = Math.max(...peaks, 1e-6)
  return peaks.map((peak) => Math.max(0.06, peak / loudest))
}

function AudioPlayer({ output, runtime }: { output: ResolvedOutput; runtime: string }) {
  const audioRef = useRef<HTMLAudioElement>(null)
  const [peaks, setPeaks] = useState<number[] | null>(null)
  const [playing, setPlaying] = useState(false)
  const [time, setTime] = useState(0)
  const [duration, setDuration] = useState(Number(output.meta?.duration ?? 0))
  const segments = segmentsOf(output)
  const url = output.url!

  useEffect(() => {
    let live = true
    peaksOf(url)
      .then((result) => live && setPeaks(result))
      .catch(() => live && setPeaks([]))
    return () => {
      live = false
    }
  }, [url])

  useEffect(() => {
    if (!playing) return
    let frame = 0
    const tick = () => {
      setTime(audioRef.current?.currentTime ?? 0)
      frame = requestAnimationFrame(tick)
    }
    frame = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(frame)
  }, [playing])

  const progress = duration > 0 ? time / duration : 0
  const seek = (seconds: number) => {
    if (!audioRef.current) return
    audioRef.current.currentTime = seconds
    setTime(seconds)
  }
  const toggle = () => {
    const audio = audioRef.current
    if (!audio) return
    if (audio.paused) void audio.play()
    else audio.pause()
  }
  const active = segments.findIndex((segment) => time >= segment.start && time < segment.end)

  return (
    <div className="space-y-3 rounded-2xl border border-ink p-4">
      <audio
        ref={audioRef}
        src={url}
        preload="auto"
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => {
          setPlaying(false)
          setTime(0)
        }}
        onLoadedMetadata={(event) => {
          const seconds = event.currentTarget.duration
          if (Number.isFinite(seconds)) setDuration(seconds)
        }}
        className="hidden"
      />
      <div className="flex items-center gap-4">
        <button
          type="button"
          onClick={toggle}
          aria-label={playing ? 'Pause' : 'Play'}
          className="grid size-12 shrink-0 cursor-pointer place-items-center rounded-full bg-sunshine text-on-sunshine transition-transform hover:scale-105 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink"
        >
          {playing ? <Pause className="size-5" /> : <Play className="ml-0.5 size-5" />}
        </button>
        <div
          role="slider"
          tabIndex={0}
          aria-label="Seek"
          aria-valuemin={0}
          aria-valuemax={Math.round(duration)}
          aria-valuenow={Math.round(time)}
          onKeyDown={(event) => {
            if (event.key === 'ArrowRight') seek(Math.min(duration, time + 5))
            if (event.key === 'ArrowLeft') seek(Math.max(0, time - 5))
          }}
          onClick={(event) => {
            const box = event.currentTarget.getBoundingClientRect()
            seek(((event.clientX - box.left) / box.width) * duration)
          }}
          className="flex h-14 min-w-0 flex-1 cursor-pointer items-center gap-[2px] focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-ink"
        >
          {(peaks ?? Array.from({ length: BARS }, () => 0.08)).map((peak, index, all) => (
            <span
              key={index}
              className={cn(
                'flex-1 rounded-full transition-colors',
                index / all.length < progress ? 'bg-ink' : 'bg-ink/25',
                !peaks && 'animate-pulse',
              )}
              style={{ height: `${Math.round(peak * 100)}%` }}
            />
          ))}
        </div>
      </div>
      <div className="flex items-center justify-between text-xs tabular-nums text-graphite">
        <span>
          {formatDuration(time)} / {formatDuration(duration)}
          {typeof output.meta?.sampleRate === 'number' &&
            ` · ${(output.meta.sampleRate / 1000).toFixed(0)} kHz`}
        </span>
        <DownloadButton runtime={runtime} path={output.path} />
      </div>
      {segments.length > 1 && (
        <ol className="space-y-1 border-t border-line pt-3">
          {segments.map((segment, index) => (
            <li key={index}>
              <button
                type="button"
                onClick={() => {
                  seek(segment.start)
                  void audioRef.current?.play()
                }}
                className={cn(
                  'flex w-full cursor-pointer gap-3 rounded-lg px-2 py-1.5 text-left text-sm transition-colors hover:bg-paper-soft',
                  index === active ? 'bg-paper-soft text-ink' : 'text-graphite',
                )}
              >
                <span className="w-10 shrink-0 tabular-nums">{formatDuration(segment.start)}</span>
                <span>{segment.text}</span>
              </button>
            </li>
          ))}
        </ol>
      )}
    </div>
  )
}
