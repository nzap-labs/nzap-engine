import { describe, expect, it } from 'vitest'
import { parseDeepLink } from './deep-links'

describe('nzap:// links', () => {
  it('opens apps by id or slug, and the gallery', () => {
    expect(parseDeepLink('nzap://apps/public:kokoro-tts')).toEqual({
      to: '/apps/$appId',
      appId: 'public:kokoro-tts',
    })
    expect(parseDeepLink('nzap://apps/public%3Akokoro-tts')).toEqual({
      to: '/apps/$appId',
      appId: 'public:kokoro-tts',
    })
    expect(parseDeepLink('nzap://app/breeze-tts')).toEqual({
      to: '/apps/$appId',
      appId: 'public:breeze-tts',
    })
    expect(parseDeepLink(`nzap://apps/local:${'a'.repeat(32)}`)).toMatchObject({
      appId: `local:${'a'.repeat(32)}`,
    })
    expect(parseDeepLink('nzap://apps')).toEqual({ to: '/apps' })
    expect(parseDeepLink('nzap://apps/')).toEqual({ to: '/apps' })
  })

  it('ignores anything else', () => {
    for (const link of [
      'https://apps/public:kokoro-tts',
      'nzap://apps/public:Kokoro',
      'nzap://apps/public:kokoro-tts/run',
      'nzap://apps/../settings',
      'nzap://settings',
      'nzap://app/<script>',
      'not a url',
    ])
      expect(parseDeepLink(link)).toBeNull()
  })
})
