import type { ScanStatus } from '@/app/store'

/**
 * When home should hand you over to the findings.
 *
 * A rummage that finishes while you are watching it should carry you onward —
 * you asked a question and the answer is ready. A rummage that finished some
 * time ago should do nothing at all, because you are only here because you
 * asked to be.
 *
 * Telling those apart needs the *transition* into `done`, not the state of
 * being `done`. `scan.status` stays `done` in the store for the rest of the
 * session, so a rule written against the value alone fires every time home is
 * opened: clicking the wordmark showed home for a second and a half and then
 * threw you back to the findings, which made home unreachable after the first
 * scan of the session.
 *
 * Kept out of the component and made pure so the rule can be stated once and
 * tested, rather than living inside an effect where it can only be found by
 * tripping over it.
 */
export interface HandOff {
  /** Whether a run has been witnessed. Carry this into the next call. */
  sawItRun: boolean
  /** Move to the findings, after the usual pause. */
  handOver: boolean
}

export function handOff(status: ScanStatus, found: number, sawItRun: boolean): HandOff {
  // Watching one now. Remember it, so its ending means something.
  if (status === 'running') return { sawItRun: true, handOver: false }

  // Nothing to hand over: still idle, cancelled, failed, or it found nothing.
  if (status !== 'done' || found === 0) return { sawItRun, handOver: false }

  // Finished, with something to show — but only worth following if the
  // finishing happened in front of you.
  if (!sawItRun) return { sawItRun, handOver: false }

  // Once per run. Forgetting immediately is what stops a re-render, or a
  // second visit, from handing you over twice.
  return { sawItRun: false, handOver: true }
}
