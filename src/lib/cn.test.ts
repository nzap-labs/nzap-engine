import { describe, expect, it } from 'vitest'
import { cn } from './cn'

describe('cn', () => {
  it('joins conditional classes', () => {
    const hidden = false
    expect(cn('a', hidden && 'b', 'c', undefined, null)).toBe('a c')
  })

  it('lets later Tailwind utilities win conflicts', () => {
    expect(cn('px-2 text-ink', 'px-4')).toBe('text-ink px-4')
  })
})
