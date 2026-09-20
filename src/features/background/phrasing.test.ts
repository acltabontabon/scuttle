import { describe, expect, it } from 'vitest'

import type { BackgroundStatus } from '@/lib/types'

import {
  CANCELLED_CAVEAT,
  CHECKS_HINT,
  GLANCE_CAVEAT,
  NOTIFY_HINT,
  keepInTrayHint,
  keepInTrayLabel,
  launchAtLoginHint,
  launchAtLoginTrouble,
  pauseState,
  standing,
  trayWord,
  trouble,
  whenish,
} from './phrasing'

const HOUR = 3600
const DAY = 24 * HOUR
const NOW = 1_700_000_000

function status(over: Partial<BackgroundStatus> = {}): BackgroundStatus {
  return {
    tray_alive: true,
    mode: true,
    checks: true,
    notify: false,
    notifications_permitted: null,
    launch_at_login: false,
    launch_at_login_available: true,
    last_check_unix: 0,
    paused_until_unix: 0,
    now_unix: NOW,
    pending_review: null,
    waiting_because: 'Looked recently.',
    ...over,
  }
}

/** Everything a person can be shown, gathered so one test can sweep it. */
const EVERYTHING = [
  keepInTrayLabel('macos'),
  keepInTrayLabel('windows'),
  keepInTrayHint('macos'),
  keepInTrayHint('windows'),
  CHECKS_HINT,
  NOTIFY_HINT,
  launchAtLoginHint('macos'),
  launchAtLoginHint('windows'),
  GLANCE_CAVEAT,
  CANCELLED_CAVEAT,
  trouble(status({ tray_alive: false }), 'macos') ?? '',
  trouble(status({ notify: true, notifications_permitted: false }), 'macos') ?? '',
  launchAtLoginTrouble(status({ launch_at_login_available: false })) ?? '',
  standing(status({ mode: false }), NOW),
  standing(status({ checks: false }), NOW),
  standing(status({ paused_until_unix: NOW + HOUR }), NOW),
  standing(status(), NOW),
  standing(status({ last_check_unix: NOW - DAY }), NOW),
].join('\n')

describe('what Scuttle says about staying in the menu bar', () => {
  it('never manufactures urgency', () => {
    expect(EVERYTHING).not.toMatch(/!/)
    expect(EVERYTHING).not.toMatch(
      /\b(problem|problems|critical|warning|urgent|boost|optimi[sz])/i,
    )
    expect(EVERYTHING).not.toMatch(/[A-Z]{4,}/)
  })

  it('never claims Scuttle knows what the user is doing', () => {
    // Low CPU is not idleness, and a machine on mains is not an empty chair.
    // If any of these words appear, some copy has started making a claim the
    // application cannot support.
    expect(EVERYTHING).not.toMatch(/\bidle\b/i)
    expect(EVERYTHING).not.toMatch(/\baway\b/i)
    expect(EVERYTHING).not.toMatch(/\bwhen you(?:'re| are) not\b/i)
    expect(EVERYTHING).not.toMatch(/\bdetects?\b/i)
  })

  it('never promises that nothing costs anything', () => {
    expect(EVERYTHING).not.toMatch(/zero impact/i)
    expect(EVERYTHING).not.toMatch(/no impact/i)
    expect(EVERYTHING).not.toMatch(/\bfree\b/i)
    expect(EVERYTHING).not.toMatch(/\bAI\b/)
    expect(EVERYTHING).not.toMatch(/\bautomatic(?:ally)? clean/i)
  })

  it('never suggests the files found should go', () => {
    expect(GLANCE_CAVEAT).not.toMatch(/safe to|delete|remove|clean up|free up/i)
    expect(CANCELLED_CAVEAT).not.toMatch(/safe to|delete|remove|clean up|free up/i)
  })
})

describe('the words each platform uses for itself', () => {
  it('calls it the menu bar on macOS and the system tray on Windows', () => {
    expect(trayWord('macos')).toBe('menu bar')
    expect(trayWord('windows')).toBe('system tray')
    expect(keepInTrayLabel('macos')).toContain('menu bar')
    expect(keepInTrayLabel('windows')).toContain('system tray')
  })

  it('names the right way to quit on each platform', () => {
    expect(keepInTrayHint('macos')).toContain('⌘Q')
    expect(keepInTrayHint('windows')).toContain('Alt+F4')
  })

  it('says that quitting and logging out are never obstructed', () => {
    for (const platform of ['macos', 'windows']) {
      expect(keepInTrayHint(platform)).toMatch(/still quits/)
      expect(keepInTrayHint(platform)).toMatch(/logging out|shutting down/)
    }
  })
})

describe('admitting when the system said no', () => {
  it('says so when the tray could not be created, rather than leaving a toggle lying', () => {
    // The failure this prevents: a switch reading "on" while closing the
    // window still quits, so the setting is quietly a lie.
    const said = trouble(status({ tray_alive: false }), 'macos')
    expect(said).toBeTruthy()
    expect(said).toMatch(/still quits/)
  })

  it('says nothing when everything is working', () => {
    expect(trouble(status(), 'macos')).toBeNull()
    expect(launchAtLoginTrouble(status())).toBeNull()
  })

  it('keeps the rest of the feature usable when notifications are refused', () => {
    const said = trouble(status({ notify: true, notifications_permitted: false }), 'macos')
    expect(said).toMatch(/still run/)
  })

  it('does not complain about notifications that were never asked about', () => {
    expect(trouble(status({ notify: true, notifications_permitted: null }), 'macos')).toBeNull()
  })
})

describe('the standing line', () => {
  it('says plainly that closing quits when background mode is off', () => {
    expect(standing(status({ mode: false }), NOW)).toMatch(/quits/)
  })

  it('does not imply checks are happening when only the tray is on', () => {
    const said = standing(status({ checks: false }), NOW)
    expect(said).toMatch(/only when you ask/)
  })

  it('shows a pause rather than pretending to be waiting for a good moment', () => {
    expect(standing(status({ paused_until_unix: NOW + HOUR }), NOW)).toBe('Paused until tomorrow.')
  })

  it('admits when nothing has been looked at yet', () => {
    expect(standing(status(), NOW)).toMatch(/Nothing looked at yet/)
  })
})

describe('pausing', () => {
  it('offers the opposite of whatever is currently true', () => {
    expect(pauseState(0, NOW).label).toBe('Pause until tomorrow')
    expect(pauseState(NOW + HOUR, NOW).label).toBe('Resume')
  })

  it('treats a pause whose time has passed as over', () => {
    expect(pauseState(NOW - 1, NOW).paused).toBe(false)
  })
})

describe('whenish', () => {
  it('is vague rather than wrong', () => {
    expect(whenish(60)).toBe('just now')
    expect(whenish(5 * HOUR)).toBe('today')
    expect(whenish(30 * HOUR)).toBe('yesterday')
    expect(whenish(3 * DAY)).toBe('3 days ago')
    expect(whenish(40 * DAY)).toBe('a while ago')
  })

  it('never reports a negative age from a clock that drifted', () => {
    expect(whenish(-10)).toBe('just now')
  })
})
