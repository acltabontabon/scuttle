import { useStore } from '@/app/store'
import { bytes } from '@/lib/format'
import type { Candidate, Category } from '@/lib/types'
import { CATEGORY_BLURB, RISK_WORD } from '@/visuals/CategoryMeta'
import { Glyph } from '@/visuals/Glyph'
import { Scuttle } from '@/visuals/Scuttle'

import shared from './Findings.module.css'
import styles from './PileView.module.css'

/**
 * Inside one pile.
 *
 * Clarity beats composition here: this is where someone decides what to do, so
 * the objects line up and say what they are. Each row shows the thing, its
 * size, Scuttle's one line about it, and the verdict — nothing else, because
 * everything else is one click away in the detail sheet.
 */
export function PileView({ category }: { category: Category }) {
  const { findings, go, openDetail } = useStore()
  const pile = findings?.piles.find((p) => p.category === category)

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

  return (
    <div className={styles.pile}>
      <header className={styles.head}>
        <div className={styles.headText}>
          <button className={styles.back} onClick={() => go({ name: 'findings' })}>
            ← Everything
          </button>
          <h2 className={styles.title}>{pile.title}</h2>
          <p className={styles.blurb}>{CATEGORY_BLURB[pile.category]}</p>
          <p className={styles.tally}>
            {pile.count} {pile.count === 1 ? 'thing' : 'things'} · {bytes(pile.bytes)}
            {pile.actionable > 0
              ? ` · ${pile.actionable} Scuttle would act on`
              : ' · nothing Scuttle suggests acting on'}
          </p>
        </div>
      </header>

      <ul className={styles.items}>
        {pile.items.map((item) => (
          <li key={item.id}>
            <button className={styles.item} onClick={() => openDetail(item)}>
              <span className={styles.itemGlyph}>
                <Glyph category={item.category} size={26} />
              </span>
              <span className={styles.itemName}>{item.display_name}</span>
              <span className={styles.itemSize}>{bytes(item.size)}</span>
              {item.remark && (
                <span className={styles.itemRemark}>{firstLine(item.remark)}</span>
              )}
              <span className={styles.itemVerdict}>
                <Verdict candidate={item} />
              </span>
            </button>
          </li>
        ))}
      </ul>

      {pile.count > pile.items.length && (
        <p className={styles.more}>
          Showing {pile.items.length} of {pile.count}. Deal with these and the rest will
          come up next time.
        </p>
      )}
    </div>
  )
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
      {candidate.group.length > 1 && <span>{candidate.group.length} copies ·</span>}
      <span className={styles.riskDot} data-risk={candidate.risk} aria-hidden="true" />
      <span>{RISK_WORD[candidate.risk].toLowerCase()} risk</span>
    </>
  )
}
