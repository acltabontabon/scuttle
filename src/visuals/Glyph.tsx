import { memo } from 'react'

import type { Category } from '@/lib/types'

/**
 * One small drawn object per pile.
 *
 * These are deliberately imperfect: coordinates are off-grid, strokes are
 * uneven and nothing is quite symmetrical, so the piles read as objects
 * someone dragged out rather than as an icon set. They are drawn in the
 * current text colour so they inherit whatever the surface is doing.
 */

interface GlyphProps {
  category: Category
  size?: number
  /** A seed so repeated glyphs in one pile are not identical. */
  variant?: number
  className?: string
}

const STROKE = {
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.7,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
}

function shapes(category: Category, variant: number) {
  // A tiny deterministic wobble, so two screenshots in a pile are not clones.
  const w = ((variant * 37) % 7) - 3

  switch (category) {
    case 'ghosts':
      return (
        <>
          <path
            {...STROKE}
            d={`M6 21 L6 ${10 + w * 0.2} C6 5.4 9.6 2.5 14 2.5 C18.4 2.5 22 5.4 22 ${10 + w * 0.2} L22 21 L18.6 18 L15.4 21 L12.2 18 L9 21 Z`}
          />
          <circle cx="11.2" cy="10.6" r="1.5" fill="currentColor" />
          <circle cx="17" cy="10.6" r="1.5" fill="currentColor" />
        </>
      )
    case 'screenshots':
      return (
        <>
          <rect {...STROKE} x={3.5} y={5.5 + w * 0.2} width="15" height="11" rx="1.4" />
          <rect {...STROKE} x={8} y={9} width="15" height="11" rx="1.4" />
          <path {...STROKE} d="M11 15.5 L14.4 12.6 L17 14.8 L20 11.6" />
        </>
      )
    case 'installers':
      return (
        <>
          <path {...STROKE} d={`M4 8.4 L14 3.4 L24 8.4 L24 18 L14 ${23 + w * 0.1} L4 18 Z`} />
          <path {...STROKE} d="M4 8.4 L14 13.4 L24 8.4" />
          <path {...STROKE} d="M14 13.4 L14 23" />
        </>
      )
    case 'heavy_strays':
      return (
        <>
          <path
            {...STROKE}
            d={`M5 21 C4 13 8 ${7 + w * 0.3} 14 7 C20 ${7 + w * 0.2} 24 13 23 21 Z`}
          />
          <path {...STROKE} d="M10.6 7.4 L11.6 3.6 L16.6 3.6 L17.4 7.4" />
          <path {...STROKE} d="M9.4 15.5 L18.4 15.5" opacity="0.5" />
        </>
      )
    case 'copies':
      return (
        <>
          <rect {...STROKE} x={3.4} y={3.6} width="13.4" height="13.4" rx="1.6" />
          <rect {...STROKE} x={9.6} y={8.4 + w * 0.15} width="13.4" height="13.4" rx="1.6" />
        </>
      )
    case 'caches':
      return (
        <>
          <ellipse {...STROKE} cx="14" cy={6.6} rx="9.4" ry="3.4" />
          <path {...STROKE} d={`M4.6 6.6 L4.6 ${12 + w * 0.15} C4.6 13.9 8.8 15.4 14 15.4 C19.2 15.4 23.4 13.9 23.4 ${12 + w * 0.15} L23.4 6.6`} />
          <path {...STROKE} d="M4.6 12.8 L4.6 18.2 C4.6 20.1 8.8 21.6 14 21.6 C19.2 21.6 23.4 20.1 23.4 18.2 L23.4 12.8" />
        </>
      )
    case 'developer_debris':
      return (
        <>
          <path {...STROKE} d={`M9.6 ${7 + w * 0.2} L4 13.4 L9.6 19.8`} />
          <path {...STROKE} d="M18.4 7 L24 13.4 L18.4 19.8" />
          <path {...STROKE} d="M15.6 4.4 L12.4 22.4" />
        </>
      )
    case 'oddments':
    default:
      return (
        <>
          <path
            {...STROKE}
            d={`M14 3.4 C19.6 3.4 23.4 7.6 23.4 13 C23.4 18.6 19 ${22.4 + w * 0.1} 14 22.4 C8.6 22.4 4.6 18.4 4.6 13 C4.6 7.6 8.4 3.4 14 3.4 Z`}
          />
          <path {...STROKE} d="M11.2 10.4 C11.2 8.4 12.6 7.4 14.2 7.4 C15.8 7.4 17 8.4 17 9.9 C17 12 14.2 12.2 14.2 14.4" />
          <circle cx="14.2" cy="17.8" r="1.3" fill="currentColor" />
        </>
      )
  }
}

function GlyphImpl({ category, size = 28, variant = 0, className }: GlyphProps) {
  return (
    <svg
      viewBox="0 0 28 26"
      width={size}
      height={size * (26 / 28)}
      aria-hidden="true"
      focusable="false"
      className={className}
      style={{ overflow: 'visible' }}
    >
      {shapes(category, variant)}
    </svg>
  )
}

export const Glyph = memo(GlyphImpl)
