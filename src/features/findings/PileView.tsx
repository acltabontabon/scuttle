import { useEffect, useMemo, useRef, useState } from 'react'

import { useStore } from '@/app/store'
import { MoveControls, MoveLine, MoveTrack } from '@/features/move/MoveStatus'
import { bytes, bytesParts } from '@/lib/format'
import type { Candidate, Category } from '@/lib/types'
import { CATEGORY_BLURB, RISK_WORD } from '@/visuals/CategoryMeta'
import { Glyph } from '@/visuals/Glyph'
import { Scuttle } from '@/visuals/Scuttle'
import { applyPick } from './selection'

import shared from './Findings.module.css'
import styles from './PileView.module.css'

/**
 * Inside one pile.
 *
 * Two parts. On the left, a standing plate: what this pile is, and a serif
 * total that climbs as you tick things, reusing the one piece of typography
 * the findings floor already made meaningful. On the right, the objects, laid
 * out as wide as the window allows.
 *
 * The old version was a single 760px column with the rest of the screen left
 * empty, and a bulk action that only covered findings Scuttle was already
 * confident about. Between them those two decisions meant the largest pile
 * most people have — four hundred screenshots, nothing Scuttle will vouch for
 * — could only be dealt with one row at a time, down a narrow channel, while
 * two thirds of the window sat unused.
 *
 * Every row is selectable except the genuinely protected ones. Scuttle
 * declining to *recommend* something is an opinion about what it should do
 * unprompted; it is not a lock on what you may decide. Ticking a box is the
 * decision, and the running total is there so it is never an abstract one.
 */
export function PileView({
  category,
  preselect,
}: {
  category: Category
  preselect?: 'suggested'
}) {
  const { findings, go, openDetail, quarantineMany, move, moving, cancelMove } = useStore()
  const pile = findings?.piles.find((p) => p.category === category)
  // Arriving from "Review suggestion" starts with Scuttle's picks ticked, so
  // the first thing shown is what it would act on — not a blank list and a
  // separate button to find out.
  const [picked, setPicked] = useState<ReadonlySet<string>>(() =>
    preselect === 'suggested' && pile
      ? new Set(
          pile.items
            .filter((item) => item.recommended_action === 'quarantine')
            .map((item) => item.id),
        )
      : new Set(),
  )
  /** Anchor for shift-click, so a run of four hundred is one gesture. */
  const anchor = useRef<string | null>(null)
  const actButton = useRef<HTMLButtonElement>(null)
  const controls = useRef<HTMLDivElement>(null)
  const title = useRef<HTMLHeadingElement>(null)

  const shown = useMemo(() => pile?.items ?? [], [pile])

  // The selection stays exactly as it was while work runs — the meter and the
  // list keep telling the truth about what was asked for. Nothing needs
  // clearing afterwards: what is chosen is always worked out from the items
  // that are still here, so whatever moved simply drops out of it, and
  // anything left behind stays ticked for another go.

  // Focus follows the button that was pressed: to the status when it appears,
  // and back to the action (or the pile's title) when it goes, so a keyboard
  // user is never left on something that no longer exists.
  const wasMoving = useRef(false)
  useEffect(() => {
    if (moving && !wasMoving.current) {
      controls.current?.focus({ preventScroll: true })
    } else if (!moving && wasMoving.current) {
      const target = actButton.current && !actButton.current.disabled ? actButton.current : title.current
      target?.focus({ preventScroll: true })
    }
    wasMoving.current = moving
  }, [moving])

  /**
   * Protected findings are the only ones held back, because those are the
   * ones the safety gate will refuse anyway — offering the tick would be
   * offering something that cannot happen.
   */
  const selectable = useMemo(() => shown.filter((item) => item.risk !== 'protected'), [shown])

  if (!pile) {
    return (
      <div className={shared.empty}>
        <Scuttle mood="shrug" size={84} />
        <p className={shared.emptyTitle}>That pile is gone.</p>
        <p className={shared.emptyLine}>
          Either you dealt with everything in it, or the last rummage found nothing there.
        </p>
        <button className={shared.againLink} onClick={() => go({ name: 'findings' })}>
          Back to the floor
        </button>
      </div>
    )
  }

  const chosen = selectable.filter((item) => picked.has(item.id))
  const chosenBytes = chosen.reduce((total, item) => total + item.size, 0)
  const allPicked = selectable.length > 0 && chosen.length === selectable.length
  const suggested = selectable.filter((item) => item.recommended_action === 'quarantine')
  const tally = bytesParts(chosenBytes)
  const share = pile.bytes > 0 ? Math.min(1, chosenBytes / pile.bytes) : 0

  /**
   * Plain click toggles one. Shift-click takes everything between the last
   * click and this one, which is the difference between selecting a burst of
   * three hundred screenshots and not bothering.
   *
   * The anchor is read *before* `setPicked` and never from inside it. React
   * defers an updater to render time, so an earlier version that read
   * `anchor.current` within the updater always saw the id assigned on the
   * line below — `from` and `id` were the same item, the run was never
   * measurable, and every shift-click quietly degraded into a plain toggle.
   */
  const click = (id: string, extend: boolean) => {
    const from = extend ? anchor.current : null
    anchor.current = id
    setPicked((current) => applyPick(selectable, current, id, from))
  }

  return (
    <div className={styles.pile}>
      <div className={styles.inner}>
        {/*
          The plate. Sticky, so the total and the action stay put while four
          hundred things scroll past them — a selection built by scrolling
          must never require scrolling back to act on.
        */}
        <aside className={styles.plate}>
          <button className={styles.back} onClick={() => go({ name: 'findings' })}>
            ← Everything
          </button>

          <span className={styles.crest} aria-hidden="true">
            <Glyph category={pile.category} size={58} />
          </span>

          <h2 className={styles.title} ref={title} tabIndex={-1}>
            {pile.title}
          </h2>
          <p className={styles.blurb}>{CATEGORY_BLURB[pile.category]}</p>
          <p className={styles.tally}>
            {pile.count} {pile.count === 1 ? 'thing' : 'things'} · {bytes(pile.bytes)} worth
            reviewing
          </p>

          {/*
            The meter.

            The same serif figure the floor uses for the grand total, here
            counting what you have chosen. It turns selecting into something
            with a visible result, which is the whole reason a person opened
            this application.
          */}
          <div className={styles.meter}>
            <p className={styles.meterValue} data-live={chosen.length > 0 || undefined}>
              {tally.value}
              <span className={styles.meterUnit}>{tally.unit}</span>
            </p>
            {moving ? (
              // The same thin line, now showing real progress.
              <MoveTrack snapshot={move.snapshot} pending={move.pending} />
            ) : (
              <div className={styles.gauge} aria-hidden="true">
                <span className={styles.gaugeFill} style={{ transform: `scaleX(${share})` }} />
              </div>
            )}
            <p className={styles.meterLine}>
              {moving ? (
                <MoveLine snapshot={move.snapshot} />
              ) : chosen.length === 0 ? (
                'nothing picked yet'
              ) : (
                `${chosen.length} of ${selectable.length} picked`
              )}
            </p>
          </div>

          {moving ? (
            <MoveControls
              snapshot={move.snapshot}
              pending={move.pending}
              onCancel={() => void cancelMove()}
              controlRef={controls}
            />
          ) : (
            <button
              ref={actButton}
              className={styles.act}
              disabled={chosen.length === 0}
              onClick={() => {
                // The selection is kept: it is cleared only once the outcome
                // is known and what moved has left the list.
                void quarantineMany(chosen.map((item) => item.id))
              }}
            >
              {chosen.length === 0
                ? 'Pick something first'
                : sweepLabel(chosen.length, chosenBytes)}
            </button>
          )}

          <p className={styles.actHint}>
            Nothing is deleted yet — empty the drawer to get the space back.
          </p>

          {selectable.length > 0 && (
            <div className={styles.picker}>
              <button
                className={styles.pickAll}
                disabled={moving}
                onClick={() => {
                  setPicked(allPicked ? new Set() : new Set(selectable.map((i) => i.id)))
                  anchor.current = null
                }}
              >
                {allPicked ? 'Clear' : `All ${selectable.length}`}
              </button>
              {suggested.length > 0 && suggested.length < selectable.length && (
                <button
                  className={styles.pickAll}
                  disabled={moving}
                  onClick={() => {
                    setPicked(new Set(suggested.map((i) => i.id)))
                    anchor.current = null
                  }}
                >
                  Scuttle&rsquo;s {suggested.length}
                </button>
              )}
              <span className={styles.pickTip}>Shift-click for a run</span>
            </div>
          )}
        </aside>

        {/*
          The objects, in as many columns as the window will take. A pile of
          four hundred is a field to sweep through, not a four-hundred-row
          scroll down one narrow channel.
        */}
        <ul className={styles.items}>
          {shown.map((item) => {
            const locked = item.risk === 'protected'
            const on = picked.has(item.id)
            return (
              <li key={item.id} className={styles.row} data-picked={on || undefined}>
                <input
                  type="checkbox"
                  className={styles.tick}
                  checked={on}
                  disabled={locked || moving}
                  onChange={() => undefined}
                  onClick={(event) => click(item.id, event.shiftKey)}
                  aria-label={
                    locked
                      ? `${item.display_name} — protected, Scuttle will not move it`
                      : `Select ${item.display_name}, ${bytes(item.size)}`
                  }
                  title={locked ? 'Protected. Scuttle will not move this.' : undefined}
                />
                <button className={styles.item} onClick={() => openDetail(item)}>
                  <span className={styles.itemGlyph}>
                    <Glyph category={item.category} size={26} />
                  </span>
                  <span className={styles.itemName}>{item.display_name}</span>
                  <span className={styles.itemSize}>
                    {bytes(item.size)}
                    {item.group.length > 1 && (
                      <span className={styles.itemWhole}>
                        of {bytes(item.group_bytes)}
                      </span>
                    )}
                  </span>
                  {item.remark && (
                    <span className={styles.itemRemark}>{firstLine(item.remark)}</span>
                  )}
                  <span className={styles.itemVerdict}>
                    <Verdict candidate={item} />
                  </span>
                </button>
              </li>
            )
          })}

          {pile.count > shown.length && (
            <li className={styles.more}>
              Showing {shown.length} of {pile.count}. Deal with these and the rest come up
              next time.
            </li>
          )}
        </ul>
      </div>
    </div>
  )
}

/**
 * The action's label. Says the count and the size, so the click is never a
 * surprise — "put all 6 in the drawer" is a different decision from "put it
 * in the drawer".
 */
export function sweepLabel(count: number, size: number): string {
  if (count === 1) return `Put it in the drawer · ${bytes(size)}`
  if (count === 2) return `Put both in the drawer · ${bytes(size)}`
  return `Put all ${count} in the drawer · ${bytes(size)}`
}

/**
 * Remarks are written as short paragraphs. A list row gets the opening one;
 * the rest is waiting in the detail sheet.
 */
function firstLine(remark: string): string {
  return remark.split('\n\n')[0] ?? remark
}

/**
 * Risk and group size, in words as well as colour.
 *
 * Confidence is deliberately *not* here: it belongs next to the evidence that
 * produced it, and a bare "High" floating in a list invites people to read it
 * as "safe", which is a different thing entirely.
 */
function Verdict({ candidate }: { candidate: Candidate }) {
  return (
    <>
      {candidate.group.length > 1 && (
        <span>{candidate.group.length} copies, keeping one ·</span>
      )}
      <span className={styles.riskDot} data-risk={candidate.risk} aria-hidden="true" />
      <span>{RISK_WORD[candidate.risk].toLowerCase()} risk</span>
    </>
  )
}
