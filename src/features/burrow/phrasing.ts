/**
 * What Scuttle says in its burrow.
 *
 * One short line, chosen from facts the core reported. The rules:
 *
 *  - Only ever about something true right now. No invented findings, no
 *    claims about a particular folder Scuttle has not said anything about.
 *  - Never a file name, never a path.
 *  - Never a joke when something needs attention, something is busy, or the
 *    person has asked Scuttle to be quiet. Those get plain words.
 *  - Occasional: most of the time the plain line is the line.
 */

export interface BurrowStatus {
  now_unix: number
  has_looked: boolean
  last_look_unix: number | null
  found: number
  suggested: number
  drawer_items: number
  drawer_bytes: number
  needs_attention: number
  busy: string | null
  personality: 'full' | 'quiet'
  reduced_motion: boolean | null
  appearance: 'system' | 'light' | 'dark' | string
}

/** How often a playful line replaces the plain one, out of 1. */
export const PLAYFUL_SHARE = 0.4

/**
 * The line. `roll` is a number in [0, 1) — random in the app, fixed in tests —
 * deciding whether this opening gets a playful line at all, and which.
 */
export function burrowLine(status: BurrowStatus, roll: number): string {
  if (status.needs_attention > 0) {
    return status.needs_attention === 1
      ? 'Something in the Drawer needs a look after an interruption.'
      : `${status.needs_attention} things in the Drawer need a look after an interruption.`
  }
  if (status.busy) return `Busy ${status.busy}.`

  const plain = plainLine(status)
  if (status.personality === 'quiet' || roll >= PLAYFUL_SHARE) return plain

  const options = playfulLines(status)
  if (options.length === 0) return plain
  const pick = Math.floor((roll / PLAYFUL_SHARE) * options.length)
  return options[Math.min(pick, options.length - 1)]!
}

function plainLine(status: BurrowStatus): string {
  if (!status.has_looked) return 'Scuttle has not looked around yet.'
  if (status.found === 0 && status.drawer_items === 0) return 'Nothing to report.'
  if (status.found === 0) return 'Nothing new since the last look.'
  return status.found === 1 ? 'One thing worth a look.' : `${status.found} things worth a look.`
}

/** Only lines that are true of the current state. */
function playfulLines(status: BurrowStatus): string[] {
  if (!status.has_looked) return ['Ready when you are. I brought my own claws.']
  const lines: string[] = []
  if (status.found > 0) lines.push('I found things. I have questions.')
  if (status.found === 0 && status.drawer_items === 0) {
    lines.push('Nothing moved. I’m a crab, not a landlord.')
  }
  if (status.drawer_items === 0 && status.found > 0) {
    lines.push('The drawer is empty. Suspiciously responsible.')
  }
  if (status.drawer_items > 0) lines.push('Filed under: future you’s problem.')
  return lines
}

/** Rough on purpose: a tray does not need the minute. */
export function lastLook(status: BurrowStatus): string {
  if (!status.has_looked || status.last_look_unix === null) return 'Not yet'
  const ago = status.now_unix - status.last_look_unix
  const hour = 3600
  const day = 24 * hour
  if (ago < 2 * hour) return 'Just now'
  if (ago < day) return 'Today'
  if (ago < 2 * day) return 'Yesterday'
  if (ago < 7 * day) return `${Math.floor(ago / day)} days ago`
  return 'A while ago'
}
