import { createContext, useCallback, useContext, useMemo, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'

/**
 * In-app replacements for `window.confirm` / `window.prompt`, which not every
 * desktop webview implements (and which ignore the NZAP design system).
 */

export interface ConfirmOptions {
  title: string
  description?: string
  confirmLabel?: string
  danger?: boolean
}

export interface PromptOptions {
  title: string
  description?: string
  label?: string
  defaultValue?: string
  placeholder?: string
  confirmLabel?: string
}

interface Dialogs {
  confirm: (options: ConfirmOptions) => Promise<boolean>
  prompt: (options: PromptOptions) => Promise<string | null>
}

type Request =
  | ({ kind: 'confirm'; resolve: (value: boolean) => void } & ConfirmOptions)
  | ({ kind: 'prompt'; resolve: (value: string | null) => void } & PromptOptions)

const DialogsContext = createContext<Dialogs | null>(null)

export function DialogsProvider({ children }: { children: ReactNode }) {
  const [request, setRequest] = useState<Request | null>(null)
  const [value, setValue] = useState('')
  const settled = useRef(false)

  const open = useCallback((next: Request) => {
    settled.current = false
    setValue(next.kind === 'prompt' ? (next.defaultValue ?? '') : '')
    setRequest(next)
  }, [])

  const dialogs = useMemo<Dialogs>(
    () => ({
      confirm: (options) =>
        new Promise<boolean>((resolve) => open({ kind: 'confirm', resolve, ...options })),
      prompt: (options) =>
        new Promise<string | null>((resolve) => open({ kind: 'prompt', resolve, ...options })),
    }),
    [open],
  )

  function finish(accepted: boolean) {
    if (!request || settled.current) return
    settled.current = true
    if (request.kind === 'confirm') request.resolve(accepted)
    else request.resolve(accepted ? value : null)
    setRequest(null)
  }

  return (
    <DialogsContext.Provider value={dialogs}>
      {children}
      <Dialog open={request !== null} onOpenChange={(isOpen) => !isOpen && finish(false)}>
        {request && (
          <DialogContent>
            <form
              onSubmit={(event) => {
                event.preventDefault()
                finish(true)
              }}
            >
              <DialogTitle>{request.title}</DialogTitle>
              {request.description && <DialogDescription>{request.description}</DialogDescription>}
              {request.kind === 'prompt' && (
                <label className="mt-4 block">
                  <span className="sr-only">{request.label ?? request.title}</span>
                  <Input
                    autoFocus
                    value={value}
                    placeholder={request.placeholder}
                    onChange={(event) => setValue(event.target.value)}
                  />
                </label>
              )}
              <div className="mt-5 flex justify-end gap-2">
                <Button variant="ghost" size="sm" onClick={() => finish(false)}>
                  Cancel
                </Button>
                <Button
                  type="submit"
                  size="sm"
                  variant={request.kind === 'confirm' && request.danger ? 'danger' : 'primary'}
                  disabled={request.kind === 'prompt' && !value.trim()}
                >
                  {request.confirmLabel ?? (request.kind === 'confirm' ? 'Confirm' : 'OK')}
                </Button>
              </div>
            </form>
          </DialogContent>
        )}
      </Dialog>
    </DialogsContext.Provider>
  )
}

export function useDialogs(): Dialogs {
  const dialogs = useContext(DialogsContext)
  if (!dialogs) throw new Error('useDialogs must be used inside <DialogsProvider>.')
  return dialogs
}
