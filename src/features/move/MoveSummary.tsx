import { useEffect, useRef, useState } from 'react'

import { useStore } from '@/app/store'
import type { FindingResult, MoveSnapshot } from '@/lib/types'
import { adviceFor, describeReport, diagnostics, issueLine, technicalLine } from './phrasing'
import { isTerminal, trackFor } from './progress'

import styles from './Move.module.css'

/**
 * The details of a move: what happened to each finding, why, and what could be
 * done about it. A panel, not a modal — it can be left open while you look
 * around, and Escape or a click elsewhere puts it away.
 *
 * It exists because a toast is the wrong place for anything that needs
 * acting on. Everything a toast could not fit is here, and stays until it is
 * dismissed or the next move starts.
 */
export function MoveSummary() {
  const { move, moving, cancelMove, dismissMove, retryMove, reviewAgain, setMoveDetailsOpen, go } =
    useStore()
  const panel = useRef<HTMLDivElement>(null)
  const [copied, setCopied] = useState(false)
  const snapshot = move.snapshot

  // Focus lands inside, Escape leaves, and a click outside closes it.
  useEffect(() => {
    panel.current?.focus({ preventScroll: true })
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setMoveDetailsOpen(false)
    }
    const onPointer = (event: MouseEvent) => {
      const target = event.target as Node
      if (panel.current?.contains(target)) return
      // The chip toggles the panel itself; do not fight it.
      if ((target as Element).closest?.('[aria-haspopup="dialog"]')) return
      setMoveDetailsOpen(false)
    }
    document.addEventListener('keydown', onKey)
    document.addEventListener('mousedown', onPointer)
    return () => {
      document.removeEventListener('keydown', onKey)
      document.removeEventListener('mousedown', onPointer)
    }
  }, [setMoveDetailsOpen])

  if (!snapshot) return null

  return (
    <div
      ref={panel}
      className={styles.panel}
      role="dialog"
      aria-label="Moving into the drawer, details"
      tabIndex={-1}
    >
      {moving || !isTerminal(snapshot.phase) ? (
        <Running snapshot={snapshot} onCancel={() => void cancelMove()} />
      ) : snapshot.report ? (
        <Finished
          snapshot={snapshot}
          copied={copied}
          onCopy={() => {
            void navigator.clipboard
              ?.writeText(diagnostics(snapshot.report!))
              .then(() => setCopied(true))
              .catch(() => undefined)
          }}
          onRetry={() => void retryMove()}
          onReview={(ids) => {
            setMoveDetailsOpen(false)
            void reviewAgain(ids)
          }}
          onDrawer={() => {
            setMoveDetailsOpen(false)
            go({ name: 'drawer' })
          }}
          onDismiss={() => void dismissMove()}
        />
      ) : null}
    </div>
  )
}

function Running({ snapshot, onCancel }: { snapshot: MoveSnapshot; onCancel: () => void }) {
  const track = trackFor(snapshot)
  return (
    <>
      <p className={styles.panelHead}>{track.label}</p>
      {snapshot.label && <p className={styles.panelSub}>{snapshot.label}</p>}
      <dl className={styles.counts}>
        <Count label="Moved" value={snapshot.moved} />
        <Count label="Skipped" value={snapshot.skipped} />
        <Count label="Failed" value={snapshot.failed} />
      </dl>
      <p className={styles.footnote}>
        Nothing is deleted. Everything that moves is kept in the drawer until you empty it.
      </p>
      {snapshot.phase !== 'finalizing' && (
        <div className={styles.panelActions}>
          <button className={styles.textButton} onClick={onCancel} disabled={snapshot.cancelling}>
            {snapshot.cancelling ? 'Stopping…' : 'Cancel'}
          </button>
        </div>
      )}
    </>
  )
}

function Count({ label, value }: { label: string; value: number }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value.toLocaleString('en-US')}</dd>
    </div>
  )
}

function Finished({
  snapshot,
  copied,
  onCopy,
  onRetry,
  onReview,
  onDrawer,
  onDismiss,
}: {
  snapshot: MoveSnapshot
  copied: boolean
  onCopy: () => void
  onRetry: () => void
  onReview: (ids: string[]) => void
  onDrawer: () => void
  onDismiss: () => void
}) {
  const report = snapshot.report!
  const outcome = describeReport(report)
  const worth = report.findings.filter((f) => f.issues.length > 0 || f.refusal || f.status !== 'moved')

  return (
    <>
      <p className={styles.panelHead}>{outcome.headline}</p>

      {worth.length > 0 && (
        <ul className={styles.findings}>
          {worth.map((finding) => (
            <FindingRow key={finding.finding_id + finding.display_name} finding={finding} />
          ))}
        </ul>
      )}

      {report.moved_files > 0 && (
        <p className={styles.footnote}>
          Moved is not freed: everything that moved is in the drawer, and no space comes back until
          you empty it.
        </p>
      )}

      <div className={styles.panelActions}>
        {outcome.retryIds.length > 0 && (
          <button className={styles.textButton} onClick={onRetry}>
            Try the rest again
          </button>
        )}
        {outcome.reviewIds.length > 0 && (
          <button className={styles.textButton} onClick={() => onReview(outcome.reviewIds)}>
            Review again
          </button>
        )}
        {report.moved_files > 0 && (
          <button className={styles.textButton} onClick={onDrawer}>
            Open the drawer
          </button>
        )}
        <button className={styles.textButton} onClick={onCopy}>
          {copied ? 'Copied' : 'Copy details'}
        </button>
        <button className={styles.textButton} data-quiet="" onClick={onDismiss}>
          Dismiss
        </button>
      </div>
    </>
  )
}

function FindingRow({ finding }: { finding: FindingResult }) {
  return (
    <li className={styles.finding}>
      <p className={styles.findingName}>
        <span>{finding.display_name}</span>
        <span className={styles.findingCounts}>
          {finding.moved.toLocaleString('en-US')} moved
          {finding.skipped > 0 && ` · ${finding.skipped.toLocaleString('en-US')} skipped`}
          {finding.failed > 0 && ` · ${finding.failed.toLocaleString('en-US')} failed`}
        </span>
      </p>
      {finding.refusal && <p className={styles.issue}>{finding.refusal.message}</p>}
      {finding.issues.map((group) => (
        <div className={styles.issueBlock} key={`${group.kind}-${group.phase}-${group.os_code ?? 'x'}`}>
          <p className={styles.issue}>{issueLine(group, finding)}</p>
          <p className={styles.advice}>{adviceFor(group.next_step)}</p>
          <p className={styles.technical}>
            {technicalLine(group)}
            {group.samples.length > 0 && ` · e.g. ${group.samples.join(', ')}`}
          </p>
        </div>
      ))}
      {finding.needs_refresh && finding.issues.length === 0 && !finding.refusal && (
        <p className={styles.issue}>This changed since the scan. Review it again.</p>
      )}
    </li>
  )
}
