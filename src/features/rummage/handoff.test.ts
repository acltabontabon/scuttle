import { describe, expect, it } from 'vitest'

import { handOff } from './handoff'

/**
 * Home is allowed to move you along exactly once, and only when a rummage
 * finished while you were looking at it. Getting this wrong does not produce a
 * visual glitch, it produces a screen you cannot stay on.
 */
describe('handOff', () => {
  it('carries you onward when a rummage finishes in front of you', () => {
    const watching = handOff('running', 3, false)
    expect(watching).toEqual({ sawItRun: true, handOver: false })

    const finished = handOff('done', 12, watching.sawItRun)
    expect(finished.handOver).toBe(true)
  })

  it('stays put when the rummage finished before you arrived', () => {
    // The reported bug. `status` is still `done` from a scan earlier in the
    // session, and home has just been opened from the wordmark. Nothing about
    // arriving here is a request to leave again.
    expect(handOff('done', 12, false)).toEqual({ sawItRun: false, handOver: false })
  })

  it('hands over once, not on every render', () => {
    const first = handOff('done', 12, true)
    expect(first.handOver).toBe(true)

    // Same status, same findings, one render later.
    expect(handOff('done', 12, first.sawItRun).handOver).toBe(false)
  })

  it('stays put when the rummage found nothing', () => {
    // There is nothing on the findings floor to go and look at.
    expect(handOff('done', 0, true).handOver).toBe(false)
  })

  it('stays put when the rummage was stopped or broke', () => {
    expect(handOff('cancelled', 9, true).handOver).toBe(false)
    expect(handOff('failed', 9, true).handOver).toBe(false)
    expect(handOff('idle', 0, true).handOver).toBe(false)
  })

  it('remembers a run it witnessed until that run ends', () => {
    // A scan is watched, then stopped: the memory must not leak into a later
    // `done` that belongs to a scan nobody watched.
    const watched = handOff('running', 2, false)
    const stopped = handOff('cancelled', 2, watched.sawItRun)
    expect(stopped.handOver).toBe(false)
    expect(stopped.sawItRun).toBe(true)
  })
})
