import { describe, expect, it } from 'vitest'

import { bytes, bytesParts, daysUntil, scatter, shortPath, whenish } from './format'

/**
 * `bytes` intentionally duplicates `human_bytes` in the Rust core. These tests
 * pin the shared rounding rules: if the two ever disagree, the same file shows
 * two different sizes depending on which screen you are looking at.
 */
describe('bytes', () => {
  it('matches the core rounding rules', () => {
    expect(bytes(0)).toBe('0 B')
    expect(bytes(999)).toBe('999 B')
    expect(bytes(1024)).toBe('1.00 KB')
    expect(bytes(1024 * 1024 * 6 + 1024 * 200)).toBe('6.20 MB')
    expect(bytes(1024 ** 3 * 318)).toBe('318 GB')
  })

  it('drops precision as the number grows, the way people speak', () => {
    expect(bytes(1024 ** 2 * 9.5)).toBe('9.50 MB')
    expect(bytes(1024 ** 2 * 18.7)).toBe('18.7 MB')
    expect(bytes(1024 ** 2 * 212)).toBe('212 MB')
  })

  it('splits into a value and a unit for typographic layouts', () => {
    expect(bytesParts(1024 ** 3 * 29.5)).toEqual({ value: '29.5', unit: 'GB' })
  })
})

describe('whenish', () => {
  const now = Date.UTC(2025, 0, 1) // fixed, so the suite does not drift
  const daysBefore = (days: number) => Math.floor(now / 1000) - days * 86400

  it('speaks in the units a person would use', () => {
    expect(whenish(daysBefore(0), now)).toBe('today')
    expect(whenish(daysBefore(1), now)).toBe('yesterday')
    expect(whenish(daysBefore(8), now)).toBe('8 days ago')
    expect(whenish(daysBefore(90), now)).toBe('about 3 months ago')
    expect(whenish(daysBefore(400), now)).toBe('about a year ago')
    expect(whenish(daysBefore(900), now)).toBe('about 2 years ago')
  })

  it('does not invent a date it was not given', () => {
    expect(whenish(null, now)).toBe('at some point')
  })
})

describe('daysUntil', () => {
  const now = Date.UTC(2025, 0, 1)

  it('never goes negative, so nothing reads as "-3 days left"', () => {
    expect(daysUntil(Math.floor(now / 1000) - 86400 * 5, now)).toBe(0)
  })

  it('rounds up, so the last day still counts as a day', () => {
    expect(daysUntil(Math.floor(now / 1000) + 86400 * 1.2, now)).toBe(2)
  })
})

describe('shortPath', () => {
  it('keeps the last two components so the file stays identifiable', () => {
    expect(shortPath('/Users/alice/Library/Application Support/Steam/common/Hades')).toBe(
      '/Users/…/common/Hades',
    )
  })

  it('keeps the leading separator', () => {
    // Without this an absolute path reads as a relative one, which is the
    // difference between "this file" and "a file with the same name".
    expect(shortPath('/a/b/c/d')).toMatch(/^\//)
  })

  it('keeps Windows drive letters', () => {
    expect(shortPath('C:\\Users\\bob\\AppData\\Local\\Thing')).toBe('C:\\…\\Local\\Thing')
  })

  it('leaves short paths alone', () => {
    expect(shortPath('/Users/alice')).toBe('/Users/alice')
  })

  it('substitutes the home directory when it knows it', () => {
    expect(shortPath('/Users/alice/Downloads/x.dmg', '/Users/alice')).toBe(
      '~/Downloads/x.dmg',
    )
  })
})

describe('scatter', () => {
  it('is stable for the same seed, so piles do not reshuffle on re-render', () => {
    expect(scatter('ghosts', 1)).toBe(scatter('ghosts', 1))
  })

  it('differs by seed and by salt', () => {
    expect(scatter('ghosts', 1)).not.toBe(scatter('ghosts', 2))
    expect(scatter('ghosts', 1)).not.toBe(scatter('copies', 1))
  })

  it('stays inside the unit interval', () => {
    for (const seed of ['a', 'ghosts:3', 'screenshots:11', '']) {
      const value = scatter(seed)
      expect(value).toBeGreaterThanOrEqual(0)
      expect(value).toBeLessThan(1)
    }
  })
})
