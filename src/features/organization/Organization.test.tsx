import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { api } from '@/lib/ipc'
import { Organization, OrganizationHistory, OrganizationOffers } from './Organization'
import type { OrganizationBatch, OrganizationInventory, OrganizationPlan } from './types'

const go = vi.fn()
const refreshFindings = vi.fn(async () => true)
vi.mock('@/app/store', () => ({ useStore: () => ({ go, refreshFindings, findings: null }) }))
vi.mock('@/lib/ipc', () => ({ api: {
  organizationInventory: vi.fn(), organizationPreference: vi.fn(), chooseOrganizationDestination: vi.fn(),
  planOrganization: vi.fn(), startOrganization: vi.fn(), undoOrganization: vi.fn(), organizationStatus: vi.fn(),
  organizationHistory: vi.fn(), cancelOrganization: vi.fn(), organizationThumbnail: vi.fn(async () => 'data:image/png;base64,cHJldmlldw=='), revealOrganization: vi.fn(),
} }))
const inventory: OrganizationInventory = { partial: false, skipped_locations: 0, items: [
  { id: 'shot', kind: 'screenshots', root: '/Desktop', path: '/Desktop/Screenshot.png', identity: { size: 5 } },
  { id: 'installer', kind: 'installers', root: '/Downloads', path: '/Downloads/AppSetup.dmg', identity: { size: 600000 } },
] }
const preference = { destination: { id: 'dest', path: '/Pictures/Screenshots' }, grouping: 'month' as const }
const plan: OrganizationPlan = { id: 'plan', kind: 'screenshots', preference, items: [{ opportunity: inventory.items[0]!, destination: '/Pictures/Screenshots/2026-10/Screenshot.png', note: null, cross_volume: false }] }
const batch: OrganizationBatch = { id: 'batch', created_unix: 1, kind: 'screenshots', preference, running: false, undoing: false, revision: 1, error: null, items: [{ file: plan.items[0]!, status: 'moved', note: null }] }
let host: HTMLDivElement
let root: Root
beforeEach(() => {
  vi.clearAllMocks(); Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  host = document.createElement('div'); document.body.append(host); root = createRoot(host)
  vi.mocked(api.organizationInventory).mockResolvedValue(inventory)
  vi.mocked(api.organizationPreference).mockResolvedValue(preference)
  vi.mocked(api.planOrganization).mockResolvedValue(plan)
  vi.mocked(api.startOrganization).mockResolvedValue(batch)
  vi.mocked(api.organizationHistory).mockResolvedValue([batch])
  vi.mocked(api.undoOrganization).mockResolvedValue({ ...batch, undoing: true, revision: 2, items: [{ ...batch.items[0]!, status: 'undone' }] })
})
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers() })
async function click(label: string) {
  const button = [...host.querySelectorAll('button')].find((b) => b.textContent?.trim() === label)
  expect(button, label).toBeDefined(); await act(async () => button!.click())
}
it('offers organization separately from cleanup and keeps history available', async () => {
  await act(async () => root.render(<OrganizationOffers />))
  expect(host.textContent).toContain('Gather 1 screenshot'); expect(host.textContent).toContain('Give this installer a home')
  await click('Organization history'); expect(go).toHaveBeenCalledWith({ name: 'organization_history' })
  expect(api.startOrganization).not.toHaveBeenCalled()
})
it('shows thumbnails and exact paths, and moves only after explicit review and confirmation', async () => {
  await act(async () => root.render(<Organization kind="screenshots" />))
  expect(host.querySelector('img')?.getAttribute('alt')).toBe('Preview of Screenshot.png')
  expect(api.planOrganization).not.toHaveBeenCalled(); expect(api.startOrganization).not.toHaveBeenCalled()
  await click('Review organization')
  expect(api.planOrganization).toHaveBeenCalledWith('screenshots', ['shot'], 'dest', 'month')
  expect(host.textContent).toContain('/Pictures/Screenshots/2026-10/Screenshot.png')
  expect(host.querySelector('h1')).toBe(document.activeElement)
  expect(api.startOrganization).not.toHaveBeenCalled()
  await click('Organize 1 screenshot'); expect(api.startOrganization).toHaveBeenCalledWith('plan')
  expect(host.textContent).toContain('1 screenshot organized'); expect(host.textContent).not.toContain('reclaimed')
})
it('changing folder invalidates the review and uses the registered destination id', async () => {
  vi.mocked(api.chooseOrganizationDestination).mockResolvedValue({ id: 'custom', path: '/Pictures/Collected' })
  await act(async () => root.render(<Organization kind="screenshots" />)); await click('Review organization'); await click('Change folder')
  expect(host.textContent).not.toContain('Organize 1 screenshot'); expect(host.textContent).toContain('/Pictures/Collected')
  await click('Review organization'); expect(api.planOrganization).toHaveBeenLastCalledWith('screenshots', ['shot'], 'custom', 'month')
})
it('selection changes invalidate a plan and prevent an empty move', async () => {
  await act(async () => root.render(<Organization kind="screenshots" />)); await click('Review organization')
  const checkbox = host.querySelector('input[aria-label="Organize Screenshot.png"]') as HTMLInputElement
  await act(async () => checkbox.click())
  expect(host.textContent).not.toContain('Organize 1 screenshot')
  expect([...host.querySelectorAll('button')].find((b) => b.textContent === 'Review organization')?.disabled).toBe(true)
})
it('history survives a new screen and provides undo and per-file outcomes', async () => {
  await act(async () => root.render(<OrganizationHistory />))
  expect(host.textContent).toContain('1 screenshot organized'); await click('Undo')
  expect(api.undoOrganization).toHaveBeenCalledWith('batch'); expect(host.textContent).toContain('1 file put back')
  expect(host.textContent).toContain('/Desktop/Screenshot.png')
})
it('reports a failed plan without enabling execution', async () => {
  vi.mocked(api.planOrganization).mockRejectedValue({ message: 'Destination unavailable.' })
  await act(async () => root.render(<Organization kind="screenshots" />)); await click('Review organization')
  expect(host.querySelector('[role="alert"]')?.textContent).toBe('Destination unavailable.')
  expect(api.startOrganization).not.toHaveBeenCalled()
})
it('allows choosing a destination when the default is protected or unavailable', async () => {
  vi.mocked(api.organizationPreference).mockRejectedValue({ message: 'Choose another folder.' })
  vi.mocked(api.chooseOrganizationDestination).mockResolvedValue({ id: 'custom', path: '/Pictures/Collected' })
  await act(async () => root.render(<Organization kind="screenshots" />)); await click('Choose an available folder')
  await click('Review organization'); expect(api.planOrganization).toHaveBeenCalledWith('screenshots', ['shot'], 'custom', 'month')
})
it('polls running jobs and presents partial results without claiming every file moved', async () => {
  vi.useFakeTimers()
  vi.mocked(api.organizationHistory).mockResolvedValue([{ ...batch, running: true, revision: 1 }])
  vi.mocked(api.organizationStatus).mockResolvedValue({ ...batch, revision: 3, items: [{ ...batch.items[0]!, status: 'failed', note: 'Another file now occupies the destination.' }] })
  await act(async () => root.render(<OrganizationHistory />))
  expect(host.textContent).toContain('Giving things a home')
  await act(async () => { await vi.advanceTimersByTimeAsync(300) })
  expect(host.textContent).toContain('0 screenshots organized'); expect(host.textContent).toContain('1 left in place'); expect(host.textContent).toContain('Another file now occupies')
  expect(refreshFindings).toHaveBeenCalled()
})

it('does not offer a retry after every selected file moved', async () => {
  await act(async () => root.render(<OrganizationHistory />))
  expect(host.textContent).not.toContain('Review remaining files')
})
it('lets a partial result return to selection without leaving the screen', async () => {
  vi.mocked(api.startOrganization).mockResolvedValue({ ...batch, items: [{ ...batch.items[0]!, status: 'failed', note: 'Permission denied.' }] })
  await act(async () => root.render(<Organization kind="screenshots" />)); await click('Review organization'); await click('Organize 1 screenshot')
  await click('Review remaining files')
  expect(host.textContent).toContain('Give these screenshots a home'); expect(host.textContent).toContain('Review organization')
  expect(api.organizationInventory).toHaveBeenCalledTimes(2)
})
