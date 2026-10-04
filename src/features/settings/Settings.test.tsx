import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { SETTINGS, BACKGROUND_STATUS } from '@/dev/fixtures'
import { StoreContext, type Store, type SettingsChange } from '@/app/store'
import { api } from '@/lib/ipc'
import { Settings } from './Settings'

vi.mock('@/lib/ipc', () => ({ api: {
  suggestedRoots: vi.fn(async () => []), developerRoots: vi.fn(async () => ['/home/test/Code', '/home/test/Projects']),
  chooseDeveloperRoot: vi.fn(async () => '/home/test/Other'), ignored: vi.fn(async () => []),
  about: vi.fn(async () => ({ version: 'test', platform: 'test', quarantine_root: '/drawer' })),
} }))
vi.mock('@/features/updates/UpdateProvider', () => ({ useUpdates: () => ({ snapshot: null }) }))
const saved = vi.fn()
function Fixture({ enabled = true }: { enabled?: boolean }) {
  const [settings, setSettings] = useState({ ...SETTINGS, include_developer_debris: enabled })
  const updateSettings = async (change: SettingsChange) => {
    const changes = typeof change === 'function' ? change(settings) : change
    saved(changes); setSettings({ ...settings, ...changes }); return true
  }
  return <StoreContext.Provider value={{ settings, updateSettings, say: vi.fn(), background: BACKGROUND_STATUS } as unknown as Store}>
    <Settings />
  </StoreContext.Provider>
}
let host: HTMLDivElement
let root: Root
beforeEach(() => { Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true }); host = document.createElement('div'); document.body.append(host); root = createRoot(host); saved.mockClear() })
afterEach(async () => { await act(async () => root.unmount()); host.remove() })
async function click(text: string) {
  const button = [...host.querySelectorAll('button')].find((b) => b.textContent?.trim() === text || b.getAttribute('aria-label') === text)
  expect(button).toBeDefined(); await act(async () => button!.click())
}
it('keeps developer controls hidden until enabled', async () => {
  await act(async () => root.render(<Fixture enabled={false} />))
  expect(host.textContent).not.toContain('Project folders')
  await click('Developer build artefacts')
  expect(host.textContent).toContain('Project folders')
})
it('persists the inactivity choice and adds/removes selected folders', async () => {
  await act(async () => root.render(<Fixture />))
  const group = host.querySelector('[aria-labelledby="Suggest build output after-label"]')!
  const thirty = [...group.querySelectorAll('button')].find((b) => b.textContent === '30 days')!
  await act(async () => thirty.click())
  expect(saved).toHaveBeenLastCalledWith({ developer_stale_days: 30 })
  await click('Add folder')
  expect(api.chooseDeveloperRoot).toHaveBeenCalled()
  expect(saved).toHaveBeenLastCalledWith({ developer_roots: ['/home/test/Code', '/home/test/Projects', '/home/test/Other'] })
  await act(async () => (host.querySelector('[aria-label="Remove /home/test/Other"]') as HTMLButtonElement).click())
  expect(saved).toHaveBeenLastCalledWith({ developer_roots: ['/home/test/Code', '/home/test/Projects'] })
  await click('Use common folders')
  expect(saved).toHaveBeenLastCalledWith({ developer_roots: [] })
})
