import { describe, expect, it } from 'vitest'

import type { MovePhase, MoveSnapshot } from '@/lib/types'
import { announcement, applySnapshot, isTerminal, seedJob, SETTLING_FRACTION, trackFor } from './progress'

function snap(over: Partial<MoveSnapshot> = {}): MoveSnapshot {
  return {
    job_id: 1,
    rev: 1,
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
    ...over,
  }
}

describe('applySnapshot', () => {
  it('takes the first snapshot it sees', () => {
    const first = snap()
    expect(applySnapshot(null, first)).toBe(first)
  })

  it('takes a higher revision and drops a repeat, a late one and an old one', () => {
    const at3 = snap({ rev: 3, processed: 30 })
    expect(applySnapshot(at3, snap({ rev: 4, processed: 40 }))!.processed).toBe(40)
    expect(applySnapshot(at3, snap({ rev: 3, processed: 99 }))).toBe(at3)
    expect(applySnapshot(at3, snap({ rev: 2, processed: 20 }))).toBe(at3)
  })

  it('survives events arriving in any order', () => {
    const events = [1, 2, 3, 4, 5, 6].map((rev) => snap({ rev, processed: rev * 10 }))
    // Every ordering of a shuffled delivery ends at the newest.
    for (const order of [[5, 1, 3, 0, 4, 2], [0, 1, 2, 3, 4, 5], [5, 4, 3, 2, 1, 0], [2, 2, 5, 5, 0, 1]]) {
      let state: MoveSnapshot | null = null
      for (const i of order) state = applySnapshot(state, events[i]!)
      expect(state!.rev).toBe(Math.max(...order.map((i) => events[i]!.rev)))
    }
  })

  it('does not let a straggler pull a finished job back to moving', () => {
    const done = snap({ rev: 9, phase: 'completed', processed: 10, total: 10 })
    const late = snap({ rev: 7, phase: 'moving', processed: 7, total: 10 })
    expect(applySnapshot(done, late)).toBe(done)
  })

  it('lets a newer job replace an older one, finished or not, and ignores an older job', () => {
    const old = snap({ job_id: 4, rev: 50, phase: 'completed' })
    const next = snap({ job_id: 5, rev: 1 })
    expect(applySnapshot(old, next)).toBe(next)
    expect(applySnapshot(next, snap({ job_id: 4, rev: 99 }))).toBe(next)
  })

  it('accepts an event that arrives before the start call has returned', () => {
    // Nothing is known about job 7 yet; its terminal event lands first.
    const finished = snap({ job_id: 7, rev: 12, phase: 'completed' })
    const afterEvent = applySnapshot(null, finished)
    // ...and the start call's response must not undo it.
    expect(seedJob(afterEvent, 7)).toBe(afterEvent)
  })

  it('seeds a job from its id only when nothing newer is known', () => {
    const seeded = seedJob(null, 3)!
    expect(seeded).toMatchObject({ job_id: 3, rev: 0, phase: 'checking' })
    // The first real event replaces the seed.
    expect(applySnapshot(seeded, snap({ job_id: 3, rev: 1 }))!.rev).toBe(1)
    // An older job's id never replaces a newer state.
    const newer = snap({ job_id: 9 })
    expect(seedJob(newer, 8)).toBe(newer)
  })
})

describe('trackFor', () => {
  it('is indeterminate while checking, because the amount of work is not known', () => {
    const t = trackFor(snap({ phase: 'checking' }))
    expect(t.mode).toBe('indeterminate')
    expect(t.detail).toBeNull()
  })

  it('is determinate on real counts while moving, and never claims a byte percentage', () => {
    const t = trackFor(snap({ phase: 'moving', total: 400, processed: 100 }))
    expect(t.mode).toBe('determinate')
    expect(t.fraction).toBeCloseTo(0.25)
    expect(t.detail).toBe('100 of 400')
    expect(t.detail).not.toContain('%')
    expect(t.detail).not.toMatch(/MB|GB|KB/)
  })

  it('mentions bytes only when bytes were really copied', () => {
    const renamed = trackFor(snap({ phase: 'moving', total: 10, processed: 5, copied_bytes: 0 }))
    expect(renamed.detail).not.toMatch(/copied/)
    const copied = trackFor(snap({ phase: 'moving', total: 10, processed: 5, copied_bytes: 5 * 1024 * 1024 }))
    expect(copied.detail).toMatch(/copied/)
  })

  it('never looks finished before it is', () => {
    const almost = trackFor(snap({ phase: 'moving', total: 100, processed: 100 }))
    expect(almost.fraction).toBeLessThan(1)
    const settling = trackFor(snap({ phase: 'finalizing', total: 100, processed: 100 }))
    expect(settling.mode).toBe('settling')
    expect(settling.fraction).toBe(SETTLING_FRACTION)
    expect(settling.fraction).toBeLessThan(1)
    expect(trackFor(snap({ phase: 'completed', total: 100, processed: 100 })).fraction).toBe(1)
  })

  it('is indeterminate rather than dividing by zero when there is nothing to count', () => {
    expect(trackFor(snap({ phase: 'moving', total: 0 })).mode).toBe('indeterminate')
    expect(trackFor(snap({ phase: 'moving', total: null })).mode).toBe('indeterminate')
  })

  it('shows how far a stopped or failed run really got', () => {
    const stopped = trackFor(snap({ phase: 'cancelled', total: 100, processed: 40 }))
    expect(stopped.fraction).toBeCloseTo(0.4)
    expect(stopped.label).toBe('Stopped')
  })

  it('says it is stopping until the worker has actually stopped', () => {
    expect(trackFor(snap({ phase: 'moving', total: 10, processed: 2, cancelling: true })).stopping).toBe(true)
    expect(trackFor(snap({ phase: 'cancelled', cancelling: true })).stopping).toBe(false)
  })

  it('gives assistive technology the phase, and counts where there are any', () => {
    expect(trackFor(snap({ phase: 'moving', total: 8, processed: 3 })).valueText).toBe(
      'Moving into the drawer, 3 of 8',
    )
    expect(trackFor(snap({ phase: 'finalizing' })).valueText).toBe('Finalizing drawer records')
  })
})

describe('announcement', () => {
  it('speaks on a change of phase and stays quiet for counts', () => {
    const early = announcement(snap({ phase: 'moving', total: 100, processed: 1 }), false)
    const late = announcement(snap({ phase: 'moving', total: 100, processed: 90 }), false)
    expect(early).toBe(late)
  })

  it('says something the instant a click lands, before any event has arrived', () => {
    expect(announcement(null, true)).toBe('Checking what you picked')
    expect(announcement(null, false)).toBe('')
  })

  it('leaves the outcome to the toast so it is announced once', () => {
    for (const phase of ['completed', 'partial', 'failed', 'cancelled'] as MovePhase[]) {
      expect(announcement(snap({ phase }), false)).toBe('')
      expect(isTerminal(phase)).toBe(true)
    }
  })
})
