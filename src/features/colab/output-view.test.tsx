import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { Block, renderMimeBundle } from './output-view'

describe('renderMimeBundle', () => {
  it('prefers images, then HTML, then text', () => {
    expect(renderMimeBundle({ 'image/png': 'abc', 'text/plain': 'x' })).toEqual({
      image: { mime: 'image/png', data: 'abc' },
    })
    expect(renderMimeBundle({ 'image/jpeg': ['a', 'b'] })).toEqual({
      image: { mime: 'image/jpeg', data: 'ab' },
    })
    expect(renderMimeBundle({ 'text/html': ['<b>', 'hi</b>'], 'text/plain': 'hi' })).toEqual({
      html: '<b>hi</b>',
    })
    expect(renderMimeBundle({ 'image/svg+xml': '<svg/>' })).toEqual({ html: '<svg/>' })
    expect(renderMimeBundle({ 'text/plain': ['4', '2'] })).toEqual({ text: '42' })
    expect(renderMimeBundle(undefined)).toEqual({})
  })
})

describe('Block', () => {
  it('renders HTML output in a script-less sandbox', () => {
    const { container } = render(
      <Block block={{ id: 1, kind: 'result', html: '<script>alert(1)</script>' }} />,
    )
    const frame = container.querySelector('iframe')!
    expect(frame.getAttribute('sandbox')).toBe('')
    expect(frame.getAttribute('srcdoc')).toContain('<script>')
  })

  it('colours stderr and errors', () => {
    const { container } = render(
      <>
        <Block block={{ id: 1, kind: 'stream', streamName: 'stderr', text: 'warn' }} />
        <Block block={{ id: 2, kind: 'error', text: 'Traceback' }} />
      </>,
    )
    expect(container.querySelectorAll('.text-coral')).toHaveLength(2)
  })
})
