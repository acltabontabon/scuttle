import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SETTINGS } from '@/dev/fixtures'
import { api } from '@/lib/ipc'
import type { DependencyCacheReport } from '@/lib/types'
import type { SettingsChange } from '@/app/store'
import { DependencyCaches } from './DependencyCaches'

vi.mock('@/lib/ipc', () => ({ api: {
  dependencyCachePreview: vi.fn(), cancelDependencyCachePreview: vi.fn(async () => {}),
  chooseDeveloperRoot: vi.fn(async () => '/fixture/custom'),
} }))
const saved = vi.fn()
function Fixture({ enabled = false }: { enabled?: boolean }) {
  const [settings, setSettings] = useState({ ...SETTINGS, dependency_caches: { ...SETTINGS.dependency_caches, enabled } })
  const patch = async (change: SettingsChange) => {
    const changes = typeof change === 'function' ? change(settings) : change
    saved(changes); setSettings({ ...settings, ...changes }); return true
  }
  return <DependencyCaches settings={settings} patch={patch} commonProjects={['/fixture/Projects']} />
}

function report(): DependencyCacheReport {
  return {
    policy_version: 1, evaluated_unix: 2_000_000_000, retention_days: 90, complete: false,
    repositories: [{ kind: 'maven', path: '/fixture/repository', present: true, entries: 1, bytes: 42, complete: false }],
    projects: [], bytes: 42, kept: 0, insufficient_evidence: 1, eligible: 0,
    notes: ['Inspection only: nothing was moved or deleted.'],
    entries: [{ id: 'old', kind: 'maven', repository: '/fixture/repository', path: '/fixture/repository/org/lib/1.0',
      artifact: 'org:lib', version: '1.0', bytes: 42, complete: false, metadata_fingerprint: 'metadata',
      newest_modified_unix: null, last_used_unix: null, origin: 'unknown', projects: [],
      decision: 'insufficient_evidence', reasons: ['unknown_usage'], explanations: ['Last use is unknown.'],
    }],
  }
}

let host: HTMLDivElement
let root: Root
beforeEach(() => {
  vi.clearAllMocks(); saved.mockClear()
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  host = document.createElement('div'); document.body.append(host); root = createRoot(host)
})
afterEach(async () => { await act(async () => root.unmount()); host.remove() })
async function click(label: string) {
  const button = [...host.querySelectorAll('button')].find((b) => b.textContent?.trim() === label || b.getAttribute('aria-label') === label)
  expect(button).toBeDefined(); await act(async () => button!.click())
}
it('is off by default and enabling preview does not start inspection', async () => {
  await act(async () => root.render(<Fixture />))
  expect(host.textContent).not.toContain('Inspect dependency caches')
  await click('Preview dependency caches')
  expect(saved).toHaveBeenCalledWith({ dependency_caches: { enabled: true, retention_days: 90, locations: [] } })
  expect(api.dependencyCachePreview).not.toHaveBeenCalled()
})
it('shows partial measurements and unknown usage without offering a destructive action', async () => {
  vi.mocked(api.dependencyCachePreview).mockResolvedValue(report())
  await act(async () => root.render(<Fixture enabled />))
  await click('Inspect dependency caches')
  expect(host.textContent).toContain('Partial inventory · at least')
  expect(host.textContent).toContain('org:lib @ 1.0')
  expect(host.textContent).toContain('Last use: unknown')
  expect(host.textContent).toContain('Insufficient evidence')
  expect([...host.querySelectorAll('button')].some((b) => /delete|drawer|prune/i.test(b.textContent ?? ''))).toBe(false)
})
it('persists retention and custom locations independently of build settings', async () => {
  await act(async () => root.render(<Fixture enabled />))
  const retention = host.querySelector('#dependency-retention') as HTMLSelectElement
  await act(async () => { retention.value = '180'; retention.dispatchEvent(new Event('change', { bubbles: true })) })
  expect(saved).toHaveBeenLastCalledWith({ dependency_caches: { enabled: true, retention_days: 180, locations: [] } })
  await click('Add cache location')
  expect(saved).toHaveBeenLastCalledWith({ dependency_caches: { enabled: true, retention_days: 180, locations: [{ kind: 'maven', path: '/fixture/custom' }] } })
  await click('Remove cache /fixture/custom')
  expect(saved).toHaveBeenLastCalledWith({ dependency_caches: { enabled: true, retention_days: 180, locations: [] } })
})
it('hides a report when its project boundary changes', async () => {
  vi.mocked(api.dependencyCachePreview).mockResolvedValue(report())
  await act(async () => root.render(<Fixture enabled />))
  await click('Inspect dependency caches')
  expect(host.textContent).toContain('org:lib @ 1.0')
  await click('Add dependency project folder')
  expect(saved).toHaveBeenLastCalledWith({ developer_roots: ['/fixture/Projects', '/fixture/custom'] })
  expect(host.textContent).not.toContain('org:lib @ 1.0')
})
it('supports cancellation and surfaces errors while keeping inspection read-only', async () => {
  let finish!: (value: DependencyCacheReport) => void
  vi.mocked(api.dependencyCachePreview).mockReturnValue(new Promise((resolve) => { finish = resolve }))
  await act(async () => root.render(<Fixture enabled />))
  await click('Inspect dependency caches')
  await click('Stop cache inspection')
  expect(api.cancelDependencyCachePreview).toHaveBeenCalledOnce()
  await act(async () => finish(report()))
  vi.mocked(api.dependencyCachePreview).mockRejectedValue({ code: 'busy', message: 'Scuttle is moving files.' })
  await click('Inspect dependency caches')
  expect(host.querySelector('[role="alert"]')?.textContent).toContain('Scuttle is moving files.')
})
