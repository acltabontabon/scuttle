import { describe, expect, it } from 'vitest'

import type { UpdateFailure, UpdateInfo, UpdatePhase, UpdateSnapshot } from '@/lib/types'

import { chipFor, describe as describeUpdate, parseNotes } from './view'

const NOW = 1_700_000_000_000
const info: UpdateInfo = { version: '0.1.0-alpha.3', notes: 'Fixed a thing.', date_unix: NOW / 1000 }

function snap(state: UpdatePhase, over: Partial<UpdateSnapshot> = {}): UpdateSnapshot {
  return {
    current_version: '0.1.0-alpha.2',
    channel: 'alpha',
    state,
    dismissed: false,
    blocked: null,
    error: null,
    ...over,
  }
}

const failure = (over: Partial<UpdateFailure> = {}): UpdateFailure => ({
  stage: 'check',
  message: 'Scuttle could not reach the update server.',
  manual: true,
  ...over,
})

describe('the header chip', () => {
  it('is absent unless there is something to say', () => {
    expect(chipFor(null)).toBeNull()
    expect(chipFor(snap({ phase: 'idle' }))).toBeNull()
    expect(chipFor(snap({ phase: 'checking' }))).toBeNull()
    expect(chipFor(snap({ phase: 'up_to_date', checked_unix: 1 }))).toBeNull()
  })

  it('announces an available update once', () => {
    const chip = chipFor(snap({ phase: 'available', info }))
    expect(chip?.text).toBe('Update available · 0.1.0-alpha.3')
    expect(chip?.mode).toBe('done')
  })

  it('goes quiet for a version that was dismissed', () => {
    expect(chipFor(snap({ phase: 'available', info }, { dismissed: true }))).toBeNull()
    expect(chipFor(snap({ phase: 'ready', info }, { dismissed: true }))).toBeNull()
  })

  it('says a downloaded update is ready', () => {
    expect(chipFor(snap({ phase: 'ready', info }))?.text).toBe('Update ready to install')
  })

  it('shows a percentage only when the size is known', () => {
    const known = chipFor(snap({ phase: 'downloading', info, received: 250, total: 1000 }))
    expect(known).toMatchObject({ text: 'Downloading update · 25%', mode: 'determinate', fraction: 0.25 })

    const unknown = chipFor(snap({ phase: 'downloading', info, received: 250, total: null }))
    expect(unknown).toMatchObject({ text: 'Downloading update…', mode: 'indeterminate' })
  })

  it('keeps showing a download the person started, even if the notice was dismissed', () => {
    const chip = chipFor(
      snap({ phase: 'downloading', info, received: 1, total: 10 }, { dismissed: true }),
    )
    expect(chip).not.toBeNull()
  })

  it('shows installing, always', () => {
    expect(chipFor(snap({ phase: 'installing', info }, { dismissed: true }))?.text).toBe('Installing…')
  })

  it('stays silent about a failed background check', () => {
    expect(
      chipFor(snap({ phase: 'idle' }, { error: failure({ manual: false }) })),
    ).toBeNull()
  })

  it('does not chip a failed check even when it was manual; Settings says that', () => {
    expect(chipFor(snap({ phase: 'idle' }, { error: failure() }))).toBeNull()
  })

  it('speaks up when a download or install did not finish', () => {
    for (const stage of ['download', 'install'] as const) {
      const chip = chipFor(
        snap({ phase: 'available', info }, { dismissed: true, error: failure({ stage }) }),
      )
      expect(chip).toMatchObject({ text: 'Update did not finish', tone: 'warn' })
    }
  })

  it('shows that an install is waiting when it was refused', () => {
    const chip = chipFor(
      snap({ phase: 'ready', info }, { dismissed: true, blocked: 'Scuttle is moving files.' }),
    )
    expect(chip?.text).toBe('Update is waiting')
  })
})

describe('the description', () => {
  it('offers a check when nothing is known', () => {
    const d = describeUpdate(snap({ phase: 'idle' }))
    expect(d.action).toEqual({ kind: 'check', label: 'Check for updates' })
  })

  it('says how long ago it looked', () => {
    const d = describeUpdate(snap({ phase: 'up_to_date', checked_unix: NOW / 1000 - 600 }), NOW)
    expect(d.headline).toBe('Scuttle is up to date')
    expect(d.detail).toBe('Checked 10 minutes ago.')
  })

  it('offers the download, and says nothing has been downloaded', () => {
    const d = describeUpdate(snap({ phase: 'available', info }))
    expect(d.action).toEqual({ kind: 'download', label: 'Download' })
    expect(d.detail).toBe('Nothing has been downloaded yet.')
    expect(d.info?.notes).toBe('Fixed a thing.')
  })

  it('offers nothing while it is working, and does not block on an unknown size', () => {
    const d = describeUpdate(snap({ phase: 'downloading', info, received: 1024, total: null }))
    expect(d.action).toBeNull()
    expect(d.working).toBe(true)
    expect(d.track).toEqual({ mode: 'indeterminate', fraction: 0 })
    expect(d.detail).toContain('1.00 KB so far')
  })

  it('has a determinate track for a known size', () => {
    const d = describeUpdate(snap({ phase: 'downloading', info, received: 500, total: 1000 }))
    expect(d.track).toEqual({ mode: 'determinate', fraction: 0.5 })
  })

  it('offers update-and-restart once downloaded, and says what that does', () => {
    const d = describeUpdate(snap({ phase: 'ready', info }))
    expect(d.action).toEqual({ kind: 'install', label: 'Update and restart' })
    expect(d.notice).toMatch(/closes Scuttle and opens the new version/)
  })

  it('says why an install was refused, and offers to try again', () => {
    const d = describeUpdate(
      snap(
        { phase: 'ready', info },
        { blocked: 'Scuttle can’t restart while it’s moving files into the Drawer.' },
      ),
    )
    expect(d.blocked).toContain('moving files into the Drawer')
    expect(d.action?.label).toBe('Try again')
  })

  it('does not report a failed background check', () => {
    const d = describeUpdate(snap({ phase: 'idle' }, { error: failure({ manual: false }) }))
    expect(d.failure).toBeNull()
  })

  it('reports a failed manual check and offers to try again', () => {
    const d = describeUpdate(snap({ phase: 'idle' }, { error: failure() }))
    expect(d.failure).toContain('could not reach')
    expect(d.action?.label).toBe('Try again')
  })

  it('reports a failed download and leaves the update available to retry', () => {
    const d = describeUpdate(
      snap({ phase: 'available', info }, { error: failure({ stage: 'download', message: 'Stopped.' }) }),
    )
    expect(d.failure).toBe('Stopped.')
    expect(d.action).toEqual({ kind: 'download', label: 'Try again' })
  })

  it('reports why updates are not available at all, with nothing to click', () => {
    const d = describeUpdate(snap({ phase: 'unavailable', reason: 'This one is not installed.' }))
    expect(d.detail).toBe('This one is not installed.')
    expect(d.action).toBeNull()
  })

  it('does not show a stale failure while working', () => {
    const d = describeUpdate(snap({ phase: 'checking' }, { error: failure() }))
    expect(d.failure).toBeNull()
    expect(d.working).toBe(true)
  })
})

describe('parseNotes', () => {
  it('reads headings, list items and paragraphs', () => {
    const lines = parseNotes('### Added\n- A thing.\n- Another **bold** thing.\n\nSome words.')
    expect(lines).toEqual([
      { kind: 'heading', text: 'Added' },
      { kind: 'item', text: 'A thing.' },
      { kind: 'item', text: 'Another bold thing.' },
      { kind: 'text', text: 'Some words.' },
    ])
  })

  it('shows links and code as their words, never as markup', () => {
    expect(parseNotes('- See [the docs](https://example.test) and `--flag`.')).toEqual([
      { kind: 'item', text: 'See the docs and --flag.' },
    ])
  })

  it('leaves release text as text, whatever it contains', () => {
    const [line] = parseNotes('<img src=x onerror=alert(1)>')
    expect(line).toEqual({ kind: 'text', text: '<img src=x onerror=alert(1)>' })
  })

  it('is empty when there are no notes', () => {
    expect(parseNotes(null)).toEqual([])
    expect(parseNotes('  \n ')).toEqual([])
  })
})
