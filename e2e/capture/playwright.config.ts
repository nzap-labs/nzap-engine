import { defineConfig, devices } from '@playwright/test'

const PORT = 4181

/** Product screenshots (not CI): 1440×900 at 2× pixel density. */
export default defineConfig({
  testDir: '.',
  timeout: 120_000,
  reporter: 'list',
  use: {
    ...devices['Desktop Chrome'],
    baseURL: `http://127.0.0.1:${PORT}`,
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 2,
  },
  webServer: {
    command: `npx vite --port ${PORT} --strictPort --host 127.0.0.1`,
    url: `http://127.0.0.1:${PORT}`,
    reuseExistingServer: true,
    cwd: '../..',
  },
})
