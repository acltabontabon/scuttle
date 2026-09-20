import { memo, useMemo } from 'react'

import { bytes, scatter } from '@/lib/format'
import type { Pile } from '@/lib/types'
import { CATEGORY_HINT, CATEGORY_PLAIN } from '@/visuals/CategoryMeta'
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
  /**
   * How big this pile is against the biggest one on the floor, 0 to 1.
   *
   * The heaps used to be identical in size and scattered at random vertical
   * offsets, which read as a grid that had gone wrong rather than as a
   * composition. The variation now comes from what the piles actually weigh:
   * forty-seven gigabytes looks like more than twenty-eight megabytes before
   * either figure has been read.
   */
  weight: number
  onOpen: () => void
}

function HeapImpl({ pile, index, weight, onOpen }: HeapProps) {
  const hint = CATEGORY_HINT[pile.category]
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
        y: (scatter(seed, 2) - 0.5) * 22 + i * 1.4,
        rotate: (scatter(seed, 3) - 0.5) * 44,
        size: 28 + scatter(seed, 4) * 11,
        variant: i,
        delay: index * 90 + i * 55,
      }
    })
  }, [pile.category, pile.count, index])

  // Square-rooted, so the largest pile is emphatic without the smallest
  // becoming a speck. Sizes on a floor are compared by eye, not measured.
  const scale = 0.82 + Math.sqrt(Math.max(0, Math.min(1, weight))) * 0.42

  return (
    <button
      className={styles.heap}
      style={{ '--heap-scale': scale.toFixed(3) } as React.CSSProperties}
      onClick={onOpen}
      data-actionable={pile.actionable}
      aria-label={`${pile.title} — ${CATEGORY_PLAIN[pile.category]}. ${pile.count} ${
        pile.count === 1 ? 'thing' : 'things'
      }, ${bytes(pile.bytes)} worth reviewing.`}
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
                bottom: 12,
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
          {/*
            A mark only where there is something to mark. "you decide" sat
            under almost every pile, which made it wallpaper rather than
            information; a suggestion is the rarer, more useful state, so
            that is the one that gets a word.
          */}
          {pile.confident_count > 0 && (
            <>
              {' · '}
              <span className={styles.suggested}>
                <span className={styles.suggestedDot} aria-hidden="true" />
                {pile.confident_count} suggested
              </span>
            </>
          )}
        </span>
        {hint && <span className={styles.hint}>{hint}</span>}
      </span>
    </button>
  )
}

export const Heap = memo(HeapImpl)
