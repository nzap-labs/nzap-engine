// The real NZAP Engine end to end: Google sign-in through the loopback
// redirect, a runtime, a cell, a notebook, the terminal and disconnecting —
// every call going through the Rust engine to nzap-mock-colab.
import fs from 'node:fs'
import { openLog } from '../wdio.conf.js'

/** The last URL the app asked the system browser to open. */
async function nextOpenedUrl(previous = 0) {
  let urls = []
  await browser.waitUntil(
    () => {
      try {
        urls = fs.readFileSync(openLog, 'utf8').trim().split('\n').filter(Boolean)
      } catch {
        urls = []
      }
      return urls.length > previous
    },
    { timeout: 20_000, timeoutMsg: 'the app never opened a browser URL' },
  )
  return { url: urls.at(-1), count: urls.length }
}

/**
 * Wait until the element's rendered text contains `expected` (a string,
 * compared case-insensitively because CSS may uppercase it, or a RegExp).
 * The element is looked up again on every poll, so it may appear late.
 * Reads `innerText` in the page: WebKitWebDriver's getText drops some
 * visible rows and joins block elements without line breaks.
 */
async function waitForText(selector, expected) {
  let last = ''
  await browser
    .waitUntil(
      async () => {
        const text = await browser.execute(
          (css) => document.querySelector(css)?.innerText ?? null,
          selector,
        )
        if (text === null) return false
        last = text
        return expected instanceof RegExp
          ? expected.test(last)
          : last.toLowerCase().includes(expected.toLowerCase())
      },
      {
        timeout: 30_000,
        timeoutMsg: `${expected} never appeared in ${selector}`,
      },
    )
    .catch((error) => {
      throw new Error(`${error.message}; last text: ${JSON.stringify(last.slice(0, 500))}`)
    })
}

/** A button inside a container, matched by its (partial) text. */
async function buttonIn(container, text) {
  const button = await (await $(container)).$(`button*=${text}`)
  await button.waitForClickable()
  return button
}

/** Click a workspace tab once nothing covers it (a closing dialog, a toast). */
async function openTab(name) {
  const tab = await $(`button[role="tab"]*=${name}`)
  await tab.waitForClickable({ timeout: 10_000 })
  await tab.click()
}

describe('NZAP Engine (real engine, mock Google)', () => {
  it('starts on the onboarding screen', async () => {
    await (await $('section[aria-label="Welcome"]')).waitForDisplayed()
    await waitForText('section[aria-label="Google Auth"]', 'not connected')
  })

  it('connects Google through the loopback redirect', async () => {
    await (await $('button*=Connect Google')).click()
    const { url } = await nextOpenedUrl()
    if (!url.includes('/o/oauth2/v2/auth') || !url.includes('code_challenge_method=S256')) {
      throw new Error(`unexpected consent URL: ${url}`)
    }
    // Play the browser: the mock consents and redirects to the app's
    // one-shot loopback listener, which completes the PKCE exchange.
    const response = await fetch(url, { redirect: 'follow' })
    if (response.status !== 200 || !(await response.text()).includes('Google connected')) {
      throw new Error(`loopback page answered ${response.status}`)
    }
    await waitForText('section[aria-label="Google Auth"]', 'Connected as ada@example.com')
  })

  it('launches a runtime and streams a cell', async () => {
    await (await $('input[placeholder="my-runtime"]')).setValue('desktop-box')
    await (await $('button*=Launch runtime')).click()
    await (await $('section[aria-label="Console"]')).waitForDisplayed({ timeout: 60_000 })

    const editor = await $('section[aria-label="Console"] textarea')
    await editor.clearValue()
    await editor.setValue('print("hello from the desktop app")')
    await (await $('button*=Run cell')).click()
    await waitForText('section[aria-label="Console"]', 'hello from the desktop app')
    await waitForText('section[aria-label="Console"]', '✓ finished')
  })

  it('answers an input() prompt', async () => {
    const editor = await $('section[aria-label="Console"] textarea')
    await editor.clearValue()
    await editor.setValue('name = input("Name? ")')
    await (await $('button*=Run cell')).click()
    const answer = await $('input[placeholder="type your answer and press Enter"]')
    await answer.waitForDisplayed()
    await answer.setValue('Ada')
    await browser.keys('Enter')
    await waitForText('section[aria-label="Console"]', 'Hello, Ada!')
  })

  it('runs a public notebook with a parameter', async () => {
    await openTab('Notebooks')
    const card = await $('li*=Print Notebook')
    await (await card.$('button*=Run')).click()
    const dialog = await $('div[role="dialog"]')
    const field = await dialog.$('input')
    await field.clearValue()
    await field.setValue('Hi from a notebook')
    await (await dialog.$('button=Run')).click()
    await waitForText('div[role="dialog"]', 'Hi from a notebook')
    // Close it with its button and wait until it is gone: its overlay
    // would otherwise intercept the next test's first click.
    await (await dialog.$('button[aria-label="Close"]')).click()
    await dialog.waitForExist({ reverse: true, timeout: 10_000 })
  })

  it('opens a shell on the runtime', async () => {
    await openTab('Terminal')
    const terminal = await $('section[aria-label="Terminal"] .xterm')
    await terminal.waitForDisplayed()
    await waitForText('section[aria-label="Terminal"] .xterm-rows', 'root@mock:/content#')
    await terminal.click()
    await browser.keys(['w', 'h', 'o', 'a', 'm', 'i', 'Enter'])
    await waitForText('section[aria-label="Terminal"] .xterm-rows', /whoami\s+root\s/)
  })

  it('browses the runtime files', async () => {
    await openTab('Files')
    await waitForText('section[aria-label="Files"]', 'sample_data')
  })

  it('releases the runtime and disconnects', async () => {
    await openTab('Runtimes')
    await (await buttonIn('section[aria-label="Runtimes"]', 'Stop')).click()
    await (await buttonIn('div[role="dialog"]', 'Stop and release')).click()
    await waitForText('section[aria-label="Runtimes"]', 'No runtimes yet')

    await (await buttonIn('section[aria-label="Google Auth"]', 'Disconnect')).click()
    const dialog = await $('div[role="dialog"]')
    await (await dialog.$('button=Disconnect')).click()
    await waitForText('section[aria-label="Google Auth"]', 'not connected')
  })
})
