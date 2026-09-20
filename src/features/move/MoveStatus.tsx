import { announcement, isTerminal, trackFor } from './progress'
import { describeReport } from './phrasing'
import type { MoveSnapshot } from '@/lib/types'
import { Scuttle } from '@/visuals/Scuttle'

import styles from './Move.module.css'

/**
 * The thin track that stands in for the selection gauge while something moves.
 *
 * It is the same 3px line the plate already draws, doing a different job, so
 * the plate does not change shape when work starts: the figure above it and
 * the words below it stay where they are.
 *
 * Meaning is carried by the words (`MoveLine`) and by the progressbar role;
 * the motion is decoration. Indeterminate work is not given a percentage,
 * and nothing reaches full until the drawer's records are written.
 */
export function MoveTrack({
  snapshot,
  pending,
}: {
  snapshot: MoveSnapshot | null
  pending: boolean
}) {
  const track = snapshot ? trackFor(snapshot) : null
  const mode = track?.mode ?? 'indeterminate'
  const determinate = mode === 'determinate' && snapshot?.total != null && snapshot.total > 0

  return (
    <div
      className={styles.track}
      role="progressbar"
      aria-label="Moving into the drawer"
      aria-valuemin={0}
      aria-valuemax={determinate ? snapshot!.total! : undefined}
      // Only a real count is a value. For indeterminate work the attribute is
      // left off, which is how a progressbar says "I don't know".
      aria-valuenow={determinate ? snapshot!.processed : undefined}
      aria-valuetext={track?.valueText ?? 'Checking what you picked'}
      data-mode={mode}
      data-pending={pending && !snapshot ? '' : undefined}
    >
      <span
        className={styles.trackFill}
        style={mode === 'indeterminate' ? undefined : { transform: `scaleX(${track?.fraction ?? 0})` }}
      />
    </div>
  )
}

/** What the track is doing, in words, and the real counts beside it. */
export function MoveLine({
  snapshot,
}: {
  snapshot: MoveSnapshot | null
}) {
  const track = snapshot ? trackFor(snapshot) : null
  return (
    <>
      <span>{track?.label ?? 'Checking what you picked'}</span>
      {track?.detail ? <span className={styles.lineDetail}> · {track.detail}</span> : null}
    </>
  )
}

/**
 * What replaces the action button while work is under way: Scuttle carrying
 * on, and a way to stop.
 *
 * "Cancel", not "Stop after this file": a copy across drives is checked
 * between chunks, so it can stop in the middle of a file, and the words say
 * what the button really does. The button says so honestly while it winds
 * down, and the outcome only reads "stopped" once the worker has.
 */
export function MoveControls({
  snapshot,
  pending,
  onCancel,
  controlRef,
}: {
  snapshot: MoveSnapshot | null
  pending: boolean
  onCancel: () => void
  controlRef?: React.Ref<HTMLDivElement>
}) {
  const track = snapshot ? trackFor(snapshot) : null
  const stopping = track?.stopping ?? false
  // Once every file is dealt with there is nothing left to cancel; the
  // records are being written and that finishes on its own.
  const settling = snapshot?.phase === 'finalizing'

  return (
    <div className={styles.controls} ref={controlRef} tabIndex={-1}>
      <span className={styles.carrier} aria-hidden="true">
        <Scuttle mood={stopping || settling ? 'idle' : 'rummaging'} size={38} />
      </span>
      {settling ? (
        <span className={styles.controlNote}>Almost there</span>
      ) : (
        <button className={styles.cancel} onClick={onCancel} disabled={stopping || (pending && !snapshot)}>
          {stopping ? 'Stopping…' : 'Cancel'}
        </button>
      )}
      {/*
        The one polite announcement: the phase, and only when it changes. The
        counts move many times a second and are left to the progressbar.
      */}
      <span className={styles.visuallyHidden} role="status" aria-live="polite">
        {announcement(snapshot, pending)}
      </span>
    </div>
  )
}

/** For the header: a short label for a finished move that left something to see. */
export function chipText(snapshot: MoveSnapshot): string | null {
  if (!isTerminal(snapshot.phase) || !snapshot.report) return null
  const report = snapshot.report
  if (report.notice) return 'Needs a look'
  if (report.outcome === 'cancelled') return 'Stopped'
  if (report.failed > 0) return `${report.failed.toLocaleString('en-US')} didn't move`
  if (report.skipped > 0) return `${report.skipped.toLocaleString('en-US')} skipped`
  return describeReport(report).hasDetails ? 'Details' : null
}
