import { useRef, useState } from 'react'

import { bytes, daysUntil, shortPath } from '@/lib/format'
import type { QuarantineRecord } from '@/lib/types'
import { Glyph } from '@/visuals/Glyph'

import styles from './Drawer.module.css'

/**
 * One thing filed in the drawer.
 *
 * Split out from the drawer itself because each slip now owns real state: an
 * operation in flight, a confirmation, a failure that belongs beside the item
 * it happened to rather than in a toast at the edge of the screen.
 */
interface DrawerItemProps {
  item: QuarantineRecord
  /** Staggers the settling animation. */
  index: number
  onRestore: (id: string) => Promise<boolean>
  onDelete: (id: string) => Promise<boolean>
}

type Busy = 'restoring' | 'deleting' | null

export function DrawerItem({ item, index, onRestore, onDelete }: DrawerItemProps) {
  const [busy, setBusy] = useState<Busy>(null)
  const [confirming, setConfirming] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const [showPath, setShowPath] = useState(false)
  const card = useRef<HTMLLIElement>(null)

  const left = daysUntil(item.expires_unix)

  /**
   * Nothing is claimed before it has happened, and nothing can be asked for
   * twice: the control stays disabled until the core has answered. If it
   * answers badly the item is still here, so the reason belongs here too.
   */
  const run = async (what: Exclude<Busy, null>, work: () => Promise<boolean>) => {
    if (busy) return
    setBusy(what)
    setFailure(null)
    const ok = await work()
    if (!ok) {
      setFailure(
        what === 'restoring' ? 'Could not put that back.' : 'Could not delete that.',
      )
      // Focus would otherwise be lost on a control that is about to re-enable
      // under a message nobody was told about.
      card.current?.focus()
    }
    setBusy(null)
  }

  return (
    <li
      ref={card}
      className={styles.card}
      data-busy={busy ?? undefined}
      tabIndex={-1}
      style={{ animationDelay: `${Math.min(index, 8) * 45}ms` }}
    >
      <span className={styles.cardGlyph}>
        <Glyph category={item.category} size={22} />
      </span>
      <span className={styles.cardName} title={item.display_name}>
        {item.display_name}
      </span>
      <span className={styles.cardSize}>
        {bytes(item.size)}
        {item.mode === 'contents' && (item.item_count ?? 0) > 0 && (
          <span className={styles.cardCount}>
            {' · '}
            {(item.item_count ?? 0).toLocaleString('en-US')}{' '}
            {item.item_count === 1 ? 'file' : 'files'}
          </span>
        )}
      </span>

      <span className={styles.cardMeta}>
        {/*
          A real button, so the full original location is reachable by
          keyboard. It is labelled "from" because the path it shows is where
          this came from, not where it is now — it is in the drawer now, and
          those are easy to confuse.
        */}
        <button
          className={styles.cardFrom}
          onClick={() => setShowPath((on) => !on)}
          aria-expanded={showPath}
          title={item.original_path}
        >
          from {showPath ? item.original_path : shortPath(item.original_path)}
        </button>
        {item.keep === false ? (
          <span className={styles.cardExpiry} data-state={expiryState(left)}>
            {expiryLabel(left)}
          </span>
        ) : (
          <span className={styles.cardExpiry} data-state="fine">
            kept until you remove it
          </span>
        )}
      </span>

      <span className={styles.cardActions}>
        <button
          className={styles.restore}
          disabled={busy !== null}
          onClick={() => void run('restoring', () => onRestore(item.id))}
        >
          {busy === 'restoring' ? 'Putting it back…' : 'Put it back'}
        </button>
        <button
          className={styles.forever}
          disabled={busy !== null}
          onClick={() => setConfirming(true)}
          aria-label={`Delete ${item.display_name} permanently`}
        >
          {busy === 'deleting' ? 'Deleting…' : 'Delete permanently'}
        </button>
      </span>

      {item.attention && (
        <p className={styles.cardAttention}>
          A move into the drawer was interrupted, and Scuttle kept everything it was not sure
          about. Nothing was deleted, and the originals were left where they were.
        </p>
      )}

      {failure && (
        <p className={styles.cardFailure} role="status">
          {failure} It is still here.
        </p>
      )}

      {confirming && (
        <div className={styles.confirm} role="alertdialog">
          <span className={styles.confirmText}>
            Delete {item.display_name} permanently? It does not go to the Trash and
            cannot be recovered.
          </span>
          <button
            className={styles.confirmYes}
            onClick={() => {
              setConfirming(false)
              void run('deleting', () => onDelete(item.id))
            }}
          >
            Delete permanently
          </button>
          <button className={styles.confirmNo} onClick={() => setConfirming(false)}>
            Cancel
          </button>
        </div>
      )}
    </li>
  )
}

/**
 * How much time is left, in words.
 *
 * Deliberately vague about the moment of deletion, because the sweep runs at
 * startup rather than on a timer: something can sit past its date for as long
 * as the application stays closed. "Goes at next start" is what actually
 * happens; "deleted today" would not be true.
 */
export function expiryLabel(daysLeft: number): string {
  if (daysLeft === 0) return 'expired · goes at next start'
  if (daysLeft === 1) return '1 day left'
  return `${daysLeft} days left`
}

/** Only the last couple of days get any emphasis; everything else is calm. */
export function expiryState(daysLeft: number): 'expired' | 'soon' | 'fine' {
  if (daysLeft === 0) return 'expired'
  if (daysLeft <= 2) return 'soon'
  return 'fine'
}
