/**
 * Presentation helpers.
 *
 * `bytes` deliberately mirrors `human_bytes` in `src-tauri/src/model.rs`:
 * the same number must read the same way wherever it appears, and a round
 * trip through IPC just to format a size would be absurd.
 */

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'] as const

export function bytes(value: number): string {
  let size = value
  let unit = 0
  while (size >= 1024 && unit < UNITS.length - 1) {
    size /= 1024
    unit += 1
  }
  if (unit === 0) return `${value} B`
  if (size >= 100) return `${size.toFixed(0)} ${UNITS[unit]}`
  if (size >= 10) return `${size.toFixed(1)} ${UNITS[unit]}`
  return `${size.toFixed(2)} ${UNITS[unit]}`
}

/** Just the number and unit, split, for typographic layouts. */
export function bytesParts(value: number): { value: string; unit: string } {
  const whole = bytes(value)
  const space = whole.lastIndexOf(' ')
  return { value: whole.slice(0, space), unit: whole.slice(space + 1) }
}

export function daysAgo(unix: number | null, now = Date.now()): number | null {
  if (unix === null) return null
  return Math.max(0, Math.floor((now / 1000 - unix) / 86400))
}

/** "today", "yesterday", "8 days ago", "about 2 years ago". */
export function whenish(unix: number | null, now = Date.now()): string {
  const days = daysAgo(unix, now)
  if (days === null) return 'at some point'
  if (days === 0) return 'today'
  if (days === 1) return 'yesterday'
  if (days < 45) return `${days} days ago`
  if (days < 365) return `about ${Math.round(days / 30)} months ago`
  const years = days / 365
  if (years < 1.5) return 'about a year ago'
  return `about ${Math.round(years)} years ago`
}

/** Days until a date, floored at zero. */
export function daysUntil(unix: number, now = Date.now()): number {
  return Math.max(0, Math.ceil((unix - now / 1000) / 86400))
}

/**
 * Shorten a path for display without hiding which file it is: the last two
 * components always survive, and the home directory becomes `~`.
 */
export function shortPath(path: string, home?: string): string {
  let display = path
  if (home && display.startsWith(home)) {
    display = `~${display.slice(home.length)}`
  }
  const separator = display.includes('\\') ? '\\' : '/'
  // Keep whatever the path started with — a leading slash or a drive letter is
  // the difference between "this file" and "a file with the same name".
  const rooted = display.startsWith(separator)
  const parts = display.split(separator).filter(Boolean)
  if (parts.length <= 3) return display
  const shortened = [parts[0], '…', ...parts.slice(-2)].join(separator)
  return rooted ? separator + shortened : shortened
}

/** A stable pseudo-random number in [0, 1) derived from a string. */
export function scatter(seed: string, salt = 0): number {
  let hash = 2166136261 ^ salt
  for (let i = 0; i < seed.length; i += 1) {
    hash ^= seed.charCodeAt(i)
    hash = Math.imul(hash, 16777619)
  }
  return ((hash >>> 0) % 100000) / 100000
}
