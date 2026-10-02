import { describe, expect, it } from 'vitest'
import bundled from '../../../crates/nzap-core/catalog/bundled.json'
import type { AppSpec } from '@/types/app'
import type { NotebookParam } from '@/types/notebook'
import {
  aboutDuration,
  appEventOf,
  appSpecOf,
  contentsPath,
  formSections,
  initialValues,
  reconcile,
  runtimeNameFor,
  runtimeRequest,
  runsOn,
  visibleOptions,
} from './spec'
import { estimateFor } from './store'

const kokoro = (
  bundled.notebooks as { slug: string; params: NotebookParam[]; app?: unknown }[]
).find((entry) => entry.slug === 'kokoro-tts')!
const spec = appSpecOf(kokoro)!

describe('app specs', () => {
  it('accepts the bundled Kokoro app and rejects anything else', () => {
    expect(spec.runtime.accelerator).toBe('T4')
    expect(appSpecOf({ app: undefined })).toBeNull()
    expect(appSpecOf({ app: { ...spec, format: 'nzap-app/2' } })).toBeNull()
    expect(appSpecOf({ app: { ...spec, outputs: [] } })).toBeNull()
    expect(appSpecOf({ app: { ...spec, estimates: { setup: -1, run: 1 } } })).toBeNull()
  })

  it('lays the form out in spec order with Advanced collapsed', () => {
    const params: NotebookParam[] = [
      { key: 'seed', label: 'Seed', type: 'integer' },
      { key: 'text', label: 'Text', type: 'text' },
      { key: 'loud', label: 'Loud', type: 'boolean' },
    ]
    const custom = {
      ...spec,
      inputs: [
        { param: 'text', widget: 'textarea' },
        { param: 'seed', widget: 'number', section: 'Advanced' },
        // A widget that cannot edit the type falls back to the default.
        { param: 'loud', widget: 'slider', min: 0, max: 1 },
      ],
    } as AppSpec
    const sections = formSections(custom, params)
    expect(sections.map((section) => section.title)).toEqual([null, 'Advanced'])
    expect(sections[0].fields.map((field) => [field.param.key, field.input.widget])).toEqual([
      ['text', 'textarea'],
      ['loud', 'switch'],
    ])
    expect(sections[1].collapsed).toBe(true)
  })

  it('filters voices by language and keeps the pick consistent', () => {
    const sections = formSections(spec, kokoro.params)
    const voice = sections[0].fields.find((field) => field.param.key === 'voice')!
    let values = initialValues(sections)
    expect(values.voice).toBe('af_heart')
    expect(visibleOptions(voice, values).every((option) => option.startsWith('a'))).toBe(true)

    values = reconcile(sections, { ...values, language: 'b' })
    expect(String(values.voice).startsWith('b')).toBe(true)
    expect(visibleOptions(voice, values)).toContain('bm_george')
  })
})

describe('runtimes', () => {
  it('maps accelerators to runtime requests', () => {
    expect(runtimeRequest('CPU', 'a')).toEqual({ name: 'a', highMem: false })
    expect(runtimeRequest('t4', 'a')).toEqual({ name: 'a', gpu: 'T4', highMem: false })
    expect(runtimeRequest('V5E1', 'a', true)).toEqual({ name: 'a', tpu: 'V5E1', highMem: true })
    expect(runsOn(spec, 'cpu')).toBe(true)
    expect(runsOn(spec, 'V6E1')).toBe(false)
    expect(runtimeNameFor('kokoro-tts', ['app-kokoro-tts'])).toBe('app-kokoro-tts-2')
  })

  it('estimates from your own runs first and skips setup when warm', () => {
    expect(estimateFor(spec, 'kokoro-tts', 'T4', {}, false)).toMatchObject({
      setup: spec.estimates.setup,
      run: spec.estimates.run,
      source: 'app',
    })
    const timings = { 'kokoro-tts': { T4: { setup: 40, run: 1.5, at: 0 } } }
    expect(estimateFor(spec, 'kokoro-tts', 'T4', timings, false)).toMatchObject({
      setup: 40,
      run: 1.5,
      source: 'yours',
    })
    expect(estimateFor(spec, 'kokoro-tts', 'T4', timings, true).setup).toBe(0)
    expect(aboutDuration(50)).toBe('~50s')
    expect(aboutDuration(150)).toBe('~2m 30s')
  })
})

describe('app events', () => {
  it('decodes events from display outputs, as objects or strings', () => {
    const payload = { v: 1, event: 'stage', id: 'load', label: 'Loading' }
    expect(
      appEventOf({ type: 'display', data: { 'application/vnd.nzap.app+json': payload } as never }),
    ).toMatchObject({ event: 'stage', id: 'load' })
    expect(
      appEventOf({
        type: 'display',
        data: { 'application/vnd.nzap.app+json': JSON.stringify({ event: 'done' }) },
      }),
    ).toMatchObject({ event: 'done' })
    expect(appEventOf({ type: 'display', data: { 'text/plain': 'hi' } })).toBeNull()
    expect(appEventOf({ type: 'stream', text: 'x' })).toBeNull()
    expect(contentsPath('/content/nzap/outputs/a.wav')).toBe('content/nzap/outputs/a.wav')
  })
})
