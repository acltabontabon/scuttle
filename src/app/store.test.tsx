import { act, useEffect } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { EMPTY_FINDINGS, SETTINGS, SPACE } from '@/dev/fixtures'
import { api, watchRummage } from '@/lib/ipc'
import type { ScanSummary } from '@/lib/types'
import { StoreProvider, useStore, type Store } from './store'

vi.mock('@/lib/ipc', () => ({
  api: {
    settings: vi.fn(), saveSettings: vi.fn(), findings: vi.fn(), rummage: vi.fn(),
    space: vi.fn(), moveStatus: vi.fn(), backgroundStatus: vi.fn(),
    cancelRummage: vi.fn(),
  },
  watchRummage: vi.fn(),
  watchMoves: vi.fn(async () => () => {}),
  watchBackground: vi.fn(async () => () => {}),
  watchIntents: vi.fn(async () => () => {}),
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

let root: Root
let host: HTMLDivElement
let store: Store
let events: Parameters<typeof watchRummage>[0]

function Probe() {
  const current = useStore()
  useEffect(() => { store = current }, [current])
  return null
}

const summary: ScanSummary = {
  scan_id: 'fast', files_seen: 0, bytes_seen: 0, candidates_found: 0,
  reclaimable_bytes: 0, duration_ms: 1,
  cancelled: false,
  hiccups: { permission_denied: 0, vanished: 0, unreadable: 0, loops_avoided: 0 },
}

beforeEach(async () => {
  vi.clearAllMocks()
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  vi.mocked(api.settings).mockResolvedValue({ ...SETTINGS })
  vi.mocked(api.findings).mockResolvedValue(EMPTY_FINDINGS)
  vi.mocked(api.moveStatus).mockResolvedValue(null)
  vi.mocked(api.backgroundStatus).mockRejectedValue(new Error('unavailable'))
  vi.mocked(watchRummage).mockImplementation(async (handlers) => {
    events = handlers
    return () => {}
  })
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => { root.render(<StoreProvider><Probe /></StoreProvider>) })
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
})

describe('rummage lifecycle', () => {
  it('keeps completion sent before the start response, ignoring another scan', async () => {
    vi.mocked(api.rummage).mockImplementation(async () => {
      events.onDone?.({ ...summary, scan_id: 'old' }, 'old')
      events.onProgress?.({ files_seen: 3, bytes_seen: 10, candidates_found: 0, current_area: 'Downloads' }, 'fast')
      events.onDone?.(summary, 'fast')
      return { scan_id: 'fast', roots: [] }
    })
    await act(async () => { await store.rummage() })
    expect(store.scan.status).toBe('done')
    expect(store.scan.summary).toEqual(summary)
    expect(store.scan.progress.files_seen).toBe(3)
  })

  it('does not let a repeated start overwrite the running scan', async () => {
    const start = deferred<{ scan_id: string; roots: [] }>()
    vi.mocked(api.rummage).mockReturnValue(start.promise)
    await act(async () => {
      const first = store.rummage()
      await store.rummage()
      start.resolve({ scan_id: 'fast', roots: [] })
      await first
    })
    expect(api.rummage).toHaveBeenCalledTimes(1)
    expect(store.scan.status).toBe('running')
    await act(async () => { events.onDone?.(summary, 'fast') })
    expect(store.scan.status).toBe('done')
  })

  it('reports cancellation errors without rejecting the action', async () => {
    vi.mocked(api.cancelRummage).mockRejectedValue({ code: 'io', message: 'Could not stop' })
    await act(async () => { await store.cancel() })
    expect(store.note?.text).toBe('Could not stop')
  })
})

describe('settings writes', () => {
  it('preserves both rapid changes and backend-owned preferences', async () => {
    let saved = { ...SETTINGS, has_rummaged_before: false }
    vi.mocked(api.settings).mockImplementation(async () => ({ ...saved }))
    const firstSave = deferred<typeof SETTINGS>()
    vi.mocked(api.saveSettings)
      .mockImplementationOnce(async (next) => {
        await firstSave.promise
        saved = { ...next, has_rummaged_before: true }
        return saved
      })
      .mockImplementationOnce(async (next) => { saved = next; return next })
    await act(async () => {
      const first = store.updateSettings({ appearance: 'dark' })
      const second = store.updateSettings({ personality: 'quiet' })
      await Promise.resolve()
      firstSave.resolve(saved)
      expect(await first).toBe(true)
      expect(await second).toBe(true)
    })
    expect(store.settings).toMatchObject({ appearance: 'dark', personality: 'quiet', has_rummaged_before: true })
  })

  it('continues saving after a rejected write', async () => {
    vi.mocked(api.saveSettings)
      .mockRejectedValueOnce(new Error('disk full'))
      .mockImplementationOnce(async (next) => next)
    await act(async () => {
      expect(await store.updateSettings({ appearance: 'dark' })).toBe(false)
      expect(await store.updateSettings({ personality: 'quiet' })).toBe(true)
    })
    expect(store.settings?.personality).toBe('quiet')
    expect(store.settings?.appearance).toBe(SETTINGS.appearance)
  })

  it('applies folder toggles against the latest selection', async () => {
    let saved = { ...SETTINGS, scan_roots: ['/a', '/b', '/c'] }
    vi.mocked(api.settings).mockImplementation(async () => saved)
    vi.mocked(api.saveSettings).mockImplementation(async (next) => { saved = next; return next })
    await act(async () => {
      const first = store.updateSettings((latest) => ({ scan_roots: latest.scan_roots.filter((p) => p !== '/a') }))
      const second = store.updateSettings((latest) => ({ scan_roots: latest.scan_roots.filter((p) => p !== '/b') }))
      await Promise.all([first, second])
    })
    expect(store.settings?.scan_roots).toEqual(['/c'])
  })
})

it('shares simultaneous space measurements', async () => {
  const measurement = deferred<typeof SPACE>()
  vi.mocked(api.space).mockReturnValue(measurement.promise)
  await act(async () => {
    const first = store.refreshSpace()
    const second = store.refreshSpace()
    expect(api.space).toHaveBeenCalledTimes(1)
    measurement.resolve(SPACE)
    expect(await first).toBe(true)
    expect(await second).toBe(true)
  })
  expect(store.space).toEqual(SPACE)
})

it('does not overwrite a new scan’s space figures with an older measurement', async () => {
  const old = deferred<typeof SPACE>()
  vi.mocked(api.space).mockReturnValueOnce(old.promise).mockResolvedValueOnce({ ...SPACE, reclaimable_estimate: 1 })
  vi.mocked(api.findings).mockResolvedValue({ ...EMPTY_FINDINGS, scan_id: 'new' })
  await act(async () => {
    const previous = store.refreshSpace()
    await store.refreshFindings()
    await store.refreshSpace()
    old.resolve(SPACE)
    await previous
  })
  expect(store.space?.reclaimable_estimate).toBe(1)
})
