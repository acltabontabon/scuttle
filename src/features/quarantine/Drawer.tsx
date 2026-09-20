import { useEffect, useState } from 'react'

import { Unavailable, Waiting } from '@/components/Unavailable'

import { useStore } from '@/app/store'
import { api } from '@/lib/ipc'
import { bytes, daysUntil, shortPath } from '@/lib/format'
import { Glyph } from '@/visuals/Glyph'
import { Scuttle } from '@/visuals/Scuttle'

import styles from './Drawer.module.css'

/**
 * The drawer.
 *
 * Quarantine is the whole safety story made visible: things move *somewhere*
 * rather than disappearing, and the somewhere looks like a shallow open box
 * with the objects still sitting in it. Nothing here is a database table.
 */
export function Drawer() {
  const { drawer, refreshDrawer, restore, removePermanently, emptyDrawer, go } = useStore()
  const [confirming, setConfirming] = useState<string | null>(null)
  const [emptying, setEmptying] = useState(false)
  const [failed, setFailed] = useState(false)

  const load = () => {
    setFailed(false)
    void refreshDrawer().then((ok) => setFailed(!ok))
  }

  useEffect(() => {
    void refreshDrawer().then((ok) => setFailed(!ok))
  }, [refreshDrawer])

  if (!drawer) {
    return failed ? (
      <Unavailable what="the drawer" onRetry={load} />
    ) : (
      <Waiting what="Opening the drawer…" />
    )
  }

  const retention = drawer.retention_days

  return (
    <div className={styles.room}>
      <header className={styles.head}>
        <div className={styles.headText}>
        <h2 className={styles.title}>The drawer</h2>
        {drawer.items.length === 0 ? (
          <p className={styles.line}>
            Nothing in here. Things you move out of a pile wait here before they go
            anywhere.
          </p>
        ) : (
          <>
            <p className={styles.tally}>
              {drawer.items.length} {drawer.items.length === 1 ? 'thing' : 'things'} ·{' '}
              {bytes(drawer.held_bytes)} held
            </p>
            <p className={styles.line}>
              Still on your disk, and still taking up that room. Put anything back
              whenever you like.
            </p>
            {/*
              The policy as implemented, not as wished for: the sweep runs in
              `commands::init`, which is app launch. Saying "after 14 days"
              flat would be a promise Scuttle cannot keep while it is closed.
            */}
            <p className={styles.policy}>
              Anything still here {retention} days after it went in is deleted the next
              time Scuttle starts.
            </p>
          </>
        )}
        </div>

        {/*
          The one action that frees anything, standing opposite the tally it
          acts on. It used to sit below the tray, which on a full drawer meant
          scrolling past everything to reach it.
        */}
        {drawer.items.length > 0 && !emptying && (
          <div className={styles.headAction}>
            <button className={styles.emptyAction} onClick={() => setEmptying(true)}>
              Empty the drawer
            </button>
            <span className={styles.emptyHint}>
              frees {bytes(drawer.held_bytes)} · deletes all {drawer.items.length}
            </span>
          </div>
        )}
      </header>

      <div className={styles.drawer}>
        {drawer.items.length === 0 ? (
          <div className={styles.empty}>
            <Scuttle mood="idle" size={70} />
            <p className={styles.emptyLine}>Empty, and that is fine.</p>
          </div>
        ) : (
          <ul className={styles.shelfGrid}>
            {drawer.items.map((item, index) => {
              const left = daysUntil(item.expires_unix)
              return (
                <li
                  key={item.id}
                  className={styles.card}
                  style={{ animationDelay: `${Math.min(index, 8) * 45}ms` }}
                >
                  <span className={styles.cardGlyph}>
                    <Glyph category={item.category} size={22} />
                  </span>
                  <span className={styles.cardName} title={item.display_name}>
                    {item.display_name}
                  </span>
                  <span className={styles.cardSize}>{bytes(item.size)}</span>

                  <span className={styles.cardMeta}>
                    {/*
                      Reversed direction so a long path keeps its tail — the
                      folder it sits in is the part worth reading, and the
                      full path is still on the title.
                    */}
                    <span className={styles.cardFrom} title={item.original_path}>
                      {shortPath(item.original_path)}
                    </span>
                    <span className={styles.cardExpiry} data-soon={left <= 2}>
                      {left === 0 ? 'goes today' : `${left} ${left === 1 ? 'day' : 'days'} left`}
                    </span>
                  </span>

                  <span className={styles.cardActions}>
                    <button
                      className={styles.cardAction}
                      onClick={() => void restore(item.id)}
                    >
                      Put it back
                    </button>
                    <button
                      className={`${styles.cardAction} ${styles.cardDanger}`}
                      onClick={() => setConfirming(item.id)}
                      aria-label={`Delete ${item.display_name} permanently`}
                    >
                      Delete permanently
                    </button>
                  </span>

                  {confirming === item.id && (
                    <div className={styles.confirm} role="alertdialog">
                      <span className={styles.confirmText}>
                        Delete {item.display_name} permanently? It does not go to the
                        Trash and cannot be recovered.
                      </span>
                      <button
                        className={styles.confirmYes}
                        onClick={() => {
                          setConfirming(null)
                          void removePermanently(item.id)
                        }}
                      >
                        Delete permanently
                      </button>
                      <button className={styles.confirmNo} onClick={() => setConfirming(null)}>
                        Cancel
                      </button>
                    </div>
                  )}
                </li>
              )
            })}
          </ul>
        )}
      </div>

      {/*
        The only irreversible action in the application, so the copy is plain
        and states exactly what happens. No jokes anywhere near this.
      */}
      {emptying && (
        <div className={styles.confirmBar} role="alertdialog">
          <span className={styles.confirmText}>
            Delete all {drawer.items.length}{' '}
            {drawer.items.length === 1 ? 'thing' : 'things'} permanently? This frees{' '}
            {bytes(drawer.held_bytes)}. Nothing goes to the Trash and nothing can be
            recovered.
          </span>
          <button
            className={styles.confirmYes}
            onClick={() => {
              setEmptying(false)
              void emptyDrawer()
            }}
          >
            Delete permanently
          </button>
          <button className={styles.confirmNo} onClick={() => setEmptying(false)}>
            Cancel
          </button>
        </div>
      )}

      <div className={styles.foot}>
        <button
          className={styles.forever}
          style={{ color: 'var(--ink-soft)' }}
          onClick={() => go({ name: 'findings' })}
        >
          ← Back to the floor
        </button>
        {drawer.items.length > 0 && (
          <button
            className={styles.forever}
            style={{ color: 'var(--ink-soft)' }}
            onClick={() => {
              void api.revealQuarantined(drawer.items[0]!.id)
            }}
          >
            Show me where the drawer lives
          </button>
        )}
      </div>
    </div>
  )
}
