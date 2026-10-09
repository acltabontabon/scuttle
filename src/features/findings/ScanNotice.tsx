import { useState } from 'react'
import { useStore } from '@/app/store'
import { platform } from '@/lib/platform'
import type { HiccupSummary } from '@/lib/types'
import styles from './Findings.module.css'

/** Counts are all the scan reports; offer guidance without inventing paths. */
export function ScanNotice({ hiccups }: { hiccups: HiccupSummary }) {
  const { go, rummage, scan, moving } = useStore()
  const [open, setOpen] = useState(false)
  const total = hiccups.permission_denied + hiccups.unreadable + hiccups.vanished
  if (total === 0) return null

  return (
    <div className={styles.notice}>
      <div className={styles.noticeSummary}>
        <p><strong>{total} {total === 1 ? 'place' : 'places'} skipped.</strong> Some areas weren&rsquo;t checked.</p>
        <button className={styles.noticeToggle} aria-expanded={open} aria-controls="scan-help" onClick={() => setOpen((value) => !value)}>
          {open ? 'Close help' : 'What can I do?'}
        </button>
      </div>
      {open && (
        <div className={styles.noticeHelp} id="scan-help">
          <ul className={styles.noticeReasons}>
            {hiccups.permission_denied > 0 && (
              <li>
                <h3>{hiccups.permission_denied} {hiccups.permission_denied === 1 ? 'place needs' : 'places need'} folder access</h3>
                <p>{platform() === 'macos'
                  ? 'Your system blocked access. Check System Settings → Privacy & Security → Files & Folders for Scuttle. Allow only folders you want scanned, then scan again. Some system folders remain restricted.'
                  : 'Your system blocked access. Check that you can open the folders included in the scan. Some folders belong to the system or another account; you can leave those out in scan settings.'}</p>
              </li>
            )}
            {hiccups.vanished > 0 && (
              <li>
                <h3>{hiccups.vanished} {hiccups.vanished === 1 ? 'place disappeared' : 'places disappeared'} during the scan</h3>
                <p>Files or folders were moved or removed while Scuttle was looking. This can happen during downloads or app updates. You can leave these alone, or scan again once things settle.</p>
              </li>
            )}
            {hiccups.unreadable > 0 && (
              <li>
                <h3>{hiccups.unreadable} {hiccups.unreadable === 1 ? 'place could' : 'places could'} not be read</h3>
                <p>Check that the scan folders are still available and any external drives are connected, then try again. If a folder stays unreadable, you can leave it out in scan settings.</p>
              </li>
            )}
          </ul>
          <p className={styles.noticeLimit}>Scuttle reports counts, so it can&rsquo;t list the affected paths here. You can still review what it found.</p>
          <div className={styles.noticeActions}>
            <button onClick={() => go({ name: 'settings' })}>Review scan folders →</button>
            <button disabled={scan.status === 'running' || moving} onClick={() => void rummage()}>Scan again</button>
          </div>
        </div>
      )}
    </div>
  )
}
