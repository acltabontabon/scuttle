import { useMemo, useState } from 'react'

import { useStore } from '@/app/store'
import { bytes, bytesParts, scatter } from '@/lib/format'
import { Unavailable } from '@/components/Unavailable'
import { CANCELLED_CAVEAT, GLANCE_CAVEAT } from '@/features/background/phrasing'
import type { HiccupSummary } from '@/lib/types'
import { Scuttle } from '@/visuals/Scuttle'
import { Heap } from './Heap'

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
  const hiccups =
    findings.hiccups.permission_denied +
    findings.hiccups.unreadable +
    findings.hiccups.vanished

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
      {/*
        The masthead.

        Two poles across the full width rather than one narrow column pushed
        against the left edge: the total on one side, what to do about it on
        the other, and a great deal of air between them. The route out of here
        is set in the same typographic voice as everything else — a phrase
        marked with a honey rule, not a rounded rectangle borrowed from an
        operating system.
      */}
      <header className={styles.mast}>
        <div className={styles.poleLeft}>
          <p className={styles.total}>
            {total.value}
            <span className={styles.totalUnit}>{total.unit}</span>
          </p>
          <p className={styles.aside}>
            worth reviewing, across {findings.piles.length}{' '}
            {findings.piles.length === 1 ? 'pile' : 'piles'}. Nothing has been touched.
          </p>
        </div>

        {/*
            The suggestion, said in two short lines instead of a paragraph.
            The old block ran to three sentences of instruction in the top
            right and competed with the total for the eye; what a first-time
            reader needs here is the size of the offer and a way into it.
          */}
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
                Review {suggested === 1 ? 'suggestion' : 'suggestions'}
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

      {/*
          What produced these findings, when it was not an ordinary rummage.
          Without this, an empty Copies pile after a background check reads as
          "you have no duplicates" — when what actually happened is that
          nothing opened a file to find out.
        */}
      {findings.kind === 'glance' && <p className={styles.caveat}>{GLANCE_CAVEAT}</p>}
      {findings.cancelled && <p className={styles.caveat}>{CANCELLED_CAVEAT}</p>}

      <p className={styles.lede}>Choose a pile to see what&rsquo;s inside.</p>

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

      {/*
        The quiet end of the screen: what was looked at, how to look again,
        what is waiting in the drawer, and anything the scan could not reach.
        In normal flow, wrapping rather than overlapping.
      */}
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

        {hiccups > 0 && <ScanNotice hiccups={findings.hiccups} total={hiccups} />}
      </footer>

    </div>
  )
}

/**
 * What the scan could not reach.
 *
 * "a floor rather than a total" was accurate and meant nothing to anyone who
 * had not read the code. This says what happened and what it implies, and
 * keeps the breakdown behind a toggle so the footer stays a footer.
 */
function ScanNotice({ hiccups, total }: { hiccups: HiccupSummary; total: number }) {
  const [open, setOpen] = useState(false)
  const parts = [
    { n: hiccups.permission_denied, label: 'needed permission Scuttle does not have' },
    { n: hiccups.unreadable, label: 'could not be read' },
    { n: hiccups.vanished, label: 'disappeared mid-scan' },
  ].filter((part) => part.n > 0)

  return (
    <div className={styles.notice}>
      <p className={styles.noticeLine}>
        {total} {total === 1 ? 'location' : 'locations'} couldn&rsquo;t be scanned. Results
        may be incomplete.
        {parts.length > 0 && (
          <button
            className={styles.noticeToggle}
            onClick={() => setOpen((on) => !on)}
            aria-expanded={open}
          >
            {open ? 'Hide details' : 'Details'}
          </button>
        )}
      </p>
      {open && (
        <ul className={styles.noticeList}>
          {parts.map((part) => (
            <li key={part.label}>
              {part.n} {part.n === 1 ? 'location' : 'locations'} {part.label}
            </li>
          ))}
        </ul>
      )}
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
