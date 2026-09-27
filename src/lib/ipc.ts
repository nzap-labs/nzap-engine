import { Channel, invoke, type InvokeArgs, type InvokeOptions } from '@tauri-apps/api/core'

/** Stable failure kinds from the engine (`nzap_core::ErrorCode`). */
export type ErrorCode =
  | 'not_connected'
  | 'auth_expired'
  | 'auth'
  | 'not_found'
  | 'invalid_input'
  | 'too_many_runtimes'
  | 'quota'
  | 'colab'
  | 'runtime'
  | 'network'
  | 'io'
  | 'cancelled'
  | 'internal'

/** A failed engine command: `{ code, message, status? }` from Rust. */
export class EngineError extends Error {
  constructor(
    readonly code: ErrorCode,
    message: string,
    readonly status?: number,
  ) {
    super(message)
    this.name = 'EngineError'
  }
}

export function toEngineError(error: unknown): EngineError {
  if (error instanceof EngineError) return error
  if (error && typeof error === 'object' && 'code' in error && 'message' in error) {
    const payload = error as { code: ErrorCode; message: string; status?: number }
    return new EngineError(payload.code, payload.message, payload.status)
  }
  if (typeof error === 'string') return new EngineError('internal', error)
  return new EngineError('internal', error instanceof Error ? error.message : 'Unexpected error.')
}

/** The message to show for any error thrown by an engine call. */
export function errorMessage(error: unknown, fallback = 'Something went wrong.'): string {
  if (error instanceof Error && error.message) return error.message
  return fallback
}

/** Invoke an engine command, turning failures into {@link EngineError}. */
export async function call<T>(
  command: string,
  args?: InvokeArgs,
  options?: InvokeOptions,
): Promise<T> {
  try {
    return await invoke<T>(command, args, options)
  } catch (error) {
    throw toEngineError(error)
  }
}

export interface StreamHandlers<E> {
  onEvent: (event: E) => void
  signal?: AbortSignal
}

/**
 * Run a streaming command. Events arrive on a Tauri channel as they are
 * produced; aborting the signal cancels the work in the engine. Resolves with
 * the command's final result.
 */
export async function stream<E, R = unknown>(
  command: string,
  args: Record<string, unknown>,
  handlers: StreamHandlers<E>,
): Promise<R> {
  if (handlers.signal?.aborted) throw new EngineError('cancelled', 'Cancelled.')
  const streamId = crypto.randomUUID()
  const channel = new Channel<E>()
  channel.onmessage = (event) => handlers.onEvent(event)
  const onAbort = () => {
    void invoke('stream_cancel', { streamId }).catch(() => undefined)
  }
  handlers.signal?.addEventListener('abort', onAbort, { once: true })
  try {
    return await call<R>(command, { ...args, streamId, onEvent: channel })
  } finally {
    handlers.signal?.removeEventListener('abort', onAbort)
  }
}

/** Open an https link in the user's browser (validated by the engine). */
export function openExternal(url: string): Promise<void> {
  return call('open_url', { url })
}
