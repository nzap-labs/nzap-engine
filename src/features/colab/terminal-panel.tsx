import { useEffect, useRef, useState } from 'react'
import { Terminal as TerminalIcon, RotateCcw } from 'lucide-react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { openTerminal, type TerminalConnection } from '@/api/colab'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/cn'

type State = 'connecting' | 'open' | 'closed'

/**
 * A real shell on the runtime — VS Code's `colab.openTerminal` / the CLI's
 * `colab console`. Keystrokes and resizes travel as the upstream
 * `{data}` / `{cols, rows}` frames through the engine to the VM's
 * `/colab/tty`.
 */
export function TerminalPanel({ sessionName }: { sessionName: string | null }) {
  const host = useRef<HTMLDivElement>(null)
  const [state, setState] = useState<State>('connecting')
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    if (!sessionName || !host.current) return
    const term = new Terminal({
      cursorBlink: true,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
      fontSize: 13,
      theme: { background: '#141414', foreground: '#f2efe8', cursor: '#f2efe8' },
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(host.current)
    fit.fit()

    let connection: TerminalConnection | null = null
    let disposed = false
    const send = (frame: object) => connection?.send(frame)

    setState('connecting')
    term.writeln(`Connecting to ${sessionName}…`)
    openTerminal(sessionName, {
      onData: (data) => {
        if (!disposed) term.write(data)
      },
      onClose: (reason) => {
        if (disposed) return
        setState('closed')
        term.writeln(`\r\n\x1b[2m[${reason ? `disconnected: ${reason}` : 'disconnected'}]\x1b[0m`)
      },
    })
      .then((opened) => {
        if (disposed) {
          opened.close()
          return
        }
        connection = opened
        setState('open')
        send({ cols: term.cols, rows: term.rows })
        term.focus()
      })
      .catch((error: unknown) => {
        setState('closed')
        term.writeln(
          `\r\n${error instanceof Error ? error.message : 'Could not open the terminal.'}`,
        )
      })

    const input = term.onData((data) => send({ data }))
    const resize = term.onResize(({ cols, rows }) => send({ cols, rows }))
    const observer = new ResizeObserver(() => {
      try {
        fit.fit()
      } catch {
        // Hidden or zero-sized container.
      }
    })
    observer.observe(host.current)

    return () => {
      disposed = true
      observer.disconnect()
      input.dispose()
      resize.dispose()
      connection?.close()
      term.dispose()
    }
  }, [sessionName, attempt])

  if (!sessionName) {
    return (
      <section className="rounded-[24px] border border-line bg-paper p-8 text-center">
        <TerminalIcon className="mx-auto size-6 text-graphite" />
        <p className="mt-3 text-sm text-graphite">Select a runtime to open a terminal on it.</p>
      </section>
    )
  }

  return (
    <section aria-label="Terminal" className="rounded-[24px] border border-ink bg-paper p-4 md:p-6">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <span
            aria-hidden
            className={cn(
              'size-2 rounded-full',
              state === 'open'
                ? 'bg-mint'
                : state === 'connecting'
                  ? 'bg-sunshine'
                  : 'bg-graphite/40',
            )}
          />
          <p className="font-medium">Terminal</p>
          <span className="truncate text-xs text-graphite">{sessionName}</span>
        </div>
        <Button
          variant="secondary"
          size="sm"
          disabled={state === 'connecting'}
          onClick={() => setAttempt((value) => value + 1)}
        >
          <RotateCcw className="size-4" /> Reconnect
        </Button>
      </div>
      <div
        ref={host}
        className="h-[28rem] overflow-hidden rounded-2xl bg-[#141414] p-2"
        onClick={(event) =>
          (event.currentTarget.querySelector('textarea') as HTMLElement | null)?.focus()
        }
      />
    </section>
  )
}
