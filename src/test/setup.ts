import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { clearMocks } from '@tauri-apps/api/mocks'
import { afterEach } from 'vitest'

// jsdom does not implement scrolling.
Element.prototype.scrollTo ??= function scrollTo() {}

afterEach(() => {
  cleanup()
  clearMocks()
  delete window.__NZAP_FAKE__
})
