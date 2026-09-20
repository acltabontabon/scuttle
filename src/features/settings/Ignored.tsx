import { useState } from 'react'

import { api } from '@/lib/ipc'
import type { IgnoredEntry } from '@/lib/types'

import styles from './Settings.module.css'

/**
 * Things Scuttle has been told not to mention.
 *
 * The old version offered "Clear the list" over a list nobody could see: no
 * count, no contents, and no way to undo one decision without undoing all of
 * them. It stayed available when there was nothing to clear, too.
 *
 * None of this touches a file. An ignore is a note saying "do not bring this
 * up again", so removing one only makes the thing eligible to appear in the
 * next rummage.
 */
export function Ignored({
  entries,
  onChanged,
  say,
}: {
  entries: IgnoredEntry[] | null
  onChanged: () => void
  say: (text: string, options?: { tone?: 'plain' | 'warn' }) => void
}) {
  const [open, setOpen] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)
  const count = entries?.length ?? 0

  const stop = async (entry: IgnoredEntry) => {
    if (busy) return
    setBusy(entry.value)
    try {
      await api.stopIgnoring(entry.kind, entry.value)
      onChanged()
    } catch {
      say('That one would not budge.', { tone: 'warn' })
    } finally {
      setBusy(null)
    }
  }

  return (
    <section className={styles.group}>
      <h3 className={styles.groupTitle}>Ignored items</h3>

      {count === 0 ? (
        <p className={styles.rowHint}>
          {entries === null
            ? 'Looking…'
            : 'Nothing ignored. Anything you tell Scuttle to forget will be listed here.'}
        </p>
      ) : (
        <>
          <div className={styles.row}>
            <div className={styles.rowText}>
              <div className={styles.rowLabel}>
                {count} {count === 1 ? 'thing' : 'things'} Scuttle will not mention
              </div>
              <p className={styles.rowHint}>
                They may turn up again in the next rummage once they stop being
                ignored. No files are affected either way.
              </p>
            </div>
            <button
              className={styles.link}
              onClick={() => setOpen((on) => !on)}
              aria-expanded={open}
            >
              {open ? 'Hide' : 'Show'}
            </button>
          </div>

          {open && (
            <>
              <ul className={styles.ignoreList}>
                {entries!.map((entry) => (
                  <li key={`${entry.kind}:${entry.value}`} className={styles.ignoreItem}>
                    <span className={styles.ignoreLabel} title={entry.label}>
                      {entry.label}
                    </span>
                    <span className={styles.ignoreKind}>{entry.kind}</span>
                    <button
                      className={styles.link}
                      disabled={busy !== null}
                      onClick={() => void stop(entry)}
                    >
                      {busy === entry.value ? 'Stopping…' : 'Stop ignoring'}
                    </button>
                  </li>
                ))}
              </ul>

              <div className={styles.row}>
                <div className={styles.rowText} />
                {/* Named for what it does, and gone entirely when there is
                    nothing for it to do. */}
                <button
                  className={`${styles.link} ${styles.danger}`}
                  disabled={busy !== null}
                  onClick={() => {
                    void api.clearIgnores().then(() => {
                      onChanged()
                      say('Scuttle has a clean slate again.')
                    })
                  }}
                >
                  Stop ignoring all {count}
                </button>
              </div>
            </>
          )}
        </>
      )}
    </section>
  )
}
