import { describe, expect, it } from 'vitest'

import { sweepLabel } from './PileView'

/**
 * The sweep button is the one place in Scuttle where a single click moves
 * several things at once, so its label has to say exactly how many and how
 * much. Counting wrong here is how a user ends up surprised by their own
 * drawer.
 */
describe('sweepLabel', () => {
  it('names the count and the size every time', () => {
    expect(sweepLabel(1, 1024 ** 3 * 6.4)).toBe('Put it in the drawer · 6.40 GB')
    expect(sweepLabel(2, 1024 ** 2 * 420)).toBe('Put both in the drawer · 420 MB')
    expect(sweepLabel(6, 1024 ** 3 * 1.4)).toBe('Put all 6 in the drawer · 1.40 GB')
  })

  it('never says "1 things" or "all 2"', () => {
    expect(sweepLabel(1, 100)).not.toMatch(/1 thing|all 1/)
    expect(sweepLabel(2, 100)).not.toMatch(/all 2/)
  })

  it('stays calm at absurd counts', () => {
    const label = sweepLabel(412, 1024 ** 3 * 8)
    expect(label).toBe('Put all 412 in the drawer · 8.00 GB')
    expect(label).not.toMatch(/!/)
  })
})
