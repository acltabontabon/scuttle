import { describe, expect, it } from 'vitest'

import type { FindingResult, IssueGroup, MoveReport } from '@/lib/types'
import { adviceFor, describeReport, diagnostics, technicalLine } from './phrasing'

function issue(kind: IssueGroup['kind'], count: number, over: Partial<IssueGroup> = {}): IssueGroup {
  return {
    kind,
    phase: 'publish',
    os_code: null,
    count,
    samples: ['a.bin'],
    next_step: 'retry',
    explanation: 'x',
    ...over,
  }
}

function finding(over: Partial<FindingResult> = {}): FindingResult {
  return {
    finding_id: 'f1',
    display_name: 'Owner cache',
    category: 'caches',
    unit: 'files',
    status: 'moved',
    moved: 0,
    skipped: 0,
    failed: 0,
    moved_bytes: 0,
    record_id: null,
    needs_refresh: false,
    retryable: false,
    issues: [],
    refusal: null,
    ...over,
  }
}

function report(over: Partial<MoveReport> = {}, findings: FindingResult[] = []): MoveReport {
  const moved = findings.reduce((n, f) => n + f.moved, 0)
  return {
    outcome: 'completed',
    moved_files: moved,
    moved_bytes: 0,
    skipped: 0,
    failed: 0,
    remaining: 0,
    cancelled: false,
    kept: null,
    findings,
    notice: null,
    ...over,
  }
}

describe('describeReport', () => {
  it('says what moved when everything did', () => {
    const r = report({}, [finding({ moved: 418 })])
    expect(describeReport(r)).toMatchObject({
      headline: 'Moved 418 files into Drawer.',
      tone: 'plain',
    })
  })

  it('names a single whole item rather than counting it', () => {
    const r = report({}, [finding({ unit: 'item', display_name: 'Chrome.dmg', moved: 1 })])
    expect(describeReport(r).headline).toBe('Moved Chrome.dmg into Drawer.')
  })

  it('says "in use" only when the failures were in-use failures', () => {
    const r = report({ outcome: 'partial' }, [
      finding({
        status: 'partial',
        moved: 418,
        failed: 6,
        issues: [issue('in_use', 6, { os_code: 32, next_step: 'close_app' })],
        retryable: true,
      }),
    ])
    expect(describeReport(r).headline).toBe(
      'Moved 418 files into Drawer. Skipped 6 that were in use.',
    )
  })

  it('does not call a permission refusal "in use"', () => {
    const r = report({ outcome: 'failed' }, [
      finding({
        status: 'failed',
        failed: 6,
        issues: [issue('access_denied', 6, { os_code: 5, next_step: 'leave_alone' })],
      }),
    ])
    const { headline } = describeReport(r)
    expect(headline).toBe("Scuttle couldn't access this folder.")
    expect(headline.toLowerCase()).not.toContain('in use')
  })

  it('says files changed since the scan only for files that changed, and offers a review', () => {
    const r = report({ outcome: 'partial' }, [
      finding({
        status: 'partial',
        moved: 20,
        skipped: 2,
        needs_refresh: true,
        issues: [issue('stale', 1), issue('missing', 1)],
      }),
    ])
    const outcome = describeReport(r)
    expect(outcome.headline).toBe(
      'Moved 20 files into Drawer. Some files changed since the scan. Review them again.',
    )
    expect(outcome.reviewIds).toEqual(['f1'])
  })

  it('offers a retry only for findings where trying again could help', () => {
    const r = report({ outcome: 'partial' }, [
      finding({ finding_id: 'a', status: 'partial', moved: 1, retryable: true, issues: [issue('in_use', 1)] }),
      finding({ finding_id: 'b', status: 'skipped', retryable: false, issues: [issue('stale', 1)] }),
    ])
    expect(describeReport(r).retryIds).toEqual(['a'])
  })

  it('never claims that space was freed', () => {
    const outcomes = [
      report({}, [finding({ moved: 5 })]),
      report({ outcome: 'partial' }, [finding({ moved: 5, issues: [issue('in_use', 1)] })]),
      report({ outcome: 'cancelled', remaining: 3, cancelled: true }, [finding({ moved: 5 })]),
    ]
    for (const r of outcomes) {
      expect(describeReport(r).headline.toLowerCase()).not.toMatch(/freed|reclaim|space back/)
    }
  })

  it('reports a stop honestly: what moved and what was left', () => {
    const r = report({ outcome: 'cancelled', remaining: 388, cancelled: true }, [finding({ moved: 12 })])
    expect(describeReport(r).headline).toBe(
      'Stopped. Moved 12 files into Drawer; 388 left where they were.',
    )
    expect(
      describeReport(report({ outcome: 'cancelled', remaining: 5, cancelled: true }, [finding()])).headline,
    ).toBe('Stopped before anything moved.')
  })

  it('tells someone the drive is full, and only then', () => {
    const full = report({ outcome: 'partial' }, [finding({ moved: 3, issues: [issue('no_space', 7)] })])
    expect(describeReport(full).headline).toContain("The drawer's drive is full")
    const notFull = report({ outcome: 'partial' }, [finding({ moved: 3, issues: [issue('in_use', 7)] })])
    expect(describeReport(notFull).headline).not.toContain('full')
  })

  it("uses Scuttle's own reason when it declined before touching anything", () => {
    const r = report({ outcome: 'failed' }, [
      finding({
        status: 'skipped',
        unit: 'item',
        refusal: { code: 'stale', message: 'Some of this changed since it was reviewed. Review it again.' },
      }),
    ])
    expect(describeReport(r).headline).toBe(
      'Some of this changed since it was reviewed. Review it again.',
    )
  })

  it('puts a crash notice first and calls it a warning', () => {
    const r = report({ outcome: 'failed', notice: 'Scuttle stopped unexpectedly while moving.' })
    expect(describeReport(r)).toMatchObject({ tone: 'warn', headline: 'Scuttle stopped unexpectedly while moving.' })
  })

  it('keeps the headline to two sentences however much went wrong', () => {
    const r = report({ outcome: 'partial' }, [
      finding({
        moved: 4,
        issues: [issue('in_use', 2), issue('stale', 1), issue('access_denied', 1), issue('no_space', 1)],
      }),
    ])
    const sentences = describeReport(r).headline.split(/(?<=\.)\s/)
    expect(sentences.length).toBeLessThanOrEqual(3)
  })
})

describe('the details', () => {
  it('advises closing an app only for the in-use next step', () => {
    for (const step of ['refresh', 'retry', 'free_space', 'leave_alone'] as const) {
      expect(adviceFor(step).toLowerCase()).not.toContain('close')
    }
    expect(adviceFor('close_app').toLowerCase()).toContain('close')
  })

  it('keeps the code and the phase where a person can find them', () => {
    expect(technicalLine(issue('in_use', 1, { os_code: 32, phase: 'publish' }))).toBe(
      'in use · while publish · OS error 32',
    )
  })

  it('writes diagnostics with names and codes but never a path', () => {
    const r = report({ outcome: 'partial' }, [
      finding({
        status: 'partial',
        moved: 2,
        issues: [issue('in_use', 1, { os_code: 32, samples: ['locked.bin'] })],
      }),
    ])
    const text = diagnostics(r)
    expect(text).toContain('in_use × 1')
    expect(text).toContain('os code: 32')
    expect(text).toContain('locked.bin')
    expect(text).not.toMatch(/[A-Za-z]:\\|\/Users\/|\/home\//)
  })
})
