import { useEffect, useRef } from 'react'

import { installedLine, LATER } from './phrasing'
import { ReleaseNotes } from './ReleaseNotes'
import { useUpdates } from './UpdateProvider'
import { chipFor, describe } from './view'
import type { Track } from './view'

import styles from './Updates.module.css'

const RADIUS = 6
const CIRCUMFERENCE = 2 * Math.PI * RADIUS

/**
 * A quiet line in the header when there is something about updating worth
 * knowing, and nothing when there is not.
 *
 * It is a chip, not a banner: it never covers anything, never asks for
 * attention twice about a version that was waved away, and opens a small panel
 * rather than a dialog. Everything it does is also available in Settings.
 */
export function UpdateChip() {
  const { snapshot, panelOpen, setPanelOpen } = useUpdates()
  const chip = useRef<HTMLButtonElement>(null)
  const view = chipFor(snapshot)

  // Hand focus back to the chip when the panel closes, so a keyboard user is
  // not dropped at the top of the page.
  const wasOpen = useRef(false)
  useEffect(() => {
    if (wasOpen.current && !panelOpen) chip.current?.focus({ preventScroll: true })
    wasOpen.current = panelOpen
  }, [panelOpen])

  // If the chip goes away while the panel is open (a dismissal, say), so does
  // the panel.
  useEffect(() => {
    if (!view && panelOpen) setPanelOpen(false)
  }, [view, panelOpen, setPanelOpen])

  if (!view || !snapshot) return null

  return (
    <div className={styles.indicator}>
      <button
        ref={chip}
        className={styles.chip}
        data-tone={view.tone}
        onClick={() => setPanelOpen(!panelOpen)}
        aria-expanded={panelOpen}
        aria-haspopup="dialog"
        aria-label={`${view.text}. Show details`}
      >
        <svg className={styles.ring} width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" data-mode={view.mode}>
          <circle cx="8" cy="8" r={RADIUS} className={styles.ringBed} />
          {view.mode !== 'done' ? (
            <circle
              cx="8"
              cy="8"
              r={RADIUS}
              className={styles.ringArc}
              strokeDasharray={
                view.mode === 'indeterminate'
                  ? `${CIRCUMFERENCE * 0.3} ${CIRCUMFERENCE}`
                  : `${CIRCUMFERENCE * view.fraction} ${CIRCUMFERENCE}`
              }
              transform="rotate(-90 8 8)"
            />
          ) : (
            <circle cx="8" cy="8" r="2.4" className={styles.ringDot} />
          )}
        </svg>
        <span className={styles.chipText}>{view.text}</span>
      </button>
      {panelOpen && <UpdatePanel />}
    </div>
  )
}

function UpdatePanel() {
  const { snapshot, setPanelOpen, act, dismiss } = useUpdates()
  const panel = useRef<HTMLDivElement>(null)

  useEffect(() => {
    panel.current?.focus({ preventScroll: true })
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setPanelOpen(false)
    }
    const onPointer = (event: MouseEvent) => {
      const target = event.target as Node
      if (panel.current?.contains(target)) return
      // The chip toggles the panel itself; do not fight it.
      if ((target as Element).closest?.('[aria-haspopup="dialog"]')) return
      setPanelOpen(false)
    }
    document.addEventListener('keydown', onKey)
    document.addEventListener('mousedown', onPointer)
    return () => {
      document.removeEventListener('keydown', onKey)
      document.removeEventListener('mousedown', onPointer)
    }
  }, [setPanelOpen])

  if (!snapshot) return null
  const d = describe(snapshot)

  return (
    <div ref={panel} className={styles.panel} role="dialog" aria-label="Updating Scuttle" tabIndex={-1}>
      <p className={styles.headline}>{d.headline}</p>
      <p className={styles.sub}>{installedLine(snapshot.current_version, snapshot.channel)} is installed.</p>

      {d.track && <UpdateTrack track={d.track} label={d.headline} />}
      {d.detail && <p className={styles.detail}>{d.detail}</p>}

      {d.info && d.stage !== 'installing' && <ReleaseNotes info={d.info} />}

      {d.notice && <p className={styles.notice}>{d.notice}</p>}
      {d.blocked && (
        <p className={styles.blocked} role="status">
          {d.blocked}
        </p>
      )}
      {d.failure && (
        <p className={styles.failure} role="alert">
          {d.failure}
        </p>
      )}

      {(d.action || d.stage === 'available' || d.stage === 'ready') && (
        <div className={styles.actions}>
          {d.action && (
            <button className={styles.primary} onClick={() => void act(d.action!.kind)} disabled={d.working}>
              {d.action.label}
            </button>
          )}
          {(d.stage === 'available' || d.stage === 'ready') && (
            <button className={styles.later} onClick={() => void dismiss()}>
              {LATER}
            </button>
          )}
        </div>
      )}
    </div>
  )
}

/**
 * The plate's own 3px gauge. When the size is not known it travels instead of
 * filling, because a number would have to be made up.
 */
export function UpdateTrack({ track, label }: { track: Track; label: string }) {
  const determinate = track.mode === 'determinate'
  return (
    <div
      className={styles.track}
      data-mode={track.mode}
      role="progressbar"
      aria-label={label}
      aria-valuemin={determinate ? 0 : undefined}
      aria-valuemax={determinate ? 100 : undefined}
      aria-valuenow={determinate ? Math.round(track.fraction * 100) : undefined}
    >
      <span className={styles.trackFill} style={determinate ? { transform: `scaleX(${track.fraction})` } : undefined} />
    </div>
  )
}
