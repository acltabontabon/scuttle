import { useEffect, useRef, useState } from 'react'

import { useStore } from '@/app/store'
import { bytes, shortPath, whenish } from '@/lib/format'
import type { Candidate } from '@/lib/types'
import {
  ACTION_MEANING,
  ACTION_WORD,
  CONFIDENCE_WORD,
  RISK_MEANING,
  RISK_WORD,
} from '@/visuals/CategoryMeta'
import { Glyph } from '@/visuals/Glyph'

import styles from './Detail.module.css'

/**
 * One finding, in full.
 *
 * This is the screen where the product either earns trust or does not, so it
 * is allowed to be more conventional than the rest: the reader is deciding
 * something, and composition should not get in the way of that.
 *
 * Everything shown here comes from the core. The interface does not compute a
 * verdict, soften a warning or invent a reason.
 */
export function Detail() {
  const { detail, openDetail, quarantine, keep, ignore, reveal } = useStore()
  // Keyed by finding rather than a bare boolean: opening a different finding
  // then closes the menu by construction, with no effect to reset it.
  const [menuOpenFor, setMenuOpenFor] = useState<string | null>(null)
  const closeRef = useRef<HTMLButtonElement>(null)
  const sheetRef = useRef<HTMLElement>(null)
  const returnFocusTo = useRef<HTMLElement | null>(null)

  // Escape closes the sheet, and focus stays inside it while it is open. A
  // dialog that announces `aria-modal` and then lets Tab wander into the
  // greyed-out page behind it is worse than one that does not announce it.
  useEffect(() => {
    if (!detail) return
    // Remember where focus came from, so closing the sheet puts it back on the
    // row that opened it rather than dropping it on the body.
    returnFocusTo.current = document.activeElement as HTMLElement | null
    closeRef.current?.focus()

    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setMenuOpenFor(null)
        openDetail(null)
        return
      }
      if (event.key !== 'Tab' || !sheetRef.current) return

      const focusable = sheetRef.current.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      )
      if (focusable.length === 0) return
      const first = focusable[0]!
      const last = focusable[focusable.length - 1]!

      if (!sheetRef.current.contains(document.activeElement)) {
        event.preventDefault()
        first.focus()
      } else if (event.shiftKey && document.activeElement === first) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault()
        first.focus()
      }
    }

    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
      returnFocusTo.current?.focus?.()
    }
  }, [detail, openDetail])

  if (!detail) return null

  const menuOpen = menuOpenFor === detail.id
  const eligibility = detail.assessment?.eligibility ?? 'by_choice'
  // Anything not blocked can be chosen from here. An application folder can
  // too — this is the one place a single item is chosen on its own — but it
  // says what it is, and the review that follows says it again.
  const actionable = eligibility !== 'blocked' && detail.risk !== 'protected'
  const application = eligibility === 'explicit_only'
  const positive = detail.evidence.filter((e) => !e.negative)
  const negative = detail.evidence.filter((e) => e.negative)

  return (
    <>
      <div className={styles.scrim} onClick={() => openDetail(null)} />
      <aside
        ref={sheetRef}
        className={styles.sheet}
        role="dialog"
        aria-modal="true"
        aria-label={detail.display_name}
      >
        <header className={styles.head}>
          <span className={styles.headGlyph}>
            <Glyph category={detail.category} size={30} />
          </span>
          <div className={styles.headText}>
            <h2 className={styles.name}>{detail.display_name}</h2>
            <p className={styles.category}>
              {detail.target_kind === 'directory' ? 'Folder' : 'File'}
              {detail.associated_app && ` · ${detail.associated_app}`}
            </p>
          </div>
          <button
            ref={closeRef}
            className={styles.close}
            onClick={() => openDetail(null)}
            aria-label="Close"
          >
            ×
          </button>
        </header>

        <div className={styles.body}>
          {detail.remark && <p className={styles.remark}>{detail.remark}</p>}

          <div className={styles.verdict}>
            <div className={styles.verdictItem}>
              <span className={styles.verdictLabel}>Confidence</span>
              <span className={styles.verdictValue}>
                {CONFIDENCE_WORD[detail.confidence]}
              </span>
            </div>
            <div className={styles.verdictItem}>
              <span className={styles.verdictLabel}>Risk</span>
              <span className={styles.verdictValue} data-risk={detail.risk}>
                {RISK_WORD[detail.risk]}
              </span>
            </div>
            <p className={styles.verdictMeaning}>{RISK_MEANING[detail.risk]}</p>
          </div>

          {/*
            Safety copy. Plain, serious, and never funny — this is the point
            where a joke would cost someone their files.
          */}
          {!actionable && (
            <p className={styles.notice}>
              {detail.assessment?.blocked ??
                'This is in a location Scuttle will not touch. It is shown here so you know it exists.'}
            </p>
          )}
          {application && (
            <p className={styles.notice}>
              This is part of an installed application, not something left over. Scuttle will
              never suggest moving it or include it in a batch. If you move it anyway, the
              application will probably stop working until you put it back.
            </p>
          )}
          {actionable && !application && (detail.assessment?.cautions.length ?? 0) > 0 && (
            <ul className={styles.cautionList}>
              {detail.assessment.cautions.map((caution) => (
                <li key={caution.kind}>{caution.detail}</li>
              ))}
            </ul>
          )}

          <section className={styles.section}>
            <h3 className={styles.sectionTitle}>Why Scuttle noticed</h3>
            <ul className={styles.evidence}>
              {positive.map((reason, index) => (
                <li
                  key={`${reason.summary}-${index}`}
                  className={styles.reason}
                  data-negative="false"
                >
                  <span className={styles.mark} aria-hidden="true">
                    ✓
                  </span>
                  <span className={styles.reasonText}>{reason.summary}</span>
                </li>
              ))}
            </ul>
          </section>

          {negative.length > 0 && (
            <section className={styles.section}>
              <h3 className={styles.sectionTitle}>Reasons to leave it alone</h3>
              <ul className={styles.evidence}>
                {negative.map((reason, index) => (
                  <li
                    key={`${reason.summary}-${index}`}
                    className={styles.reason}
                    data-negative="true"
                  >
                    <span className={styles.mark} aria-hidden="true">
                      ✗
                    </span>
                    <span className={styles.reasonText}>{reason.summary}</span>
                  </li>
                ))}
              </ul>
            </section>
          )}

          <section className={styles.section}>
            <h3 className={styles.sectionTitle}>Facts</h3>
            <div className={styles.facts}>
              <span className={styles.factKey}>Size</span>
              <span className={styles.factValue}>{bytes(detail.size)}</span>

              <span className={styles.factKey}>Changed</span>
              <span className={styles.factValue}>{whenish(detail.modified_unix)}</span>

              {detail.created_unix !== null && (
                <>
                  <span className={styles.factKey}>Created</span>
                  <span className={styles.factValue}>{whenish(detail.created_unix)}</span>
                </>
              )}

              <span className={styles.factKey}>Verdict</span>
              <span className={styles.factValue}>
                {ACTION_WORD[detail.recommended_action]} —{' '}
                {ACTION_MEANING[detail.recommended_action]}
              </span>

              <span className={styles.factKey}>Where</span>
              <span className={`${styles.factValue} ${styles.path} selectable`} title={detail.path}>
                {shortPath(detail.path)}
              </span>

              <span className={styles.factKey}>Found by</span>
              <span className={styles.factValue}>{detail.detector}</span>
            </div>
          </section>

          {detail.group.length > 1 && <Members candidate={detail} />}
        </div>

        <footer className={styles.actions}>
          <button className={styles.action} onClick={() => void reveal(detail)}>
            Reveal
          </button>

          <div className={styles.ignoreWrap}>
            <button
              className={styles.action}
              onClick={() => setMenuOpenFor(menuOpen ? null : detail.id)}
              aria-expanded={menuOpen}
              aria-haspopup="menu"
            >
              Keep
            </button>
            {menuOpen && (
              <div className={styles.menu} role="menu">
                <button
                  className={styles.menuItem}
                  role="menuitem"
                  onClick={() => void keep(detail)}
                >
                  Keep it, ask again next time
                </button>
                <button
                  className={styles.menuItem}
                  role="menuitem"
                  onClick={() => void ignore(detail, 'path')}
                >
                  Never mention this again
                </button>
                {detail.associated_app && (
                  <button
                    className={styles.menuItem}
                    role="menuitem"
                    onClick={() => void ignore(detail, 'app')}
                  >
                    Never mention {detail.associated_app}
                  </button>
                )}
                <button
                  className={styles.menuItem}
                  role="menuitem"
                  onClick={() => void ignore(detail, 'category')}
                >
                  Skip this whole pile from now on
                </button>
              </div>
            )}
          </div>

          <button
            className={`${styles.action} ${application ? '' : styles.primary}`}
            disabled={!actionable}
            onClick={() => void quarantine(detail)}
            title={
              !actionable
                ? 'Scuttle will not act on this one.'
                : application
                  ? 'Review moving this application folder. Nothing moves until you confirm.'
                  : 'Review what would move. Nothing moves until you confirm.'
            }
          >
            {application ? 'Move this application folder…' : 'Put in the drawer…'}
          </button>
        </footer>
      </aside>
    </>
  )
}

/**
 * The members of a group finding.
 *
 * Scuttle marks the copy it would keep and lets you act on any of them. It
 * does not pre-select anything, because which duplicate matters is a question
 * only the person who made them can answer.
 */
function Members({ candidate }: { candidate: Candidate }) {
  const { quarantine, quarantineGroup } = useStore()
  // Copies are a person's own files: theirs to choose among, with any caution
  // shown in the review. Only application files are held back from this.
  const eligibility = candidate.assessment?.eligibility ?? 'by_choice'
  const actionable =
    candidate.risk !== 'protected' && eligibility !== 'blocked' && eligibility !== 'explicit_only'

  return (
    <section className={styles.section}>
      <h3 className={styles.sectionTitle}>
        All {candidate.group.length} of them
      </h3>

      {/*
        The bulk choices. Scuttle decides *which* copy survives from the dates
        it recorded; the button only names the intent. Nothing is pre-selected,
        because which duplicate matters is a question only the person who made
        them can answer.
      */}
      {actionable && (
        <div className={styles.groupActions}>
          <button
            className={styles.groupAction}
            onClick={() => void quarantineGroup(candidate, 'newest')}
          >
            Keep the newest
          </button>
          <button
            className={styles.groupAction}
            onClick={() => void quarantineGroup(candidate, 'oldest')}
          >
            Keep the oldest
          </button>
          <span className={styles.groupHint}>
            The rest go to the drawer, where you can still get them back.
          </span>
        </div>
      )}
      <ul className={styles.members}>
        {candidate.group.map((member, index) => (
          <li key={member.path} className={styles.member}>
            <span className={styles.memberPath} title={member.path}>
              {shortPath(member.path)}
            </span>
            {member.suggested_keep ? (
              <span className={styles.memberKeep}>newest</span>
            ) : (
              <span />
            )}
            {actionable ? (
              <button
                className={styles.memberAction}
                onClick={() => void quarantine(candidate, index)}
              >
                Drawer
              </button>
            ) : (
              <span />
            )}
          </li>
        ))}
      </ul>
    </section>
  )
}
