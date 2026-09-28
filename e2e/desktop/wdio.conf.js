// Desktop E2E: the real app (debug build) and the real Rust engine, pointed
// at nzap-mock-colab instead of Google. Runs on Linux (WebKitWebDriver under
// xvfb) and Windows (msedgedriver); macOS has no WKWebView driver.
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '../..')
const exe = process.platform === 'win32' ? '.exe' : ''
const application = path.join(root, 'target', 'debug', `nzap-engine${exe}`)
const mockBinary = path.join(root, 'target', 'debug', `mock-colab${exe}`)
const MOCK_PORT = 9901

// Everything the app writes goes to a throwaway directory.
const workDir = fs.mkdtempSync(path.join(os.tmpdir(), 'nzap-e2e-'))
export const openLog = path.join(workDir, 'opened-urls.log')
Object.assign(process.env, {
  NZAP_MOCK_GOOGLE: `http://127.0.0.1:${MOCK_PORT}`,
  NZAP_DATA_DIR: path.join(workDir, 'app'),
  NZAP_NO_KEYCHAIN: '1',
  NZAP_E2E_OPEN_LOG: openLog,
})

let mock
let tauriDriver

function tauriDriverArgs() {
  const nativeDriver = process.env.NATIVE_DRIVER
  return nativeDriver ? ['--native-driver', nativeDriver] : []
}

export const config = {
  hostname: '127.0.0.1',
  port: 4444,
  specs: ['./specs/**/*.e2e.js'],
  maxInstances: 1,
  capabilities: [
    {
      maxInstances: 1,
      'tauri:options': { application },
      // tauri-driver speaks WebDriver Classic, not BiDi.
      'wdio:enforceWebDriverClassic': true,
    },
  ],
  logLevel: 'warn',
  reporters: ['spec'],
  framework: 'mocha',
  mochaOpts: { ui: 'bdd', timeout: 180_000 },
  waitforTimeout: 20_000,

  onPrepare: async () => {
    for (const binary of [application, mockBinary]) {
      if (!fs.existsSync(binary))
        throw new Error(`Build ${binary} first (see e2e/desktop/README.md).`)
    }
    mock = spawn(mockBinary, [String(MOCK_PORT)], { stdio: 'inherit' })
    await new Promise((resolve) => setTimeout(resolve, 1500))
  },

  beforeSession: () => {
    const driver =
      process.env.TAURI_DRIVER ?? path.join(os.homedir(), '.cargo', 'bin', `tauri-driver${exe}`)
    tauriDriver = spawn(driver, tauriDriverArgs(), {
      stdio: [null, process.stdout, process.stderr],
    })
  },

  // On failure, record what the webview actually shows.
  afterTest: async (test, _context, { passed }) => {
    if (passed) return
    const dir = path.join(import.meta.dirname, 'artifacts')
    fs.mkdirSync(dir, { recursive: true })
    const name = test.title.replace(/[^a-z0-9]+/gi, '-').toLowerCase()
    try {
      const page = await browser.execute(() => ({
        url: location.href,
        title: document.title,
        readyState: document.readyState,
        tauri: '__TAURI_INTERNALS__' in window,
        text: document.body?.innerText.slice(0, 1500) ?? '(no body)',
      }))
      console.log(`[diagnostics] ${test.title}: ${JSON.stringify(page, null, 2)}`)
      await browser.saveScreenshot(path.join(dir, `${name}.png`))
    } catch (error) {
      console.log(`[diagnostics] ${test.title}: could not inspect the page: ${error}`)
    }
  },

  afterSession: () => {
    tauriDriver?.kill()
  },

  onComplete: () => {
    mock?.kill()
    fs.rmSync(workDir, { recursive: true, force: true })
  },
}
