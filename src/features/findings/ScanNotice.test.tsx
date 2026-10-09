import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { StoreContext, type Store } from '@/app/store'
import { EMPTY_FINDINGS } from '@/dev/fixtures'
import { Findings } from './Findings'
import { ScanNotice } from './ScanNotice'

const hiccups = { permission_denied: 12, vanished: 3, unreadable: 0, loops_avoided: 0 }
const go = vi.fn()
const rummage = vi.fn(async () => {})
let host: HTMLDivElement
let root: Root
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  go.mockClear()
  rummage.mockClear()
})
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks() })

async function render({ empty = false, busy = false, skipped = hiccups } = {}) {
  const store = { go, rummage, moving: busy, scan: { status: 'idle' }, findings: { ...EMPTY_FINDINGS, hiccups: skipped } } as unknown as Store
  await act(async () => root.render(
    <StoreContext.Provider value={store}>
      {empty ? <Findings /> : <ScanNotice hiccups={skipped} />}
    </StoreContext.Provider>,
  ))
}
async function click(label: string) {
  const button = [...host.querySelectorAll('button')].find((element) => element.textContent === label)
  expect(button).toBeDefined()
  await act(async () => button!.click())
}

it('explains permission and transient skips separately and wires up recovery actions', async () => {
  await render()
  expect(host.textContent).toContain('15 places skipped.')
  await click('What can I do?')
  expect(host.textContent).toContain('12 places need folder access')
  expect(host.textContent).toContain('3 places disappeared during the scan')
  expect(host.textContent).toContain('can’t list the affected paths')
  await click('Review scan folders →')
  expect(go).toHaveBeenCalledWith({ name: 'settings' })
  await click('Scan again')
  expect(rummage).toHaveBeenCalledOnce()
})

it('offers macOS folder access guidance without requesting broad permissions', async () => {
  vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Macintosh')
  await render()
  await click('What can I do?')
  expect(host.textContent).toContain('Privacy & Security → Files & Folders')
  expect(host.textContent).not.toContain('Full Disk Access')
})

it('does not claim a tidy machine when the empty result has skipped areas', async () => {
  await render({ empty: true })
  expect(host.textContent).toContain('Nothing found in the areas checked.')
  expect(host.textContent).not.toContain('suspiciously tidy')
})

it('does not start another scan while a move is in progress', async () => {
  await render({ busy: true })
  await click('What can I do?')
  const retry = [...host.querySelectorAll('button')].find((button) => button.textContent === 'Scan again')!
  expect(retry.disabled).toBe(true)
})

it('does not show a notice for a scan with no skips', async () => {
  await render({ skipped: { permission_denied: 0, vanished: 0, unreadable: 0, loops_avoided: 0 } })
  expect(host.textContent).toBe('')
})
