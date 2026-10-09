import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it } from 'vitest'
import { Preview } from './Preview'
import { installPreviewApi } from './preview-api'

let host: HTMLDivElement
let root: Root
let restoreApi: () => unknown
beforeEach(() => {
  restoreApi = installPreviewApi()
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})
afterEach(async () => { await act(async () => root.unmount()); host.remove(); restoreApi() })

async function click(label: string) {
  const button = [...host.querySelectorAll('button')].find((element) =>
    element.textContent?.trim() === label || element.getAttribute('aria-label')?.startsWith(label),
  )
  expect(button).toBeDefined()
  await act(async () => button!.click())
}

it('opens every category and resets selection when navigating to another pile', async () => {
  await act(async () => root.render(<Preview />))
  await click('Rummage')
  await click('Installers —')
  expect(host.querySelector('main h2')?.textContent).toBe('Installers')
  const first = host.querySelector('main input[type="checkbox"]') as HTMLInputElement
  await act(async () => first.click())
  expect(host.querySelector('main')?.textContent).toContain('1 of 2 picked')
  // These scene buttons change the category without unmounting the App.
  await click('Screenshots pile')
  expect(host.querySelector('main h2')?.textContent).toBe('Screenshots')
  expect(host.querySelector('main')?.textContent).toContain('nothing picked yet')
  expect((host.querySelector('main input[type="checkbox"]') as HTMLInputElement).checked).toBe(false)
  expect(host.querySelector('nav [aria-current="page"]')?.textContent).toBe('Findings')
})

it('lets the scan help open sample folders and change which ones are included', async () => {
  await act(async () => root.render(<Preview />))
  await click('Rummage')
  await click('What can I do?')
  await click('Review scan folders →')
  expect(host.textContent).not.toContain('Could not load scan folders')
  const downloads = [...host.querySelectorAll('[role="checkbox"]')].find((element) => element.textContent?.includes('Downloads'))!
  expect(downloads.getAttribute('aria-checked')).toBe('true')
  await act(async () => (downloads as HTMLButtonElement).click())
  expect(downloads.getAttribute('aria-checked')).toBe('false')
})
