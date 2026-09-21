import { useEffect, useRef, useState } from 'react'

import { useStore } from '@/app/store'
import { bytes } from '@/lib/format'
import type { CautionKind, MovePlan, MoveRequest, PlannedItem } from '@/lib/types'

import { IMPACT_WORDS, canConfirm, confirmLabel, restoreLines, shapeLine } from './reviewing'
import styles from './Review.module.css'

/**
 * What a move will do, before it does it.
 *
 * Every way of moving something opens this: a hand-picked batch, one item from
 * its detail sheet, a group, Scuttle's own suggestions. It shows the exact
 * path of each thing, whether it is a file or a whole folder, why Scuttle
 * noticed it, what it is unsure of, where it goes and how it comes back.
 * Cautions are accepted here once each, for the whole move — never file by
 * file — and the core refuses anything whose caution was not accepted,
 * whatever this screen does.
 */
export function Review() {
  const { review, closeReview, confirmReview } = useStore()
  // Keyed by the request it was given for, so a new review starts with
  // nothing accepted by construction rather than by an effect resetting it.
  const [accepted, setAccepted] = useState<{ for: MoveRequest | null; kinds: Set<CautionKind> }>({
    for: null,
    kinds: new Set(),
  })
  const sheetRef = useRef<HTMLElement>(null)
  const cancelRef = useRef<HTMLButtonElement>(null)
  const plan = review?.plan ?? null

  useEffect(() => {
    if (!review) return
    const returnTo = document.activeElement as HTMLElement | null
    cancelRef.current?.focus()
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.stopPropagation()
        closeReview()
        return
      }
      if (event.key !== 'Tab' || !sheetRef.current) return
      const focusable = sheetRef.current.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])',
      )
      if (focusable.length === 0) return
      const first = focusable[0]!
      const last = focusable[focusable.length - 1]!
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault()
        first.focus()
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      returnTo?.focus?.()
    }
  }, [review, closeReview])

  if (!review) return null

  const acknowledged: ReadonlySet<CautionKind> =
    accepted.for === review.request ? accepted.kinds : new Set()
  const toggle = (kind: CautionKind) => {
    const next = new Set(acknowledged)
    if (next.has(kind)) next.delete(kind)
    else next.add(kind)
    setAccepted({ for: review.request, kinds: next })
  }

  return (
    <>
      <div className={styles.scrim} onClick={closeReview} />
      <section
        ref={sheetRef}
        className={styles.sheet}
        role="dialog"
        aria-modal="true"
        aria-labelledby="review-title"
      >
        <header className={styles.head}>
          <h2 id="review-title" className={styles.title}>
            Before anything moves
          </h2>
          {plan && <p className={styles.summary}>{summary(plan)}</p>}
        </header>

        <div className={styles.body}>
          {!plan && <p className={styles.loading}>Checking exactly what would move…</p>}

          {plan && (
            <>
              <ul className={styles.items}>
                {plan.items.map((item) => (
                  <Item key={`${item.finding_id}-${item.path}`} item={item} />
                ))}
              </ul>

              {plan.cautions.length > 0 && (
                <fieldset className={styles.cautions}>
                  <legend className={styles.sectionTitle}>Worth knowing first</legend>
                  {plan.cautions.map((group) => (
                    <label key={group.kind} className={styles.caution}>
                      <input
                        type="checkbox"
                        checked={acknowledged.has(group.kind)}
                        onChange={() => toggle(group.kind)}
                      />
                      <span>
                        <span className={styles.cautionHead}>{group.headline}</span>
                        {plan.ready > 1 && (
                          <span className={styles.cautionCount}>
                            {' '}
                            Applies to {group.count} of {plan.ready}.
                          </span>
                        )}
                        <span className={styles.cautionAccept}> I understand, move it anyway.</span>
                      </span>
                    </label>
                  ))}
                </fieldset>
              )}

              <section className={styles.where}>
                <h3 className={styles.sectionTitle}>Where it goes</h3>
                <p className={`${styles.path} selectable`}>{plan.drawer}</p>
                {restoreLines(plan).map((line) => (
                  <p key={line} className={styles.restore}>
                    {line}
                  </p>
                ))}
              </section>
            </>
          )}
        </div>

        <footer className={styles.actions}>
          <button ref={cancelRef} className={styles.action} onClick={closeReview}>
            {plan && plan.ready === 0 ? 'Close' : 'Cancel'}
          </button>
          {plan && plan.ready > 0 && (
            <button
              className={`${styles.action} ${styles.primary}`}
              data-application={plan.items.some(
                (i) => i.status === 'ready' && i.impact === 'application_install',
              )}
              disabled={!canConfirm(plan, acknowledged)}
              onClick={() => void confirmReview([...acknowledged])}
            >
              {confirmLabel(plan)}
            </button>
          )}
        </footer>
      </section>
    </>
  )
}

function summary(plan: MovePlan): string {
  const refused = plan.items.filter((i) => i.status === 'refused').length
  if (plan.ready === 0) {
    return refused === 1 ? 'This one cannot move. Here is why.' : 'None of these can move. Here is why.'
  }
  const what = plan.ready === 1 ? 'One thing' : `${plan.ready} things`
  const tail = refused > 0 ? ` ${refused} will stay where ${refused === 1 ? 'it is' : 'they are'}.` : ''
  return `${what}, ${bytes(plan.ready_bytes)}, would go to the Drawer.${tail}`
}

function Item({ item }: { item: PlannedItem }) {
  const reasons = item.reasons.filter((r) => !r.negative).slice(0, 3)
  const against = item.reasons.filter((r) => r.negative).slice(0, 3)
  return (
    <li className={styles.item} data-status={item.status}>
      <div className={styles.itemHead}>
        <span className={styles.itemName}>{item.display_name}</span>
        <span className={styles.itemSize}>{bytes(item.size)}</span>
      </div>
      {item.path && <p className={`${styles.path} selectable`}>{item.path}</p>}
      <p className={styles.shape}>
        {shapeLine(item)} · {IMPACT_WORDS[item.impact]}
      </p>
      {item.status !== 'ready' && item.note && <p className={styles.note}>{item.note}</p>}
      {item.status === 'ready' && (
        <>
          {reasons.length + against.length > 0 && (
            <ul className={styles.reasons}>
              {reasons.map((r) => (
                <li key={r.summary}>
                  <span aria-hidden="true">✓</span> {r.summary}
                </li>
              ))}
              {against.map((r) => (
                <li key={r.summary} data-negative="true">
                  <span aria-hidden="true">✗</span> {r.summary}
                </li>
              ))}
            </ul>
          )}
          {item.cautions.length > 0 && (
            <p className={styles.itemCautions}>
              {item.cautions.map((c) => c.detail).join(' ')}
            </p>
          )}
        </>
      )}
    </li>
  )
}
