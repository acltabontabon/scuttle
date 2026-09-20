/**
 * What Scuttle says about staying in the menu bar.
 *
 * The rule this module exists to keep: never describe something the
 * application cannot actually do. Background mode has three places where the
 * honest answer is "the system said no" — a tray that would not build, a login
 * item the platform refused, notifications that were denied — and each of them
 * gets a plain sentence rather than a toggle left looking switched on.
 */

import type { BackgroundStatus } from '@/lib/types'

/** macOS calls it the menu bar; Windows calls it the system tray. */
export function trayWord(platform: string): 'menu bar' | 'system tray' {
  return platform === 'windows' ? 'system tray' : 'menu bar'
}

export function keepInTrayLabel(platform: string): string {
  return platform === 'windows'
    ? 'Keep Scuttle in the system tray'
    : 'Keep Scuttle in the menu bar'
}

export function keepInTrayHint(platform: string): string {
  const quit = platform === 'windows' ? 'Alt+F4' : '⌘Q'
  return (
    `Closing the window leaves Scuttle running behind its ${trayWord(platform)} icon ` +
    `instead of quitting. Quit Scuttle, from the icon or with ${quit}, still quits ` +
    'properly, and logging out or shutting down is never held up. Off by default.'
  )
}

export const CHECKS_HINT =
  'At most one look a day, and only when the machine seems able to spare it — ' +
  'on mains power, not saving battery, and not busy with something else. It reads ' +
  'names, sizes and dates and never opens a file, so duplicates and near-identical ' +
  'screenshots still need a rummage you start yourself. Nothing is moved or removed.'

export const NOTIFY_HINT =
  'One quiet summary a day at most, and only when something new has turned up. ' +
  'It says how many and of what, never a filename or a path. Opening it takes you ' +
  'to the findings; nothing is touched.'

export function launchAtLoginHint(platform: string): string {
  return platform === 'windows'
    ? 'Starts Scuttle in the background when you sign in, through the ordinary ' +
        'per-user startup list. No service, no administrator rights.'
    : 'Starts Scuttle in the background when you log in, as a normal login item. ' +
        'No installer, no background service, no administrator rights.'
}

/** Shown when a setting is on but the system is not cooperating. */
export function trouble(status: BackgroundStatus, platform: string): string | null {
  if (status.mode && !status.tray_alive) {
    return (
      `Scuttle could not put an icon in the ${trayWord(platform)}, so closing the ` +
      'window still quits. Nothing is wrong with your files — this is Scuttle ' +
      'failing to appear, not something it found.'
    )
  }
  if (status.notify && status.notifications_permitted === false) {
    return (
      'Your system is not allowing Scuttle to send notifications. Background ' +
      'checks still run; you will see what turned up next time you open the window.'
    )
  }
  return null
}

export function launchAtLoginTrouble(status: BackgroundStatus): string | null {
  if (!status.launch_at_login_available) {
    return 'Scuttle cannot set this up on this system, so it stays off.'
  }
  return null
}

/** How the pause reads, and what the button offers. */
export function pauseState(
  pausedUntilUnix: number,
  nowUnix: number,
): { paused: boolean; label: string; says: string } {
  const paused = pausedUntilUnix > nowUnix
  return {
    paused,
    label: paused ? 'Resume' : 'Pause until tomorrow',
    says: paused ? 'Paused until tomorrow.' : '',
  }
}

/**
 * The line under the background-mode group: where things stand, in one
 * sentence, without ever claiming to know what the user is doing.
 */
export function standing(status: BackgroundStatus, nowUnix: number): string {
  if (!status.mode) return 'Off. Closing the window quits Scuttle.'
  if (!status.checks) return 'Scuttle stays in place, and looks around only when you ask.'
  if (status.paused_until_unix > nowUnix) return 'Paused until tomorrow.'
  if (status.last_check_unix === 0) return `Nothing looked at yet. ${status.waiting_because}`
  return `Last look ${whenish(nowUnix - status.last_check_unix)}. ${status.waiting_because}`
}

/** Rough on purpose: a status line does not need the minute. */
export function whenish(secondsAgo: number): string {
  const HOUR = 3600
  const DAY = 24 * HOUR
  if (secondsAgo < 2 * HOUR) return 'just now'
  if (secondsAgo < DAY) return 'today'
  if (secondsAgo < 2 * DAY) return 'yesterday'
  if (secondsAgo < 7 * DAY) return `${Math.floor(secondsAgo / DAY)} days ago`
  return 'a while ago'
}

/**
 * What the findings screen says when the last scan was a background check.
 *
 * The point is the second sentence. Without it, an empty Copies pile reads as
 * "you have no duplicates" when what actually happened is that nobody looked.
 */
export const GLANCE_CAVEAT =
  'This was a background check. It read names, sizes and dates without opening ' +
  'anything, so duplicates and near-identical screenshots are not in here — a ' +
  'rummage you start will look for those.'

export const CANCELLED_CAVEAT =
  'This look was stopped part-way. What turned up is real; what is missing was ' +
  'simply never reached.'
