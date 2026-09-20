import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Contrast is a product requirement, not a matter of taste: "artistic does not
 * mean inaccessible". This reads the real token file, so a palette change that
 * makes text unreadable fails here rather than in someone's hands.
 *
 * The light palette once shipped `--ink-faint` at 2.76:1 carrying most of the
 * small text in the app, and `--honey` at 2.08:1. Hence this file.
 */

// Resolved from the project root: under jsdom `import.meta.url` is an
// http:// URL and cannot be turned back into a file path.
const css = readFileSync(resolve(process.cwd(), 'src/styles/tokens.css'), 'utf8')

function channel(value: number): number {
  const c = value / 255
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
}

function luminance(hex: string): number {
  const h = hex.replace('#', '')
  const [r, g, b] = [0, 2, 4].map((i) => channel(parseInt(h.slice(i, i + 2), 16)))
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!
}

function contrast(a: string, b: string): number {
  const [la, lb] = [luminance(a), luminance(b)]
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05)
}

/** Tokens from one `:root` block of the stylesheet. */
function palette(index: number): Record<string, string> {
  const block = css.split(':root')[index]
  if (!block) throw new Error(`no :root block at ${index}`)
  const found: Record<string, string> = {}
  for (const match of block.matchAll(/--([a-z-]+):\s*(#[0-9a-f]{6})/g)) {
    found[match[1]!] = match[2]!
  }
  return found
}

/** Every token that type is ever set in. */
const TEXT_TOKENS = ['ink', 'ink-soft', 'ink-faint', 'honey-ink', 'moss-ink', 'clay-ink']

const THEMES: [string, number][] = [
  ['light', 1],
  ['dark', 2],
]

describe.each(THEMES)('%s palette', (_name, index) => {
  const tokens = palette(index)

  it.each(TEXT_TOKENS)('--%s meets AA against the paper', (token) => {
    expect(tokens[token], `--${token} is missing`).toBeDefined()
    expect(contrast(tokens[token]!, tokens.paper!)).toBeGreaterThanOrEqual(4.5)
  })

  it('keeps raised and deep surfaces readable too', () => {
    for (const surface of ['paper-raised', 'paper-deep']) {
      for (const token of ['ink', 'ink-soft']) {
        expect(
          contrast(tokens[token]!, tokens[surface]!),
          `--${token} on --${surface}`,
        ).toBeGreaterThanOrEqual(4.5)
      }
    }
  })

  it('keeps text readable on the accent fills that carry it', () => {
    // The tinted bands — the background-check caveat on Findings, the "the
    // system said no" line in Settings — set type directly on an accent fill.
    // Their own -ink siblings are not readable there: honey-ink on honey-soft
    // is 3.99:1 in the light palette. Ordinary ink is, and the accent does its
    // work as the fill and the border.
    for (const fill of ['honey-soft', 'clay-soft']) {
      expect(
        contrast(tokens.ink!, tokens[fill]!),
        `--ink on --${fill}`,
      ).toBeGreaterThanOrEqual(4.5)
    }
  })

  it('draws hairlines that can actually be seen', () => {
    // Borders are not text, so AA does not apply — but invisible is not a
    // border. The dark theme once drew every separator at 1.09:1 because it
    // reused the recessed fill colour, which sits *below* the surface there.
    expect(contrast(tokens.rule!, tokens.paper!)).toBeGreaterThanOrEqual(1.3)
    expect(contrast(tokens.rule!, tokens['paper-raised']!)).toBeGreaterThanOrEqual(1.3)
  })
})

describe('the palette as a whole', () => {
  it('defines a text sibling for every accent', () => {
    // The plain accents are tuned for fills and dots. If one of these ever
    // goes missing, something is setting type in a colour nobody checked.
    for (const [, index] of THEMES) {
      const tokens = palette(index)
      for (const accent of ['honey', 'moss', 'clay']) {
        expect(tokens[`${accent}-ink`], `--${accent}-ink`).toBeDefined()
      }
    }
  })

  it('never lets a risk level be told apart by hue alone', () => {
    // Guards the pairing, not the colours: risk is always written out in words
    // beside its dot. See PileView and Detail.
    const light = palette(1)
    expect(contrast(light['moss-ink']!, light['clay-ink']!)).toBeLessThan(4.5)
  })
})
