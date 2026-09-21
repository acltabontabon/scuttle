import { bytes, whenish } from '@/lib/format'
import type { UpdateSnapshot } from '@/lib/types'

/**
 * What updating says.
 *
 * Every word about updating is here, for the same reason the tray's are in
 * `background/phrasing.ts`: the interface must never promise something the
 * application cannot do, and a promise is easiest to audit in one file. Two
 * rules run through it. It says what did *not* happen as readily as what did —
 * nothing downloaded, nothing installed, Scuttle still running — and it never
 * describes installing as anything other than what it is: Scuttle closing and
 * opening again.
 */

export const AUTO_CHECK_LABEL = 'Automatically check for updates'

export const AUTO_CHECK_HINT =
  'Shortly after starting, and about once a day after that, Scuttle asks GitHub whether a newer version exists. It only looks: nothing is downloaded until you say so.'

/** Said wherever installing is offered, because it is the one surprising part. */
export const RESTART_NOTICE =
  'Installing closes Scuttle and opens the new version. Your settings, Drawer and history stay where they are.'

/** Said while it is happening. */
export const RESTARTING_NOTICE = 'Scuttle is closing, and will open again on its own.'

export const NOT_YET_DOWNLOADED = 'Nothing has been downloaded yet.'

export const KEEP_WORKING = 'You can keep using Scuttle while this downloads.'

export const CHECK = 'Check for updates'
export const CHECK_AGAIN = 'Check again'
export const TRY_AGAIN = 'Try again'
export const DOWNLOAD = 'Download'
export const INSTALL = 'Update and restart'
export const LATER = 'Later'

/** "Scuttle 0.1.0-alpha.2", with the channel said aloud when it is not stable. */
export function installedLine(current: string, channel: UpdateSnapshot['channel']): string {
  return channel === 'alpha' ? `Scuttle ${current} · alpha` : `Scuttle ${current}`
}

/** "4.2 MB of 18 MB", or, when the server did not say how big, "4.2 MB so far". */
export function sizeLine(received: number, total: number | null): string {
  return total !== null && total > 0
    ? `${bytes(Math.min(received, total))} of ${bytes(total)}`
    : `${bytes(received)} so far`
}

/** A whole percentage, or null when there is nothing honest to say. */
export function percent(received: number, total: number | null): number | null {
  if (total === null || total <= 0) return null
  return Math.min(100, Math.floor((received / total) * 100))
}

/** "just now", "12 minutes ago", "3 hours ago", then the usual day words. */
export function checkedAgo(checkedUnix: number, now = Date.now()): string {
  const seconds = Math.max(0, now / 1000 - checkedUnix)
  if (seconds < 90) return 'just now'
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes} minutes ago`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return hours === 1 ? 'an hour ago' : `${hours} hours ago`
  return whenish(checkedUnix, now)
}

/** The release date, kept short. */
export function releasedOn(unix: number | null): string | null {
  if (unix === null) return null
  return new Date(unix * 1000).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}
