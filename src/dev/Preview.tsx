import { useMemo, useState } from 'react'

import { App } from '@/app/App'
import { StoreContext, type Store, type View } from '@/app/store'
import type { Candidate, Findings, Settings } from '@/lib/types'
import { CANDIDATES, DRAWER, EMPTY_FINDINGS, FINDINGS, SETTINGS, SPACE } from './fixtures'

import styles from './Preview.module.css'

/**
 * The design workbench.
 *
 * Reachable only at `/preview.html` during development. It mounts the real
 * application against fixture state so every screen, pile and verdict can be
 * looked at side by side — including the ones that are hard to produce on
 * demand, like "nothing found" and "this has save data in it".
 *
 * It is not part of the shipping bundle: `preview.html` is not an input to the
 * production build.
 */

const SCENES: { id: string; label: string; view: View; findings: Findings }[] = [
  { id: 'home', label: 'Home', view: { name: 'home' }, findings: FINDINGS },
  { id: 'floor', label: 'The floor', view: { name: 'findings' }, findings: FINDINGS },
  { id: 'empty', label: 'Found nothing', view: { name: 'findings' }, findings: EMPTY_FINDINGS },
  { id: 'ghosts', label: 'Ghosts pile', view: { name: 'pile', category: 'ghosts' }, findings: FINDINGS },
  { id: 'shots', label: 'Screenshots pile', view: { name: 'pile', category: 'screenshots' }, findings: FINDINGS },
  { id: 'drawer', label: 'Drawer', view: { name: 'drawer' }, findings: FINDINGS },
  { id: 'space', label: 'Space', view: { name: 'space' }, findings: FINDINGS },
  { id: 'settings', label: 'Settings', view: { name: 'settings' }, findings: FINDINGS },
]

const DETAILS: { id: string; label: string }[] = [
  { id: 'g1', label: 'Detail: confident ghost' },
  { id: 'g3', label: 'Detail: save data (refused)' },
  { id: 's1', label: 'Detail: screenshot burst' },
  { id: 'h1', label: 'Detail: heavy stray' },
  { id: 'c1', label: 'Detail: duplicates' },
  { id: 'o1', label: 'Detail: oddment' },
]

export function Preview() {
  const [sceneId, setSceneId] = useState(SCENES[1]!.id)
  const [detailId, setDetailId] = useState<string | null>(null)
  const [theme, setTheme] = useState<Settings['appearance']>('system')

  const scene = SCENES.find((s) => s.id === sceneId)!
  const detail = detailId ? (CANDIDATES.find((c) => c.id === detailId) ?? null) : null

  const store = useMemo<Store>(() => {
    const noop = async () => {}
    const ok = async () => true
    return {
      view: scene.view,
      go: (view) => {
        const match = SCENES.find(
          (s) => s.view.name === view.name && (view.name !== 'pile' || s.view.name === 'pile'),
        )
        if (match) setSceneId(match.id)
        setDetailId(null)
      },
      scan: {
        status: 'idle',
        scanId: null,
        phase: { phase: 'preparing' },
        progress: { files_seen: 0, bytes_seen: 0, candidates_found: 0, current_area: '' },
        roots: [],
        latest: null,
        foundCount: 0,
        heaviestBytes: 0,
        summary: null,
        error: null,
      },
      rummage: noop,
      cancel: noop,
      findings: scene.findings,
      refreshFindings: ok,
      detail,
      openDetail: (candidate: Candidate | null) => setDetailId(candidate?.id ?? null),
      drawer: DRAWER,
      refreshDrawer: ok,
      space: SPACE,
      refreshSpace: ok,
      settings: { ...SETTINGS, appearance: theme },
      updateSettings: async (next) => {
        setTheme(next.appearance)
        return true
      },
      note: null,
      say: () => {},
      dismissNote: () => {},
      quarantine: noop,
      quarantineGroup: noop,
      quarantineConfident: noop,
      quarantineAllConfident: noop,
      quarantineMany: noop,
      emptyDrawer: noop,
      keep: noop,
      ignore: noop,
      restore: ok,
      removePermanently: ok,
      reveal: noop,
    }
  }, [scene, detail, theme])

  return (
    <div className={styles.workbench}>
      <aside className={styles.rail}>
        <p className={styles.railTitle}>Scenes</p>
        {SCENES.map((s) => (
          <button
            key={s.id}
            className={styles.railItem}
            aria-current={s.id === sceneId}
            onClick={() => {
              setSceneId(s.id)
              setDetailId(null)
            }}
          >
            {s.label}
          </button>
        ))}

        <p className={styles.railTitle}>Detail sheet</p>
        {DETAILS.map((d) => (
          <button
            key={d.id}
            className={styles.railItem}
            aria-current={d.id === detailId}
            onClick={() => setDetailId(detailId === d.id ? null : d.id)}
          >
            {d.label}
          </button>
        ))}

        <p className={styles.railTitle}>Theme</p>
        <div className={styles.railRow}>
          {(['system', 'light', 'dark'] as const).map((mode) => (
            <button
              key={mode}
              className={styles.railChip}
              aria-current={theme === mode}
              onClick={() => setTheme(mode)}
            >
              {mode}
            </button>
          ))}
        </div>
      </aside>

      <div className={styles.window}>
        <StoreContext.Provider value={store}>
          <App />
        </StoreContext.Provider>
      </div>
    </div>
  )
}
