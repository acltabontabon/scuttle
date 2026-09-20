import { useMemo, useState } from 'react'

import { useStore } from '@/app/store'
import { bytesParts, scatter } from '@/lib/format'
import { Unavailable } from '@/components/Unavailable'
import { Scuttle } from '@/visuals/Scuttle'
import { Heap } from './Heap'

import styles from './Findings.module.css'

/**
 * The floor.
 *
 * Scuttle empties its pockets: every pile is a heap of drawn objects sitting
 * on the paper, not a card in a dashboard. The only number given prominence is
 * the total, and it is phrased as "worth a look" rather than as a saving,
 * because Scuttle has not decided anything yet.
 */
export function Findings() {
  const { findings, go, scan, rummage, refreshFindings } = useStore()
  const [retrying, setRetrying] = useState(false)

  if (!findings) {
    return (
      <Unavailable
        what="the last rummage"
        onRetry={() => {
          if (retrying) return
          setRetrying(true)
          void refreshFindings().finally(() => setRetrying(false))
        }}
      />
    )
  }

  if (!findings.has_rummaged) {
    return (
      <div className={styles.empty}>
        <Scuttle mood="idle" size={92} />
        <p className={styles.emptyTitle}>Nothing to show yet.</p>
        <button className={styles.againLink} onClick={() => void rummage()}>
          Rummage
        </button>
      </div>
    )
  }

  if (findings.piles.length === 0) {
    return (
      <NothingFound
        seed={String(findings.finished_unix ?? findings.files_seen)}
        onRummage={() => void rummage()}
      />
    )
  }

  const total = bytesParts(findings.total_bytes)
  const hiccups =
    findings.hiccups.permission_denied +
    findings.hiccups.unreadable +
    findings.hiccups.vanished

  return (
    <div className={styles.floor}>
      <header className={styles.opening}>
        <p className={styles.total}>
          {total.value}
          <span className={styles.totalUnit}>{total.unit}</span>
        </p>
        <p className={styles.aside}>
          worth a look, across {findings.piles.length}{' '}
          {findings.piles.length === 1 ? 'pile' : 'piles'}.
        </p>
        {hiccups > 0 && (
          <p className={styles.hiccups}>
            {hiccups} {hiccups === 1 ? 'place' : 'places'} Scuttle could not read.
            Whatever is in there was left alone.
          </p>
        )}
      </header>

      <div className={styles.scatter}>
        {findings.piles.map((pile, index) => (
          <div key={pile.category} className={styles.slot}>
            <Heap
              pile={pile}
              index={index}
              onOpen={() => go({ name: 'pile', category: pile.category })}
            />
          </div>
        ))}
      </div>

      <p className={styles.again}>
        <span>
          {scan.summary
            ? `${scan.summary.files_seen.toLocaleString()} files looked at.`
            : `${findings.files_seen.toLocaleString()} files looked at.`}
        </span>
        <button className={styles.againLink} onClick={() => void rummage()}>
          Rummage again
        </button>
      </p>
    </div>
  )
}

/**
 * The empty state, which gets real effort.
 *
 * A cleanup utility that finds nothing has exactly one honest thing to say,
 * and this is the moment most of them start inventing problems instead.
 */
function NothingFound({ seed, onRummage }: { seed: string; onRummage: () => void }) {
  // Rare enough to be a surprise rather than a bit. Seeded from the scan so
  // the same empty result reads the same way every time you look at it —
  // an Easter egg that flickers as the clock ticks is just a glitch.
  const oddity = useMemo(() => {
    const roll = scatter(seed)
    if (roll > 0.82) return 'It did drag out one empty folder, then lost interest.'
    if (roll > 0.64) return 'It checked twice.'
    return null
  }, [seed])

  return (
    <div className={styles.empty}>
      <Scuttle mood="asleep" size={104} />
      <p className={styles.emptyTitle}>Nothing interesting.</p>
      <p className={styles.emptyLine}>Your computer is suspiciously tidy.</p>
      {oddity && <p className={styles.sock}>{oddity}</p>}
      <button className={styles.againLink} onClick={onRummage} style={{ marginTop: 'var(--step)' }}>
        Rummage again
      </button>
    </div>
  )
}
