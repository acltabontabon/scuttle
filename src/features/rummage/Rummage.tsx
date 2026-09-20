import { useEffect, useState } from 'react'

import { useStore } from '@/app/store'
import { bytes } from '@/lib/format'
import { Glyph } from '@/visuals/Glyph'
import { Scuttle, type Mood } from '@/visuals/Scuttle'
import { outcomeAside, outcomeLine, phaseLine } from './phrasing'

import styles from './Rummage.module.css'

/**
 * Home.
 *
 * One dominant action and a great deal of nothing. No disk percentage, no
 * health score, no counters — those all arrive *after* the user has asked a
 * question, which is the only time they mean anything.
 */
export function Rummage() {
  const { scan, rummage, cancel, go, findings } = useStore()
  const [showTechnical, setShowTechnical] = useState(false)

  const running = scan.status === 'running'
  const found = scan.foundCount

  // Once a rummage finishes with something to show, move along. The pause is
  // long enough to read the outcome line and short enough not to be a wait.
  useEffect(() => {
    if (scan.status !== 'done' || found === 0) return
    const timer = window.setTimeout(() => go({ name: 'findings' }), 1500)
    return () => window.clearTimeout(timer)
  }, [scan.status, found, go])

  // Rare on purpose: something genuinely enormous turned up, and Scuttle is
  // visibly having trouble with it.
  const ABSURDLY_LARGE = 10 * 1024 ** 3
  const heaviest = scan.heaviestBytes

  const mood: Mood = running
    ? 'rummaging'
    : scan.status === 'done' && found === 0
      ? 'asleep'
      : scan.status === 'done'
        ? heaviest >= ABSURDLY_LARGE
          ? 'straining'
          : 'found'
        : scan.status === 'cancelled'
          ? 'shrug'
          : 'idle'

  return (
    <div className={styles.stage}>
      <div className={styles.centre}>
        {scan.status === 'idle' && (
          <>
            <h1 className={styles.title}>Scuttle</h1>
            <p className={styles.line}>Your computer leaves stuff everywhere.</p>
            <button className={styles.rummage} onClick={() => void rummage()}>
              Rummage
            </button>
            {findings?.has_rummaged && (
              <button
                className={styles.detailToggle}
                onClick={() => go({ name: 'findings' })}
              >
                Or look at what turned up last time
              </button>
            )}
          </>
        )}

        {running && (
          <div className={styles.working}>
            <p className={styles.phase} key={phaseLine(scan.phase)}>
              {phaseLine(scan.phase)}
            </p>

            {scan.latest && <LatestCatch />}

            <div className={styles.counts}>
              <span>
                <span className={styles.countValue}>
                  {scan.progress.files_seen.toLocaleString()}
                </span>{' '}
                looked at
              </span>
              <span>·</span>
              <span>
                <span className={styles.countValue}>{found}</span>{' '}
                {found === 1 ? 'thing' : 'things'} so far
              </span>
            </div>

            <div className={styles.stopRow}>
              <button className={styles.stop} onClick={() => void cancel()}>
                Stop
              </button>
              <span aria-hidden="true" style={{ color: 'var(--ink-ghost)' }}>
                ·
              </span>
              <button
                className={styles.detailToggle}
                onClick={() => setShowTechnical((on) => !on)}
                aria-expanded={showTechnical}
              >
                {showTechnical ? 'Less detail' : 'More detail'}
              </button>
            </div>

            {showTechnical && (
              <dl className={styles.technical}>
                <dt>scan</dt>
                <dd>{scan.scanId?.slice(0, 8) ?? '—'}</dd>
                <dt>phase</dt>
                <dd>{scan.phase.phase}</dd>
                <br />
                <dt>area</dt>
                <dd>{scan.progress.current_area || '—'}</dd>
                <br />
                <dt>read</dt>
                <dd>{bytes(scan.progress.bytes_seen)}</dd>
                <dt>files</dt>
                <dd>{scan.progress.files_seen.toLocaleString()}</dd>
              </dl>
            )}
          </div>
        )}

        {(scan.status === 'done' || scan.status === 'cancelled') && (
          <div className={styles.working}>
            <p className={styles.phase}>
              {outcomeLine(found, scan.status === 'cancelled')}
            </p>
            <p className={styles.line}>
              {outcomeAside(found, scan.status === 'cancelled')}
            </p>
            <div className={styles.stopRow}>
              {found > 0 && (
                <button className={styles.stop} onClick={() => go({ name: 'findings' })}>
                  Show me
                </button>
              )}
              <button className={styles.detailToggle} onClick={() => void rummage()}>
                Rummage again
              </button>
            </div>
          </div>
        )}

        {scan.status === 'failed' && (
          <div className={styles.working}>
            <p className={styles.phase}>That did not work.</p>
            <p className={styles.line}>{scan.error}</p>
            <button className={styles.detailToggle} onClick={() => void rummage()}>
              Try again
            </button>
          </div>
        )}
      </div>

      <div className={styles.floor}>
        <div className={styles.floorLine} />
        <div className={styles.creature} data-pacing={running}>
          <Scuttle mood={mood} size={running ? 74 : 86} />
        </div>
        {running && scan.roots.length > 0 && (
          <p className={styles.roots}>
            {scan.roots.map((root) => root.label).join(' · ')}
          </p>
        )}
      </div>
    </div>
  )
}

/**
 * The most recent thing to turn up, shown one at a time.
 *
 * This is the only running commentary: a list would turn the wait into a
 * spreadsheet, and the findings screen does that job properly in a moment.
 */
function LatestCatch() {
  const { scan } = useStore()
  const latest = scan.latest
  if (!latest) return null

  return (
    <p className={styles.catch} key={latest.id}>
      <Glyph category={latest.category} size={19} />
      <span className={styles.catchName}>{latest.display_name}</span>
      <span className={styles.catchSize}>{bytes(latest.size)}</span>
    </p>
  )
}
