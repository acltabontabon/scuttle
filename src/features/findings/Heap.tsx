import { memo, useMemo } from 'react'

import { bytes, scatter } from '@/lib/format'
import type { Pile } from '@/lib/types'
import { Glyph } from '@/visuals/Glyph'

import styles from './Heap.module.css'

/**
 * How many objects to draw for a pile.
 *
 * Logarithmic, so eighteen screenshots look like more than three, a thousand
 * look faintly ridiculous, and nothing ever tries to draw a thousand glyphs.
 */
function heapSize(count: number): number {
  return Math.max(2, Math.min(13, Math.round(Math.log2(count + 1)) + 2))
}

interface HeapProps {
  pile: Pile
  /** Staggers the settling animation across a row of piles. */
  index: number
  onOpen: () => void
}

function HeapImpl({ pile, index, onOpen }: HeapProps) {
  const objects = useMemo(() => {
    const total = heapSize(pile.count)
    return Array.from({ length: total }, (_, i) => {
      const seed = `${pile.category}:${i}`
      // Objects spread outward and settle downward, so the heap reads as
      // something poured out rather than a neat stack.
      const spread = 1 - i / total
      return {
        key: seed,
        x: (scatter(seed, 1) - 0.5) * 86 * (0.45 + spread),
        y: (scatter(seed, 2) - 0.5) * 22 + i * 1.5,
        rotate: (scatter(seed, 3) - 0.5) * 44,
        size: 26 + scatter(seed, 4) * 10,
        variant: i,
        delay: index * 90 + i * 55,
      }
    })
  }, [pile.category, pile.count, index])

  return (
    <button
      className={styles.heap}
      onClick={onOpen}
      data-actionable={pile.actionable}
      aria-label={`${pile.title}, ${pile.count} ${pile.count === 1 ? 'thing' : 'things'}, ${bytes(pile.bytes)}`}
    >
      <div className={styles.objects}>
        <span className={styles.shadow} />
        {objects.map((object) => (
          <span
            key={object.key}
            className={styles.object}
            style={
              {
                left: '50%',
                bottom: 16,
                '--x': `calc(-50% + ${object.x}px)`,
                '--y': `${-object.y}px`,
                '--r': `${object.rotate}deg`,
                transform: `translate(calc(-50% + ${object.x}px), ${-object.y}px) rotate(${object.rotate}deg)`,
                animation: `settle 620ms cubic-bezier(0.22, 1.2, 0.36, 1) ${object.delay}ms backwards`,
              } as React.CSSProperties
            }
          >
            <Glyph category={pile.category} size={object.size} variant={object.variant} />
          </span>
        ))}
        <span
          className={styles.dust}
          style={{ animationDelay: `${index * 90 + objects.length * 55}ms` }}
        />
      </div>

      <span className={styles.label}>
        <span className={styles.name}>{pile.title}</span>
        <span className={styles.size}>{bytes(pile.bytes)}</span>
        <span className={styles.count}>
          {pile.count} {pile.count === 1 ? 'thing' : 'things'}
          {pile.actionable === 0 && ' · nothing suggested'}
        </span>
      </span>
    </button>
  )
}

export const Heap = memo(HeapImpl)
