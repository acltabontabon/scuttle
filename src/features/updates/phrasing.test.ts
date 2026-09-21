import { describe, expect, it } from 'vitest'

import {
  AUTO_CHECK_HINT,
  checkedAgo,
  installedLine,
  percent,
  RESTART_NOTICE,
  sizeLine,
} from './phrasing'

const NOW = 1_700_000_000_000

describe('sizeLine', () => {
  it('says how much of how much when the size is known', () => {
    expect(sizeLine(4 * 1024 * 1024, 18 * 1024 * 1024)).toBe('4.00 MB of 18.0 MB')
  })

  it('does not invent a total when the server did not send one', () => {
    expect(sizeLine(4 * 1024 * 1024, null)).toBe('4.00 MB so far')
    expect(sizeLine(4 * 1024 * 1024, 0)).toBe('4.00 MB so far')
  })

  it('never reads as having passed the end', () => {
    expect(sizeLine(2000, 1000)).toBe('1000 B of 1000 B')
  })
})

describe('percent', () => {
  it('is a whole number of a known size', () => {
    expect(percent(250, 1000)).toBe(25)
    expect(percent(999, 1000)).toBe(99)
  })

  it('is nothing at all when there is no honest number', () => {
    expect(percent(250, null)).toBeNull()
    expect(percent(250, 0)).toBeNull()
  })

  it('stops at one hundred', () => {
    expect(percent(5000, 1000)).toBe(100)
  })
})

describe('checkedAgo', () => {
  it('is gentle about recent times', () => {
    expect(checkedAgo(NOW / 1000 - 10, NOW)).toBe('just now')
    expect(checkedAgo(NOW / 1000 - 20 * 60, NOW)).toBe('20 minutes ago')
    expect(checkedAgo(NOW / 1000 - 3 * 3600, NOW)).toBe('3 hours ago')
  })

  it('falls back to day words after a day', () => {
    expect(checkedAgo(NOW / 1000 - 3 * 86400, NOW)).toBe('3 days ago')
  })

  it('does not go negative when the clocks disagree a little', () => {
    expect(checkedAgo(NOW / 1000 + 30, NOW)).toBe('just now')
  })
})

describe('the words that matter', () => {
  it('says the alpha channel aloud and leaves stable unremarked', () => {
    expect(installedLine('0.1.0-alpha.2', 'alpha')).toBe('Scuttle 0.1.0-alpha.2 · alpha')
    expect(installedLine('0.2.0', 'stable')).toBe('Scuttle 0.2.0')
  })

  it('says that installing closes and reopens Scuttle', () => {
    expect(RESTART_NOTICE).toMatch(/closes Scuttle and opens the new version/)
  })

  it('says that checking only looks', () => {
    expect(AUTO_CHECK_HINT).toMatch(/only looks/)
    expect(AUTO_CHECK_HINT).toMatch(/nothing is downloaded until you say so/)
  })
})
