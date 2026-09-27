import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { describe, expect, it } from 'vitest'
import { DialogsProvider, useDialogs } from './dialogs'

function Harness() {
  const dialogs = useDialogs()
  const [result, setResult] = useState('none')
  return (
    <>
      <button
        onClick={async () =>
          setResult(
            String(
              await dialogs.confirm({ title: 'Delete it?', confirmLabel: 'Delete', danger: true }),
            ),
          )
        }
      >
        ask
      </button>
      <button
        onClick={async () =>
          setResult(String(await dialogs.prompt({ title: 'Name it', defaultValue: 'untitled.py' })))
        }
      >
        prompt
      </button>
      <output>{result}</output>
    </>
  )
}

function setup() {
  const user = userEvent.setup()
  render(
    <DialogsProvider>
      <Harness />
    </DialogsProvider>,
  )
  return user
}

describe('DialogsProvider', () => {
  it('confirms and cancels', async () => {
    const user = setup()
    await user.click(screen.getByText('ask'))
    expect(screen.getByRole('dialog', { name: 'Delete it?' })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Delete' }))
    expect(screen.getByRole('status')).toHaveTextContent('true')

    await user.click(screen.getByText('ask'))
    await user.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(screen.getByRole('status')).toHaveTextContent('false')
  })

  it('prompts with a default value and submits on Enter', async () => {
    const user = setup()
    await user.click(screen.getByText('prompt'))
    const input = screen.getByRole('textbox')
    expect(input).toHaveValue('untitled.py')
    await user.clear(input)
    await user.type(input, 'train.py{Enter}')
    expect(screen.getByRole('status')).toHaveTextContent('train.py')

    await user.click(screen.getByText('prompt'))
    await user.keyboard('{Escape}')
    expect(screen.getByRole('status')).toHaveTextContent('null')
  })

  it('will not submit an empty prompt', async () => {
    const user = setup()
    await user.click(screen.getByText('prompt'))
    await user.clear(screen.getByRole('textbox'))
    expect(screen.getByRole('button', { name: 'OK' })).toBeDisabled()
  })
})
