import { useEffect, useState } from 'react'

import { useStore } from '@/app/store'
import { bytes } from '@/lib/format'
import { Unavailable, Waiting } from '@/components/Unavailable'

import styles from './Space.module.css'

/**
 * Where the space went.
 *
 * The brief for this screen is "human-readable storage explanation", not
 * "disk analyser", so it answers in sentences first and shapes second. The
 * arithmetic and all of the hedging come from the core: the interface does
 * not decide what is reclaimable, and it never says "safe to delete".
 */

/** Warm, distinguishable, and consistent between the band and the legend. */
const SEGMENT_COLOURS = [
  'var(--honey)',
  'var(--moss)',
  'var(--clay)',
  'color-mix(in srgb, var(--honey) 55%, var(--paper))',
  'color-mix(in srgb, var(--moss) 55%, var(--paper))',
  'color-mix(in srgb, var(--clay) 45%, var(--paper))',
]

export function Space() {
  const { space, refreshSpace, go } = useStore()
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    void refreshSpace().then((ok) => setFailed(!ok))
  }, [refreshSpace])

  if (!space) {
    return failed ? (
      <Unavailable
        what="this volume"
        onRetry={() => {
          setFailed(false)
          void refreshSpace().then((ok) => setFailed(!ok))
        }}
      />
    ) : (
      <Waiting what="Measuring…" />
    )
  }

  const measured = space.areas.reduce((total, area) => total + area.bytes, 0)
  // Whatever the named areas did not account for. Shown, not hidden: a chart
  // that silently adds up to less than the disk is a chart that is lying.
  const unaccounted = Math.max(0, space.volume_used - measured)
  // `??` would let a zero through and divide by it.
  const largestTally = space.worth_checking[0]?.bytes || 1
  const anyIncomplete = space.areas.some((area) => !area.complete)

  return (
    <div className={styles.room}>
      <div className={styles.inner}>
        <p className={styles.summary}>{space.summary}</p>

        <div
          className={styles.band}
          role="img"
          aria-label={`Storage on this volume: ${space.areas
            .map((area) => `${area.label} ${bytes(area.bytes)}`)
            .join(', ')}`}
        >
          {space.areas.map((area, index) => (
            <span
              key={area.label}
              className={styles.segment}
              style={{
                flexGrow: area.bytes,
                background: SEGMENT_COLOURS[index % SEGMENT_COLOURS.length],
              }}
              title={`${area.label} — ${bytes(area.bytes)}`}
            />
          ))}
          {unaccounted > 0 && (
            <span
              className={styles.segment}
              style={{ flexGrow: unaccounted, background: 'var(--hollow)' }}
              title={`Everything else — ${bytes(unaccounted)}`}
            />
          )}
        </div>

        <ul className={styles.legend}>
          {space.areas.map((area, index) => (
            <li key={area.label} className={styles.legendItem}>
              <span
                className={styles.swatch}
                style={{ background: SEGMENT_COLOURS[index % SEGMENT_COLOURS.length] }}
                aria-hidden="true"
              />
              <span className={styles.legendLabel}>{area.label}</span>
              <span className={styles.legendValue}>
                {area.complete ? '' : 'at least '}
                {bytes(area.bytes)}
              </span>
            </li>
          ))}
          {unaccounted > 0 && (
            <li className={styles.legendItem}>
              <span
                className={styles.swatch}
                style={{ background: 'var(--hollow)' }}
                aria-hidden="true"
              />
              <span className={styles.legendLabel}>Everything else</span>
              <span className={styles.legendValue}>{bytes(unaccounted)}</span>
            </li>
          )}
        </ul>

        <section className={styles.section}>
          <h3 className={styles.sectionTitle}>Things Scuttle thinks are worth checking</h3>
          {space.worth_checking.length === 0 ? (
            <p className={styles.sectionNote}>
              Nothing yet. Rummage and this fills in.
            </p>
          ) : (
            <>
              <p className={styles.sectionNote}>
                From the last rummage. Sizes are what those findings take up, not a
                promise about what you can remove.
              </p>
              <ul className={styles.tallies}>
                {space.worth_checking.map((tally) => (
                  <li key={tally.category}>
                    <button
                      className={styles.tally}
                      onClick={() => go({ name: 'pile', category: tally.category })}
                    >
                      <span className={styles.tallyLabel}>{tally.label}</span>
                      <span className={styles.tallyBar}>
                        <span
                          className={styles.tallyFill}
                          style={{ width: `${(tally.bytes / largestTally) * 100}%` }}
                        />
                      </span>
                      <span className={styles.tallyValue}>{bytes(tally.bytes)}</span>
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </section>

        <p className={styles.footnote}>
          {bytes(space.volume_free)} free of {bytes(space.volume_total)}.
          {anyIncomplete &&
            ' Some folders were too large or too locked-down to measure completely, so those figures are floors rather than totals.'}
        </p>
      </div>
    </div>
  )
}
