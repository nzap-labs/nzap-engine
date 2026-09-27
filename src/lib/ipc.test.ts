import { mockIPC } from '@tauri-apps/api/mocks'
import type { Channel } from '@tauri-apps/api/core'
import { describe, expect, it } from 'vitest'
import { call, EngineError, errorMessage, stream, toEngineError } from './ipc'

describe('call', () => {
  it('returns the command result', async () => {
    mockIPC((cmd, args) => ({ cmd, args }))
    await expect(call('ping', { value: 1 })).resolves.toEqual({ cmd: 'ping', args: { value: 1 } })
  })

  it('turns engine failures into EngineError', async () => {
    mockIPC(() => {
      throw { code: 'too_many_runtimes', message: 'Release one first.', status: 412 }
    })
    const error = await call('session_create').catch((failure: unknown) => failure)
    expect(error).toBeInstanceOf(EngineError)
    expect(error).toMatchObject({
      code: 'too_many_runtimes',
      message: 'Release one first.',
      status: 412,
    })
  })
})

describe('toEngineError', () => {
  it('handles strings, errors and unknown values', () => {
    expect(toEngineError('boom')).toMatchObject({ code: 'internal', message: 'boom' })
    expect(toEngineError(new Error('x'))).toMatchObject({ code: 'internal', message: 'x' })
    expect(toEngineError(42).message).toBe('Unexpected error.')
    expect(errorMessage(undefined, 'fallback')).toBe('fallback')
  })
})

describe('stream', () => {
  it('delivers channel events and resolves with the result', async () => {
    let received: Record<string, unknown> = {}
    mockIPC((_cmd, args) => {
      received = args as Record<string, unknown>
      const channel = received.onEvent as Channel<unknown>
      channel.onmessage({ type: 'stream', text: 'a' })
      channel.onmessage({ type: 'stream', text: 'b' })
      return { type: 'execute_reply', status: 'ok' }
    })
    const events: unknown[] = []
    const result = await stream(
      'session_execute',
      { name: 'box' },
      { onEvent: (event) => events.push(event) },
    )
    expect(result).toEqual({ type: 'execute_reply', status: 'ok' })
    expect(events).toEqual([
      { type: 'stream', text: 'a' },
      { type: 'stream', text: 'b' },
    ])
    expect(received.name).toBe('box')
    expect(typeof received.streamId).toBe('string')
  })

  it('cancels the engine stream when aborted', async () => {
    const cancelled: unknown[] = []
    let finish: (value: unknown) => void = () => undefined
    mockIPC((cmd, args) => {
      if (cmd === 'stream_cancel') {
        cancelled.push((args as { streamId: string }).streamId)
        finish({ code: 'cancelled', message: 'Cancelled.' })
        return true
      }
      return new Promise((_, reject) => {
        finish = reject
      })
    })
    const controller = new AbortController()
    const running = stream(
      'session_execute',
      {},
      { onEvent: () => undefined, signal: controller.signal },
    )
    controller.abort()
    await expect(running).rejects.toMatchObject({ code: 'cancelled' })
    expect(cancelled).toHaveLength(1)
  })

  it('refuses to start when already aborted', async () => {
    mockIPC(() => null)
    const controller = new AbortController()
    controller.abort()
    await expect(
      stream('x', {}, { onEvent: () => undefined, signal: controller.signal }),
    ).rejects.toMatchObject({
      code: 'cancelled',
    })
  })
})
