import { useCallback, useEffect, useState } from 'react'

import { useStore } from '@/app/store'
import { bytes } from '@/lib/format'
import { Unavailable } from '@/components/Unavailable'
import { Scuttle } from '@/visuals/Scuttle'

import styles from './Space.module.css'

/**
 * Where the space went.
 *
 * The brief for this screen is "human-readable storage explanation", not
 * "disk analyser", so it answers in sentences first and shapes second. The
 * arithmetic and all of the hedging come from the core: the interface does
 * not decide what is reclaimable, and it never says "safe to delete".
 */

/**
 * The first measurement, which is the only one anybody waits for.
 *
 * Shaped like the screen it is about to become — a headline, a band, a legend
 * — so the layout does not jump when the real figures arrive. It can actually
 * animate now: the measuring runs on a worker rather than on the thread that
 * draws the window, which is what used to make this a freeze rather than a
 * wait.
 */
function Measuring() {
  return (
    <div className={styles.room}>
      <div className={styles.waiting} role="status">
        <Scuttle mood="rummaging" size={72} />
        <p className={styles.waitingLine}>Working out where it all went.</p>
        <div className={styles.ghostBand} aria-hidden="true">
          <span style={{ flexGrow: 5 }} />
          <span style={{ flexGrow: 3 }} />
          <span style={{ flexGrow: 2 }} />
          <span style={{ flexGrow: 8 }} />
        </div>
        <p className={styles.waitingNote}>
          Reading folder sizes. Big ones take a moment.
        </p>
      </div>
    </div>
  )
}

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
  const [measuring, setMeasuring] = useState(false)

  /** Asked for by hand, from the masthead. */
  const measure = useCallback(() => {
    setMeasuring(true)
    setFailed(false)
    void refreshSpace().then((ok) => {
      setFailed(!ok)
      setMeasuring(false)
    })
  }, [refreshSpace])

  /*
   * Measure only when there is nothing at all to show.
   *
   * This used to re-walk the disk on every single visit, which on a full
   * volume meant a two second wait each time Space was opened. The figures
   * are refreshed at the moments that actually change them — emptying the
   * drawer does it — and the masthead carries a "Measure again" for
   * everything else, so opening this screen costs nothing by default.
   *
   * Nothing is set synchronously here: the flag for a first measurement is
   * derived from having no figures yet, and the only state this writes is
   * written in the callback, once the answer is back and if the screen is
   * still open.
   */
  useEffect(() => {
    if (space !== null) return
    let live = true
    void refreshSpace().then((ok) => {
      if (live) setFailed(!ok)
    })
    return () => {
      live = false
    }
  }, [space, refreshSpace])

  if (!space) {
    return failed ? (
      <Unavailable
        what="this volume"
        onRetry={measure}
      />
    ) : (
      <Measuring />
    )
  }

  const measured = space.areas.reduce((total, area) => total + area.bytes, 0)
  // Whatever the named areas did not account for. Shown, not hidden: a chart
  // that silently adds up to less than the disk is a chart that is lying.
  const unaccounted = Math.max(0, space.volume_used - measured)
  // `??` would let a zero through and divide by it.
  const largestTally = space.worth_checking[0]?.bytes || 1
  const anyIncomplete = space.areas.some((area) => !area.complete)

  // The core writes the summary as a headline and a qualifying sentence,
  // separated by a blank line. Setting them as one block left a tall grey
  // paragraph doing the work a masthead should do; split, the figure can
  // carry the weight and the hedge can sit quietly beneath it.
  const [headline, ...rest] = space.summary.split('\n\n')
  const aside = rest.join(' ')

  return (
    <div className={styles.room}>
      <div className={styles.inner}>
        <header className={styles.mast}>
          <div className={styles.poleLeft}>
            <p className={styles.headline}>{headline}</p>
            {aside && <p className={styles.aside}>{aside}</p>}
          </div>
          {/*
            The free-space figure was buried in a footnote under everything
            else. It is the counterweight this headline wants, and it belongs
            where the eye already is.
          */}
          <div className={styles.free}>
            <span className={styles.freeValue}>{bytes(space.volume_free)}</span>
            <span className={styles.freeOf}>free of {bytes(space.volume_total)}</span>
            {/*
              Measuring is no longer automatic on every visit, so it has to be
              askable. While it runs the figures on screen stay exactly where
              they are — they are still true, just possibly a little old — and
              only this line changes.
            */}
            <button
              className={styles.remeasure}
              onClick={measure}
              disabled={measuring}
              data-busy={measuring || undefined}
            >
              {measuring ? 'Measuring…' : 'Measure again'}
            </button>
          </div>
        </header>

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
                {!area.complete && <span className={styles.legendQualifier}>at least </span>}
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
              <span className={styles.legendLabel}>
                Everything else
                <span className={styles.legendQualifier}>
                  {' '}
                  — the rest of the volume, which Scuttle does not look through
                </span>
              </span>
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
From the last rummage, and the same figures the piles show: what you could
                free by acting on everything in each one, keeping a copy of anything
                duplicated.
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

        {anyIncomplete && (
          <p className={styles.footnote}>
Marked <em>at least</em> where a folder was too large or too locked-down to
            finish measuring. Those figures are floors, not totals.
          </p>
        )}
      </div>
    </div>
  )
}
