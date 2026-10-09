import { useEffect, useMemo } from 'react'

import { Organization, OrganizationHistory } from '@/features/organization/Organization'
import { Detail } from '@/features/detail/Detail'
import { MoveIndicator } from '@/features/move/MoveIndicator'
import { Review } from '@/features/move/Review'
import { Findings } from '@/features/findings/Findings'
import { PileView } from '@/features/findings/PileView'
import { Drawer } from '@/features/quarantine/Drawer'
import { Rummage } from '@/features/rummage/Rummage'
import { Settings } from '@/features/settings/Settings'
import { Space } from '@/features/space/Space'
import { UpdateChip } from '@/features/updates/UpdateChip'
import { platform } from '@/lib/platform'
import { Mark } from '@/visuals/Mark'
import { Icon } from '@/visuals/Icon'
import { useStore, type View } from './store'

import styles from './App.module.css'

/**
 * Scan, Drawer and Settings are always reachable. Findings and Space become
 * useful after the first rummage, with Findings also active inside a category.
 */
const NAV: { view: View; label: string; available: (state: NavState) => boolean }[] = [
  { view: { name: 'home' }, label: 'Scan', available: () => true },
  { view: { name: 'findings' }, label: 'Findings', available: (s) => s.hasRummaged },
  { view: { name: 'drawer' }, label: 'Drawer', available: () => true },
  { view: { name: 'space' }, label: 'Space', available: (s) => s.hasRummaged },
  { view: { name: 'settings' }, label: 'Settings', available: () => true },
]

interface NavState {
  hasRummaged: boolean
  heldCount: number
  moving: boolean
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

  /*
   * Stop the decorative loops while nobody is looking at them.
   *
   * With background mode on, closing the window hides it rather than
   * destroying it, so the mascot would otherwise go on breathing behind the
   * menu bar. Scheduling and scan coordination live in Rust and are not
   * affected by this either way — this is only about not compositing frames
   * nobody can see.
   */
  useEffect(() => {
    const root = document.documentElement
    const apply = () => {
      if (document.visibilityState === 'hidden') root.setAttribute('data-window', 'hidden')
      else root.removeAttribute('data-window')
    }
    apply()
    document.addEventListener('visibilitychange', apply)
    return () => {
      document.removeEventListener('visibilitychange', apply)
      root.removeAttribute('data-window')
    }
  }, [])

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
  }, [findings, space, refreshSpace])

  const navState: NavState = {
    hasRummaged: findings?.has_rummaged === true,
    heldCount: drawer?.items.length ?? 0,
    moving: store.moving,
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
        return <PileView key={`${view.category}:${view.preselect ?? 'all'}`} category={view.category} preselect={view.preselect} />
      case 'organize':
        return <Organization key={view.kind} kind={view.kind} />
      case 'organization_history':
        return <OrganizationHistory />
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
          <Mark size={24} />
          Scuttle
        </button>

        <div className={styles.barRight}>
          {/* Work in progress, and work that left something to look at, wherever you are. */}
          <MoveIndicator />
          {/* Only when there is something about updating worth knowing. */}
          <UpdateChip />
          {sections.length > 0 && (
          <nav className={styles.nav} aria-label="Sections">
            {sections.map(({ view: target, label }) => (
              <button
                key={label}
                className={styles.navLink}
                aria-current={view.name === target.name || (['pile', 'organize', 'organization_history'].includes(view.name) && target.name === 'findings') ? 'page' : undefined}
                onClick={() => go(target)}
              >
                <Icon name={target.name === 'home' ? 'scan' : target.name === 'pile' || target.name === 'organize' || target.name === 'organization_history' ? 'findings' : target.name} size={16} />
                {label}
                {target.name === 'drawer' && heldCount > 0 && (
                  <span className={styles.navCount}>{heldCount}</span>
                )}
              </button>
            ))}
          </nav>
          )}
        </div>
      </header>

      <main className={styles.stage}>{stage}</main>

      <Detail />
      <Review />

      {store.quitting && (
        <div className={styles.note} data-tone="plain" role="status">
          <span className={styles.noteText}>
            Finishing safely before quitting. Scuttle stops at the next file, settles the Drawer,
            and then closes. Anything that already moved can be put back next time.
          </span>
        </div>
      )}

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
