// Launch the debug app with the E2E environment for a few seconds and report
// what it printed and whether it stayed up. Run before the WebDriver suite so
// a startup failure shows up as the app's own output instead of a driver
// timeout.
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '../..')
const exe = process.platform === 'win32' ? '.exe' : ''
const application = path.join(root, 'target', 'debug', `nzap-engine${exe}`)
const workDir = fs.mkdtempSync(path.join(os.tmpdir(), 'nzap-smoke-'))

const child = spawn(application, [], {
  env: {
    ...process.env,
    NZAP_MOCK_GOOGLE: 'http://127.0.0.1:9',
    NZAP_DATA_DIR: path.join(workDir, 'app'),
    NZAP_NO_KEYCHAIN: '1',
    RUST_BACKTRACE: '1',
  },
  stdio: ['ignore', 'pipe', 'pipe'],
})
let output = ''
child.stdout.on('data', (chunk) => (output += chunk))
child.stderr.on('data', (chunk) => (output += chunk))

let exited = null
child.on('exit', (code, signal) => (exited = { code, signal }))

await new Promise((resolve) => setTimeout(resolve, 15_000))
const crashed = exited
if (!exited) child.kill()
await new Promise((resolve) => setTimeout(resolve, 500))

console.log('----- app output -----')
console.log(output.trim() || '(nothing)')
console.log('----------------------')
if (crashed) {
  console.log(`The app exited during startup: ${JSON.stringify(crashed)}`)
  process.exit(1)
}
console.log('The app stayed up for 15 s.')
fs.rmSync(workDir, { recursive: true, force: true })
