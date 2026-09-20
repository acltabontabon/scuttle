import { useEffect, useMemo } from 'react'

import { Detail } from '@/features/detail/Detail'
import { Findings } from '@/features/findings/Findings'
import { PileView } from '@/features/findings/PileView'
import { Drawer } from '@/features/quarantine/Drawer'
import { Rummage } from '@/features/rummage/Rummage'
import { Settings } from '@/features/settings/Settings'
import { Space } from '@/features/space/Space'
import { Mark } from '@/visuals/Mark'
import { useStore, type View } from './store'

import styles from './App.module.css'

/**
 * `available` decides whether a section is worth offering yet. Settings has no
 * condition: on a fresh install nothing else is reachable, and a first-time
 * user who wants to choose where Scuttle looks *before* the first rummage
 * needs a way in.
 */
const NAV: { view: View; label: string; available: (state: NavState) => boolean }[] = [
  { view: { name: 'findings' }, label: 'Findings', available: (s) => s.hasRummaged },
  { view: { name: 'drawer' }, label: 'Drawer', available: (s) => s.heldCount > 0 },
  { view: { name: 'space' }, label: 'Space', available: (s) => s.hasRummaged },
  { view: { name: 'settings' }, label: 'Settings', available: () => true },
]

interface NavState {
  hasRummaged: boolean
  heldCount: number
}

/** Rough platform sniff, used only to leave room for the traffic lights. */
function platform(): 'macos' | 'other' {
  return typeof navigator !== 'undefined' && /Mac/i.test(navigator.userAgent)
    ? 'macos'
    : 'other'
}

export function App() {
  const store = useStore()
  const { settings, findings, drawer, space, view, go, note, dismissNote, refreshDrawer, refreshSpace } =
    store

  // Appearance and motion are applied to the document root so CSS can do the
  // rest without any component knowing about themes.
  useEffect(() => {
    const root = document.documentElement
    if (settings?.appearance && settings.appearance !== 'system') {
      root.setAttribute('data-theme', settings.appearance)
    } else {
      root.removeAttribute('data-theme')
    }
    if (settings?.reduced_motion === true) root.setAttribute('data-motion', 'still')
    else root.removeAttribute('data-motion')
  }, [settings?.appearance, settings?.reduced_motion])

  // The drawer count is worth knowing without opening it; nothing else is.
  // Depending on `store` here would re-run this on every progress event,
  // since the store's identity changes with the scan state.
  useEffect(() => {
    if (drawer === null) void refreshDrawer()
  }, [drawer, refreshDrawer])

  /*
   * Measure the volume in the background, once, as soon as Space is reachable
   * at all.
   *
   * `api.space()` walks the disk, and nothing was started until the moment
   * someone clicked Space — so the first visit always paid for the whole
   * measurement with a blank screen and a "Measuring…" line. Doing it here
   * costs nobody anything: no view is waiting on it, and by the time the tab
   * is clicked the answer is usually already in the store.
   */
  useEffect(() => {
    if (findings?.has_rummaged !== true || space !== null) return
    void refreshSpace()
  }, [findings?.has_rummaged, space, refreshSpace])

  const navState: NavState = {
    hasRummaged: findings?.has_rummaged === true,
    heldCount: drawer?.items.length ?? 0,
  }
  const heldCount = navState.heldCount
  const sections = NAV.filter((item) => item.available(navState))

  const stage = useMemo(() => {
    switch (view.name) {
      case 'home':
        return <Rummage />
      case 'findings':
        return <Findings />
      case 'pile':
        return <PileView category={view.category} preselect={view.preselect} />
      case 'drawer':
        return <Drawer />
      case 'space':
        return <Space />
      case 'settings':
        return <Settings />
    }
  }, [view])

  return (
    <div className={styles.shell}>
      {/*
        `data-tauri-drag-region` is what actually makes this strip drag the
        window. The CSS `app-region` property it used to rely on is a
        Chromium extension despite its `-webkit-` prefix, and macOS runs this
        application in WKWebView, where it does nothing at all — so the title
        bar was hidden and the window could not be moved.

        Tauri hands a mousedown to the native window only when the event's
        own target carries the attribute, so the wordmark and the nav links
        below stay clickable without any opt-out of their own.
      */}
      <header data-tauri-drag-region className={styles.bar} data-platform={platform()}>
        <button
          className={styles.wordmark}
          onClick={() => go({ name: 'home' })}
          aria-label="Scuttle, back to the beginning"
        >
          <Mark size={17} />
          Scuttle
          {view.name !== 'home' && <span className={styles.wordmarkDot}>·</span>}
        </button>

        {sections.length > 0 && (
          <nav className={styles.nav} aria-label="Sections">
            {sections.map(({ view: target, label }) => (
              <button
                key={label}
                className={styles.navLink}
                aria-current={view.name === target.name ? 'page' : undefined}
                onClick={() => go(target)}
              >
                {label}
                {target.name === 'drawer' && heldCount > 0 && (
                  <span className={styles.navCount}>{heldCount}</span>
                )}
              </button>
            ))}
          </nav>
        )}
      </header>

      <main className={styles.stage}>{stage}</main>

      <Detail />

      {note && (
        <div className={styles.note} data-tone={note.tone} role="status" key={note.id}>
          <span className={styles.noteText}>{note.text}</span>
          {note.action && (
            <button
              className={styles.noteAction}
              onClick={() => {
                note.action?.run()
                dismissNote()
              }}
            >
              {note.action.label}
            </button>
          )}
          <button className={styles.noteDismiss} onClick={dismissNote} aria-label="Dismiss">
            ×
          </button>
        </div>
      )}
    </div>
  )
}
