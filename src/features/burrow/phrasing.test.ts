import { describe, expect, it } from 'vitest'

import { burrowLine, lastLook, PLAYFUL_SHARE, type BurrowStatus } from './phrasing'

const NOW = 1_800_000_000

function status(partial: Partial<BurrowStatus> = {}): BurrowStatus {
  return {
    now_unix: NOW,
    has_looked: true,
    last_look_unix: NOW - 3 * 86400,
    found: 12,
    suggested: 3,
    drawer_items: 0,
    drawer_bytes: 0,
    needs_attention: 0,
    busy: null,
    personality: 'full',
    reduced_motion: null,
    appearance: 'system',
    ...partial,
  }
}

const ROLLS = Array.from({ length: 40 }, (_, i) => i / 40)

describe("Scuttle's line in the burrow", () => {
  it('is plain most of the time', () => {
    expect(burrowLine(status(), 0.99)).toBe('12 things worth a look.')
  })

  it('is plain whenever Scuttle is asked to be quiet', () => {
    for (const roll of ROLLS) {
      expect(burrowLine(status({ personality: 'quiet' }), roll)).toBe('12 things worth a look.')
    }
  })

  it('never jokes about something that needs attention', () => {
    for (const roll of ROLLS) {
      expect(burrowLine(status({ needs_attention: 1 }), roll)).toContain('needs a look')
    }
  })

  it('never jokes while files are moving', () => {
    for (const roll of ROLLS) {
      expect(burrowLine(status({ busy: 'moving files into the Drawer' }), roll)).toBe(
        'Busy moving files into the Drawer.',
      )
    }
  })

  it('only says what is true: no empty-drawer joke when the drawer holds things', () => {
    for (const roll of ROLLS) {
      const line = burrowLine(status({ drawer_items: 4, found: 0 }), roll)
      expect(line).not.toMatch(/empty|landlord/)
    }
  })

  it('never claims findings that are not there', () => {
    for (const roll of ROLLS) {
      const line = burrowLine(status({ found: 0, drawer_items: 0 }), roll)
      expect(line).not.toMatch(/found things|worth a look/)
    }
  })

  it('never mentions a path or a file name, and stays short', () => {
    for (const roll of ROLLS) {
      for (const s of [status(), status({ found: 0 }), status({ drawer_items: 2 }), status({ has_looked: false })]) {
        const line = burrowLine(s, roll)
        expect(line).not.toMatch(/[\\/]|\.\w{2,4}\b/)
        expect(line.length).toBeLessThan(70)
      }
    }
  })

  it('does get playful sometimes', () => {
    expect(burrowLine(status(), PLAYFUL_SHARE / 2)).not.toBe('12 things worth a look.')
  })

  it('reads the last look roughly', () => {
    expect(lastLook(status())).toBe('3 days ago')
    expect(lastLook(status({ has_looked: false }))).toBe('Not yet')
  })
})
