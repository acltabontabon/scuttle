import { useEffect, useRef, useState } from 'react'

import { useStore } from '@/app/store'
import { bytes, findingBytes } from '@/lib/format'
import { Glyph } from '@/visuals/Glyph'
import { Icon } from '@/visuals/Icon'
import { Scuttle, type Mood } from '@/visuals/Scuttle'
import { outcomeAside, outcomeLine, phaseLine } from './phrasing'
import { handOff } from './handoff'

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

  /**
   * Once a rummage finishes in front of you, move along. The pause is long
   * enough to read the outcome line and short enough not to be a wait.
   *
   * The rule itself lives in `handOff`, where it can be stated and tested.
   * A ref is the right home for the flag it carries: it only has to survive
   * the renders between a rummage starting and ending, and both of those
   * happen while this screen is mounted. Arriving here after the fact starts
   * it at `false` — precisely the case that must not navigate anywhere.
   */
  const sawItRun = useRef(scan.status === 'running')
  useEffect(() => {
    const next = handOff(scan.status, found, sawItRun.current)
    sawItRun.current = next.sawItRun
    if (!next.handOver) return
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

  if (scan.status === 'idle') {
    return (
      <div className={styles.welcome}>
        <div className={styles.hero}>
          <div className={styles.heroCopy}>
            <p className={styles.eyebrow}>A little room to breathe</p>
            <h1 className={styles.heroTitle}>Less clutter.<br /><em>More possibility.</em></h1>
            <p className={styles.heroLine}>Forgotten downloads, old caches, the leftovers of apps long gone. Let Scuttle find what&rsquo;s taking up space.</p>
            <button className={styles.rummage} onClick={() => void rummage()}>
              Rummage <Icon name="arrow" />
            </button>
            <p className={styles.promise}><Icon name="shield" size={16} /> Just a look. Your files stay put until you decide.</p>
            {findings?.has_rummaged && (
              <button className={styles.lastFindings} onClick={() => go({ name: 'findings' })}>
                View your last findings <Icon name="arrow" size={15} />
              </button>
            )}
          </div>
          <div className={styles.heroArt} aria-hidden="true">
            <div className={styles.orbit} />
            <span className={styles.artGhost}><Glyph category="ghosts" size={49} /></span>
            <span className={styles.artCopies}><Glyph category="copies" size={42} /></span>
            <span className={styles.artCache}><Glyph category="caches" size={54} /></span>
            <span className={styles.artShot}><Glyph category="screenshots" size={46} /></span>
            <div className={styles.heroCreature}><Scuttle mood="idle" size={190} /></div>
            <span className={styles.artCaption}>A curious little helper.<br />A tidier little corner of your world.</span>
          </div>
        </div>
        <div className={styles.steps}>
          {[
            ['01', 'Find the forgotten', 'Scuttle looks through familiar folders for things worth a second look.'],
            ['02', 'You make the call', 'See what turned up, why it was noticed, and choose what stays.'],
            ['03', 'Keep a way back', 'Moved files wait in the drawer. Restore them while they’re still there.'],
          ].map(([number, title, line]) => (
            <div className={styles.step} key={number}>
              <span className={styles.stepNumber}>{number}</span>
              <div><h2>{title}</h2><p>{line}</p></div>
            </div>
          ))}
        </div>
      </div>
    )
  }

  return (
    <div className={styles.stage}>
      <div className={styles.centre}>
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
                files examined
              </span>
              <span>·</span>
              <span>
                <span className={styles.countValue}>{found}</span>{' '}
                {found === 1 ? 'finding' : 'findings'} so far
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
      <span className={styles.catchSize}>{findingBytes(latest)}</span>
    </p>
  )
}
