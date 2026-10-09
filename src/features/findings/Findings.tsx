import { OrganizationOffers } from '@/features/organization/Organization'
import { useMemo, useState } from 'react'

import { useStore } from '@/app/store'
import { bytes, bytesParts, scatter } from '@/lib/format'
import { Unavailable } from '@/components/Unavailable'
import { CANCELLED_CAVEAT, GLANCE_CAVEAT } from '@/features/background/phrasing'
import { Scuttle } from '@/visuals/Scuttle'
import { Icon } from '@/visuals/Icon'
import { Heap } from './Heap'
import { ScanNotice } from './ScanNotice'

import styles from './Findings.module.css'

/**
 * The floor.
 *
 * Scuttle empties its pockets: every pile is a heap of drawn objects sitting
 * on the paper, not a card in a dashboard. The only number given prominence is
 * the total, and it is phrased as "worth reviewing" rather than as a saving,
 * because Scuttle has not decided anything yet — and because the bytes only
 * come back at the far end of the drawer, not here.
 */
export function Findings() {
  return <div className={styles.withOrganization}>
    <div className={styles.organization}><OrganizationOffers /></div>
    <div className={styles.findingsBody}><FindingsBody /></div>
  </div>
}

function FindingsBody() {
  const { findings, go, scan, rummage, refreshFindings, drawer } = useStore()
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

  const hiccups = findings.hiccups.permission_denied + findings.hiccups.unreadable + findings.hiccups.vanished

  if (findings.piles.length === 0 && hiccups > 0) {
    return (
      <div className={styles.incompleteEmpty}>
        <Scuttle mood="shrug" size={84} />
        <h1 className={styles.emptyTitle}>Nothing found in the areas checked.</h1>
        <p className={styles.emptyLine}>Some places were skipped, so this isn&rsquo;t a complete look at your computer.</p>
        <ScanNotice hiccups={findings.hiccups} />
        <button className={styles.againLink} onClick={() => void rummage()}>Scan again</button>
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
  const suggested = findings.piles.reduce((n, pile) => n + pile.confident_count, 0)
  const suggestedBytes = findings.piles.reduce((n, pile) => n + pile.confident_bytes, 0)
  const held = drawer?.items.length ?? 0

  // Weight is carried by how big each heap is drawn, not by how many grid
  // cells it occupies. Giving the heaviest pile a double-width cell left a
  // hole in the middle of the floor and told the reader nothing that the size
  // of the heap itself does not already say.
  const heaviestBytes = findings.piles.reduce((most, pile) => Math.max(most, pile.bytes), 0)

  // Where the suggestions actually are. The header's action opens the pile
  // holding the most of them rather than inventing a cross-pile review screen;
  // every pile that has any is marked, so the rest are findable from here.
  const suggestedPiles = findings.piles.filter((pile) => pile.confident_count > 0)
  const reviewIn = suggestedPiles.reduce<(typeof findings.piles)[number] | undefined>(
    (best, pile) => (pile.confident_bytes > (best?.confident_bytes ?? -1) ? pile : best),
    undefined,
  )

  return (
    <div className={styles.floor}>
      <header className={styles.mast}>
        <div className={styles.poleLeft}>
          <p className={styles.eyebrow}>The collection · Your findings</p>
          <h1 className={styles.headline}>Look what turned up.</h1>
          <p className={styles.total}>
            {findings.piles.some((pile) => pile.bytes_is_lower_bound) && <span className={styles.totalUnit}>at least </span>}
            {total.value}
            <span className={styles.totalUnit}>{total.unit}</span>
          </p>
          <p className={styles.aside}>worth a second look, across {findings.piles.length} {findings.piles.length === 1 ? 'category' : 'categories'}.</p>
          <p className={styles.safety}><Icon name="shield" size={14} /> Nothing has been moved or deleted.</p>
        </div>
        <div className={styles.poleRight}>
          {suggested > 0 && reviewIn ? (
            <div className={styles.offer}>
              <p className={styles.offerLine}>
                {suggested} suggested {suggested === 1 ? 'item' : 'items'} ·{' '}
                {bytes(suggestedBytes)}
              </p>
              <button
                className={styles.offerAction}
                onClick={() =>
                  go({ name: 'pile', category: reviewIn.category, preselect: 'suggested' })
                }
              >
                Review {suggested === 1 ? 'suggestion' : 'suggestions'} <Icon name="arrow" size={16} />
              </button>
              {suggestedPiles.length > 1 && (
                <p className={styles.offerNote}>
                  across {suggestedPiles.length} piles, marked below
                </p>
              )}
            </div>
          ) : (
            <p className={styles.offerNote}>
              Nothing suggested — these are yours to judge.
            </p>
          )}
        </div>
      </header>
      {findings.kind === 'glance' && <p className={styles.caveat}>{GLANCE_CAVEAT}</p>}
      {findings.cancelled && <p className={styles.caveat}>{CANCELLED_CAVEAT}</p>}

      <div className={styles.lede}>
        <h2>A place for everything</h2>
        <p>Open a category. Take a look. You decide what stays.</p>
      </div>

      <div className={styles.scatter}>
        {findings.piles.map((pile, index) => (
          <div key={pile.category} className={styles.slot}>
            <Heap
              pile={pile}
              index={index}
              weight={heaviestBytes > 0 ? pile.bytes / heaviestBytes : 1}
              onOpen={() => go({ name: 'pile', category: pile.category })}
            />
          </div>
        ))}
      </div>
      <footer className={styles.foot}>
        <p className={styles.footRow}>
          <span className={styles.footFact}>
            {(scan.summary?.files_seen ?? findings.files_seen).toLocaleString()} files
            examined
          </span>
          <button className={styles.footAction} onClick={() => void rummage()}>
            Rummage again
          </button>
          {held > 0 && (
            <button className={styles.footAction} onClick={() => go({ name: 'drawer' })}>
              Drawer · {bytes(drawer?.held_bytes ?? 0)} held
            </button>
          )}
        </p>

        <ScanNotice hiccups={findings.hiccups} />
      </footer>

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
      <p className={styles.emptyTitle}>Nothing to clear out.</p>
      <p className={styles.emptyLine}>Nothing needs a cleanup review in the areas checked.</p>
      {oddity && <p className={styles.sock}>{oddity}</p>}
      <button className={styles.againLink} onClick={onRummage} style={{ marginTop: 'var(--step)' }}>
        Rummage again
      </button>
    </div>
  )
}
