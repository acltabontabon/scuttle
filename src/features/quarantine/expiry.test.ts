import { describe, expect, it } from 'vitest'

import { expiryLabel, expiryState } from './DrawerItem'

/**
 * The drawer's expiry wording has to match what the core actually does. The
 * sweep runs in `commands::init` — app launch — so nothing is deleted at a
 * particular hour, and an item can sit past its date indefinitely while
 * Scuttle is closed.
 */
describe('expiryLabel', () => {
  it('never promises deletion at a moment', () => {
    for (const days of [0, 1, 2, 7, 14]) {
      expect(expiryLabel(days)).not.toMatch(/today|tonight|now|midnight/)
    }
  })

  it('says what happens to something already past its date', () => {
    expect(expiryLabel(0)).toBe('expired · goes at next start')
  })

  it('counts the remaining days, singular and plural', () => {
    expect(expiryLabel(1)).toBe('1 day left')
    expect(expiryLabel(9)).toBe('9 days left')
    expect(expiryLabel(1)).not.toMatch(/1 days/)
  })
})

describe('expiryState', () => {
  it('reserves emphasis for the last couple of days', () => {
    // Every row shouting is every row ignored.
    expect(expiryState(14)).toBe('fine')
    expect(expiryState(3)).toBe('fine')
    expect(expiryState(2)).toBe('soon')
    expect(expiryState(1)).toBe('soon')
    expect(expiryState(0)).toBe('expired')
  })
})
