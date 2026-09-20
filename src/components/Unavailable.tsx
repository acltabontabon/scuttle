import { Scuttle } from '@/visuals/Scuttle'

import styles from './Unavailable.module.css'

/**
 * What a screen shows when its data did not arrive.
 *
 * Every view used to render `null` in this case, which is a blank window and
 * no way forward — the toast that explained it had usually gone by the time
 * anyone looked. A dead end with a retry is worse than working and better
 * than nothing.
 */
export function Unavailable({
  what,
  onRetry,
}: {
  what: string
  onRetry: () => void
}) {
  return (
    <div className={styles.block}>
      <Scuttle mood="shrug" size={78} />
      <p className={styles.line}>Scuttle could not read {what}.</p>
      <p className={styles.detail}>
        Nothing is wrong with your files — this is Scuttle failing to look, not
        something it found.
      </p>
      <button className={styles.retry} onClick={onRetry}>
        Try again
      </button>
    </div>
  )
}

/** The same shape, while a screen is still waiting. */
export function Waiting({ what }: { what: string }) {
  return (
    <div className={styles.block}>
      <Scuttle mood="rummaging" size={70} />
      <p className={styles.detail}>{what}</p>
    </div>
  )
}
