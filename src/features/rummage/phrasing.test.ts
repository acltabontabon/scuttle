import { describe, expect, it } from 'vitest'

import { outcomeAside, outcomeLine, phaseLine } from './phrasing'

/**
 * Scuttle's voice is a product requirement, so it gets tests. The point of
 * these is not the exact wording — it is that the app never manufactures
 * urgency and never claims to have done work it has not done.
 */
describe('phaseLine', () => {
  it('describes the work the core actually reported', () => {
    expect(phaseLine({ phase: 'rummaging', area: 'Downloads' })).toBe(
      'Rummaging through Downloads…',
    )
    expect(phaseLine({ phase: 'examining', note: 'Looking for copies' })).toBe(
      'Looking for copies…',
    )
    expect(phaseLine({ phase: 'finished' })).toBe('Done.')
  })
})

describe('outcome copy', () => {
  it('says nothing was found without inventing a problem', () => {
    const line = `${outcomeLine(0, false)} ${outcomeAside(0, false)}`
    expect(line).toBe('Nothing interesting. Your computer is suspiciously tidy.')
  })

  it('never shouts, never counts problems, never uses fear', () => {
    const everything = [
      outcomeLine(0, false), outcomeLine(1, false), outcomeLine(40, false), outcomeLine(3, true),
      outcomeAside(0, false), outcomeAside(1, false), outcomeAside(9, false), outcomeAside(2, true),
      phaseLine({ phase: 'preparing' }),
      phaseLine({ phase: 'rummaging', area: 'Downloads' }),
      phaseLine({ phase: 'considering', note: 'Weighing the big things' }),
    ].join(' ')

    expect(everything).not.toMatch(/!/)
    expect(everything).not.toMatch(/\b(problem|problems|critical|warning|urgent|boost|optimi[sz])/i)
    expect(everything).not.toMatch(/[A-Z]{4,}/)
  })

  it('is honest when the user stopped it', () => {
    expect(outcomeLine(5, true)).toBe('Stopped.')
    expect(outcomeAside(5, true)).toContain('still there')
  })
})
