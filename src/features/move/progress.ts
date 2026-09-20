import { bytes } from '@/lib/format'
import type { MovePhase, MoveSnapshot } from '@/lib/types'

/**
 * The shape of a move as the interface sees it.
 *
 * Everything here is a pure function of snapshots, because the hard part of
 * showing progress honestly is not drawing it — it is deciding what to believe.
 * Events can arrive late, twice, or out of order; one can even arrive before
 * the call that started the job has returned. The rules below make all of that
 * harmless.
 */

const TERMINAL: ReadonlySet<MovePhase> = new Set(['completed', 'partial', 'failed', 'cancelled'])

export function isTerminal(phase: MovePhase): boolean {
  return TERMINAL.has(phase)
}

/**
 * Fold an incoming snapshot into what is already known. Returns the very same
 * object when the incoming one should be ignored, so callers can tell.
 *
 *  - A newer job replaces an older one, whatever state the older was in.
 *  - Within a job, only a higher revision is news. Anything else — a repeat, a
 *    late arrival, a snapshot fetched before a newer event landed — is dropped.
 *  - Because a job's last snapshot has its highest revision, once a job has
 *    finished no straggler from earlier in the same job can pull it back.
 */
export function applySnapshot(
  current: MoveSnapshot | null,
  incoming: MoveSnapshot,
): MoveSnapshot | null {
  if (current === null) return incoming
  if (incoming.job_id > current.job_id) return incoming
  if (incoming.job_id < current.job_id) return current
  return incoming.rev > current.rev ? incoming : current
}

/**
 * A stand-in for a job whose id the start call has just returned, used only if
 * no event for it has arrived yet. Never replaces something newer.
 */
export function seedJob(current: MoveSnapshot | null, jobId: number): MoveSnapshot | null {
  if (current !== null && current.job_id >= jobId) return current
  return {
    job_id: jobId,
    rev: 0,
    phase: 'checking',
    cancelling: false,
    total: null,
    processed: 0,
    moved: 0,
    skipped: 0,
    failed: 0,
    moved_bytes: 0,
    copied_bytes: 0,
    label: '',
    report: null,
  }
}

/**
 * How to draw the track.
 *
 *  - `indeterminate`: work of unknown size. Checking is always this: Scuttle
 *    does not yet know how much there is, and a percentage would be invented.
 *  - `determinate`: a real fraction of a real total, counted in files (or in
 *    whole items). Never bytes for a rename — a rename copies none.
 *  - `settling`: everything is dealt with and the Drawer's records are being
 *    written. Held short of full, because nothing is finished until it is
 *    recorded.
 */
export type TrackMode = 'indeterminate' | 'determinate' | 'settling'

export interface Track {
  mode: TrackMode
  /** 0..1. Meaningful for `determinate`; `settling` sits just short of 1. */
  fraction: number
  /** What is happening, in words. This carries the meaning. */
  label: string
  /** Counts, when there are real ones to show. */
  detail: string | null
  /** For assistive technology: the label plus the counts. */
  valueText: string
  /** True only while a stop has been asked for and work is still winding down. */
  stopping: boolean
}

/** Just short of full: the honest place to wait while records are written. */
export const SETTLING_FRACTION = 0.98

function count(n: number): string {
  return n.toLocaleString('en-US')
}

export function trackFor(snapshot: MoveSnapshot): Track {
  const stopping = snapshot.cancelling && !isTerminal(snapshot.phase)
  const total = snapshot.total
  const counts =
    total !== null && total > 0 ? `${count(snapshot.processed)} of ${count(total)}` : null
  // Only real copies have bytes to speak of.
  const copied = snapshot.copied_bytes > 0 ? `${bytes(snapshot.copied_bytes)} copied` : null

  switch (snapshot.phase) {
    case 'checking': {
      const label = 'Checking what you picked'
      return { mode: 'indeterminate', fraction: 0, label, detail: null, valueText: label, stopping }
    }
    case 'moving': {
      const label = stopping ? 'Stopping' : 'Moving into the drawer'
      if (total === null || total === 0) {
        return { mode: 'indeterminate', fraction: 0, label, detail: null, valueText: label, stopping }
      }
      // Not allowed to look finished while still moving.
      const fraction = Math.min(snapshot.processed / total, SETTLING_FRACTION)
      const detail = [counts, copied].filter(Boolean).join(' · ')
      return {
        mode: 'determinate',
        fraction,
        label,
        detail,
        valueText: `${label}, ${counts}`,
        stopping,
      }
    }
    case 'finalizing': {
      const label = 'Finalizing drawer records'
      return {
        mode: 'settling',
        fraction: SETTLING_FRACTION,
        label,
        detail: counts,
        valueText: label,
        stopping,
      }
    }
    case 'completed':
      return {
        mode: 'determinate',
        fraction: 1,
        label: 'Done',
        detail: counts,
        valueText: 'Done',
        stopping: false,
      }
    default: {
      // Partial, failed or cancelled: show how far it really got.
      const fraction =
        total !== null && total > 0 ? Math.min(1, snapshot.processed / total) : 0
      const label =
        snapshot.phase === 'cancelled'
          ? 'Stopped'
          : snapshot.phase === 'failed'
            ? 'Could not finish'
            : 'Finished, with some left'
      return {
        mode: 'determinate',
        fraction,
        label,
        detail: counts,
        valueText: label,
        stopping: false,
      }
    }
  }
}

/**
 * What a screen reader should be told, and only when it changes meaning: the
 * phase, and the outcome. Counts change many times a second and are not
 * announced; they are available on demand through the progressbar itself.
 */
export function announcement(snapshot: MoveSnapshot | null, pending: boolean): string {
  if (snapshot === null) return pending ? 'Checking what you picked' : ''
  if (snapshot.cancelling && !isTerminal(snapshot.phase)) return 'Stopping'
  switch (snapshot.phase) {
    case 'checking':
      return 'Checking what you picked'
    case 'moving':
      return 'Moving into the drawer'
    case 'finalizing':
      return 'Finalizing drawer records'
    default:
      return ''
  }
}
