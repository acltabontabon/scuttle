import { useEffect, useMemo, useRef, useState } from 'react'
import { flushSync } from 'react-dom'

import { App } from '@/app/App'
import { StoreContext, type Store, type View } from '@/app/store'
import type { Candidate, Findings, Settings } from '@/lib/types'
import { isTerminal } from '@/features/move/progress'
import { UpdateProvider } from '@/features/updates/UpdateProvider'
import {
  BACKGROUND_STATUS,
  CANDIDATES,
  DRAWER,
  EMPTY_FINDINGS,
  FINDINGS,
  GLANCE_FINDINGS,
  MOVE_SCENES,
  SETTINGS,
  SPACE,
  UPDATE_SCENES,
} from './fixtures'

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
  {
    id: 'glance',
    label: 'After a background check',
    view: { name: 'findings' },
    findings: GLANCE_FINDINGS,
  },
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
  const [moveId, setMoveId] = useState('none')
  const [stream, setStream] = useState<{ running: boolean; result: string } | null>(null)
  const [streamed, setStreamed] = useState<number | null>(null)
  const streamStart = useRef(0)
  const [detailsOpen, setDetailsOpen] = useState(false)

  const scene = SCENES.find((s) => s.id === sceneId)!
  const detail = detailId ? (CANDIDATES.find((c) => c.id === detailId) ?? null) : null

  const baseScene = MOVE_SCENES.find((m) => m.id === moveId)!
  // A stress run replaces the moving scene's counts with a fast-changing
  // stream, to see what a burst of updates costs the interface.
  const moveScene = useMemo(
    () =>
      streamed !== null && baseScene.snapshot
        ? { ...baseScene, snapshot: { ...baseScene.snapshot, processed: streamed, moved: streamed } }
        : baseScene,
    [baseScene, streamed],
  )

  // Ten updates a second is what the core sends at most; this sends a hundred,
  // for six seconds, while watching how long each frame takes to arrive and
  // whether the main thread ever stalls.
  useEffect(() => {
    if (!stream?.running) return
    let frames = 0
    let last = performance.now()
    let worstGap = 0
    let longTasks = 0
    let raf = 0
    // A 5 ms timer that should arrive every 5 ms. Any gap much longer than
    // that is the main thread being busy — which is what "the window froze"
    // means — and, unlike animation frames, it is not throttled when the pane
    // is in the background.
    let lastBeat = performance.now()
    let worstBeat = 0
    const beat = window.setInterval(() => {
      const now = performance.now()
      worstBeat = Math.max(worstBeat, now - lastBeat)
      lastBeat = now
    }, 5)
    const observer =
      typeof PerformanceObserver !== 'undefined'
        ? new PerformanceObserver((list) => (longTasks += list.getEntries().length))
        : null
    try {
      observer?.observe({ entryTypes: ['longtask'] })
    } catch {
      /* not supported here */
    }
    const tick = (now: number) => {
      frames += 1
      worstGap = Math.max(worstGap, now - last)
      last = now
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    streamStart.current = performance.now()
    let n = 0
    const timer = window.setInterval(() => {
      n += 1
      setStreamed(n)
      if (performance.now() - streamStart.current > 6000) {
        window.clearInterval(timer)
        window.clearInterval(beat)
        cancelAnimationFrame(raf)
        observer?.disconnect()
        const seconds = (performance.now() - streamStart.current) / 1000
        setStreamed(null)
        setStream({
          running: false,
          result: `${n} updates in ${seconds.toFixed(1)}s · slowest 5 ms beat took ${worstBeat.toFixed(0)} ms · ${longTasks} long tasks · ${(frames / seconds).toFixed(0)} fps (throttled if the pane is hidden) · worst frame gap ${worstGap.toFixed(0)} ms`,
        })
      }
    }, 10)
    return () => {
      window.clearInterval(timer)
      window.clearInterval(beat)
      cancelAnimationFrame(raf)
      observer?.disconnect()
    }
  }, [stream?.running])

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
      background: BACKGROUND_STATUS,
      refreshBackground: noop,
      pauseBackground: noop,
      setLaunchAtLogin: noop,
      note: null,
      say: () => {},
      dismissNote: () => {},
      move: { snapshot: moveScene.snapshot, pending: moveScene.pending === true },
      moving:
        moveScene.pending === true ||
        (moveScene.snapshot !== null && !isTerminal(moveScene.snapshot.phase)),
      cancelMove: noop,
      dismissMove: async () => {
        setMoveId('none')
        setDetailsOpen(false)
      },
      retryMove: noop,
      reviewAgain: noop,
      moveDetailsOpen: detailsOpen,
      setMoveDetailsOpen: setDetailsOpen,
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
  }, [scene, detail, theme, moveScene, detailsOpen])

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

        <p className={styles.railTitle}>A move</p>
        {MOVE_SCENES.map((m) => (
          <button
            key={m.id}
            className={styles.railItem}
            aria-current={m.id === moveId}
            onClick={() => {
              setMoveId(m.id)
              setDetailsOpen(false)
            }}
          >
            {m.label}
          </button>
        ))}

        <button
          className={styles.railItem}
          onClick={() => {
            setMoveId('moving')
            setStream({ running: true, result: '' })
          }}
        >
          Stream 100 updates/s
        </button>
        <button
          className={styles.railItem}
          onClick={() => {
            // Commit 500 updates one after another, synchronously, and time
            // each. Unlike a timer this is not slowed by a hidden pane, so it
            // says what one progress event costs the interface to show.
            setMoveId('moving')
            setTimeout(() => {
              const times: number[] = []
              for (let i = 1; i <= 500; i += 1) {
                const t = performance.now()
                flushSync(() => setStreamed(i))
                times.push(performance.now() - t)
              }
              setStreamed(null)
              times.sort((a, b) => a - b)
              const total = times.reduce((a, b) => a + b, 0)
              setStream({
                running: false,
                result: `500 commits: mean ${(total / 500).toFixed(2)} ms · p95 ${times[474]!.toFixed(2)} ms · worst ${times[499]!.toFixed(2)} ms`,
              })
            }, 50)
          }}
        >
          Render cost ×500
        </button>
        {stream && (
          <p className={styles.railTitle} data-testid="stream-result">
            {stream.running ? 'running…' : stream.result}
          </p>
        )}

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
          {/* Static: there is no core behind the workbench. `?update=ready` and
              friends pick a scene from the fixtures. */}
          <UpdateProvider
            live={false}
            initial={UPDATE_SCENES[new URLSearchParams(window.location.search).get('update') ?? ''] ?? null}
          >
            <App />
          </UpdateProvider>
        </StoreContext.Provider>
      </div>
    </div>
  )
}
