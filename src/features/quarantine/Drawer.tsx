import { useEffect, useState } from 'react'

import { Unavailable, Waiting } from '@/components/Unavailable'

import { useStore } from '@/app/store'
import { api } from '@/lib/ipc'
import { bytes, daysUntil, shortPath, whenish } from '@/lib/format'
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
  const { drawer, refreshDrawer, restore, removePermanently, go } = useStore()
  const [confirming, setConfirming] = useState<string | null>(null)
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
        <h2 className={styles.title}>The drawer</h2>
        <p className={styles.line}>
          {drawer.items.length === 0
            ? 'Nothing in here. Things you quarantine wait here before they go anywhere.'
            : `${drawer.items.length} ${drawer.items.length === 1 ? 'thing' : 'things'}, ${bytes(drawer.held_bytes)}. Still yours — nothing has been deleted. After ${retention} days Scuttle clears them out.`}
        </p>
      </header>

      <div className={styles.drawer}>
        {drawer.items.length === 0 ? (
          <div className={styles.empty}>
            <Scuttle mood="idle" size={70} />
            <p className={styles.emptyLine}>Empty, and that is fine.</p>
          </div>
        ) : (
          <ul className={styles.shelf}>
            {drawer.items.map((item, index) => {
              const left = daysUntil(item.expires_unix)
              return (
                <li
                  key={item.id}
                  className={styles.thing}
                  style={{ animationDelay: `${index * 55}ms` }}
                >
                  <span className={styles.thingGlyph}>
                    <Glyph category={item.category} size={24} />
                  </span>
                  <span className={styles.thingName}>{item.display_name}</span>
                  <span className={styles.thingSize}>{bytes(item.size)}</span>
                  <span className={styles.thingActions}>
                    <button className={styles.restore} onClick={() => void restore(item.id)}>
                      Put it back
                    </button>
                    <button
                      className={styles.forever}
                      onClick={() => setConfirming(item.id)}
                    >
                      Remove
                    </button>
                  </span>

                  <span className={styles.thingFrom} title={item.original_path}>
                    from {shortPath(item.original_path)}
                  </span>
                  <span className={styles.expiry} data-soon={left <= 2}>
                    Quarantined {whenish(item.quarantined_unix)} ·{' '}
                    {left === 0
                      ? 'goes today'
                      : `${left} ${left === 1 ? 'day' : 'days'} left`}
                  </span>

                  {confirming === item.id && (
                    <div className={styles.confirm} role="alertdialog">
                      {/*
                        The only irreversible action in the application, so the
                        copy is plain and states exactly what happens. No jokes
                        anywhere near this.
                      */}
                      <span className={styles.confirmText}>
                        Remove {item.display_name} permanently? This deletes it from your
                        computer. It cannot be undone, and it does not go to the Bin.
                      </span>
                      <button
                        className={styles.confirmYes}
                        onClick={() => {
                          setConfirming(null)
                          void removePermanently(item.id)
                        }}
                      >
                        Remove permanently
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
