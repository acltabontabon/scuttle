import { useEffect, useState } from 'react'

import { Unavailable, Waiting } from '@/components/Unavailable'

import { useStore } from '@/app/store'
import { api } from '@/lib/ipc'
import { bytes } from '@/lib/format'
import { DrawerItem } from './DrawerItem'
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
  const { drawer, refreshDrawer, restore, removePermanently, emptyDrawer, go, background } =
    useStore()
  const [emptying, setEmptying] = useState(false)
  const [purging, setPurging] = useState(false)
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
            {/*
              "Put anything back whenever you like" was not true — restoring
              only works while the item is still here. And the policy is
              stated as implemented: the sweep runs in `commands::init`, which
              is app launch, so nothing is deleted at a particular hour.
            */}
            <p className={styles.line}>
              These files still take up disk space. Put them back before they expire.
            </p>
            {/*
              Retention has not changed: an item still expires after exactly
              the days it was given. What changed is when the deletion
              happens. An application that stays in the menu bar starts far
              less often, so it sweeps on its own schedule instead — otherwise
              "after {retention} days" would quietly become "eventually".
            */}
            <p className={styles.policy}>
              {background?.mode
                ? `Items expire after ${retention} days, and are deleted shortly after that while Scuttle is running.`
                : `Items expire after ${retention} days and are deleted the next time Scuttle starts.`}
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
            <Scuttle mood="asleep" size={76} />
            {/* The route back already sits in the footer a few lines down;
                repeating it here was two links to the same place. */}
            <p className={styles.emptyLine}>Empty, and that is fine.</p>
          </div>
        ) : (
          <ul className={styles.shelfGrid}>
            {drawer.items.map((item, index) => (
              <DrawerItem
                key={item.id}
                item={item}
                index={index}
                onRestore={restore}
                onDelete={removePermanently}
              />
            ))}
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
            disabled={purging}
            onClick={() => {
              if (purging) return
              setPurging(true)
              void emptyDrawer().finally(() => {
                setPurging(false)
                setEmptying(false)
              })
            }}
          >
            {purging ? 'Deleting…' : `Delete all ${drawer.items.length} permanently`}
          </button>
          <button
            className={styles.confirmNo}
            disabled={purging}
            onClick={() => setEmptying(false)}
          >
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
          ← Back to findings
        </button>
        {drawer.items.length > 0 && (
          <button
            className={styles.forever}
            style={{ color: 'var(--ink-soft)' }}
            onClick={() => {
              void api.revealQuarantined(drawer.items[0]!.id)
            }}
          >
            Open drawer folder
          </button>
        )}
      </div>
    </div>
  )
}
