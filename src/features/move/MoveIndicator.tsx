import { useEffect, useRef } from 'react'

import { useStore } from '@/app/store'
import { MoveSummary } from './MoveSummary'
import { chipText } from './MoveStatus'
import { isTerminal, trackFor } from './progress'

import styles from './Move.module.css'

const RADIUS = 6
const CIRCUMFERENCE = 2 * Math.PI * RADIUS

/**
 * The move, wherever you are in Scuttle.
 *
 * Navigating away from the pile does not stop the work, and it should not stop
 * you seeing it. While a move runs this is a small ring and a few words in the
 * header; when it has finished and left something worth a look — files that
 * were in use, files that changed — it stays as a quiet chip, so an
 * unresolved failure does not vanish with a toast.
 */
export function MoveIndicator() {
  const { move, moving, moveDetailsOpen, setMoveDetailsOpen } = useStore()
  const chip = useRef<HTMLButtonElement>(null)
  const { snapshot } = move

  const finished = snapshot !== null && isTerminal(snapshot.phase)
  const afterText = finished ? chipText(snapshot) : null

  // Hand focus back to the chip when the details close, so a keyboard user
  // is not dropped at the top of the page.
  const wasOpen = useRef(false)
  useEffect(() => {
    if (wasOpen.current && !moveDetailsOpen) chip.current?.focus({ preventScroll: true })
    wasOpen.current = moveDetailsOpen
  }, [moveDetailsOpen])

  if (!moving && afterText === null) return null

  const track = snapshot && !finished ? trackFor(snapshot) : null
  const mode = moving ? (track?.mode ?? 'indeterminate') : 'done'
  const fraction = track?.fraction ?? 0
  const text = moving
    ? [track?.label ?? 'Checking', track?.detail].filter(Boolean).join(' · ')
    : afterText

  return (
    <div className={styles.indicator}>
      <button
        ref={chip}
        className={styles.chip}
        onClick={() => setMoveDetailsOpen(!moveDetailsOpen)}
        aria-expanded={moveDetailsOpen}
        aria-haspopup="dialog"
        aria-label={moving ? `Moving into the drawer. ${text}. Show details` : `Last move: ${text}. Show details`}
      >
        <svg className={styles.ring} width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" data-mode={mode}>
          <circle cx="8" cy="8" r={RADIUS} className={styles.ringBed} />
          {mode !== 'done' && (
            <circle
              cx="8"
              cy="8"
              r={RADIUS}
              className={styles.ringArc}
              strokeDasharray={
                mode === 'indeterminate'
                  ? `${CIRCUMFERENCE * 0.3} ${CIRCUMFERENCE}`
                  : `${CIRCUMFERENCE * fraction} ${CIRCUMFERENCE}`
              }
              transform="rotate(-90 8 8)"
            />
          )}
          {mode === 'done' && <circle cx="8" cy="8" r="2.4" className={styles.ringDot} />}
        </svg>
        <span className={styles.chipText}>{text}</span>
      </button>
      {moveDetailsOpen && <MoveSummary />}
    </div>
  )
}
