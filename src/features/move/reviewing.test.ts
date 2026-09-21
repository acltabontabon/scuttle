import { describe, expect, it } from 'vitest'

import type { MovePlan, PlannedItem } from '@/lib/types'

import { canConfirm, confirmLabel, restoreLines, shapeLine } from './reviewing'

function item(partial: Partial<PlannedItem>): PlannedItem {
  return {
    finding_id: 'f',
    display_name: 'thing',
    path: '/Users/r/Downloads/thing',
    shape: 'file',
    size: 10,
    contains: null,
    confidence: 'high',
    reasons: [],
    remark: null,
    impact: 'personal_file',
    eligibility: 'by_choice',
    cautions: [],
    status: 'ready',
    note: null,
    kept_until_removed: true,
    ...partial,
  }
}

function plan(items: PlannedItem[], cautions: MovePlan['cautions'] = []): MovePlan {
  return {
    items,
    cautions,
    ready: items.filter((i) => i.status === 'ready').length,
    ready_bytes: 0,
    drawer: '/drawer',
    retention_days: 14,
  }
}

describe('the review of a move', () => {
  it('says plainly whether a whole folder moves', () => {
    expect(shapeLine(item({ shape: 'file' }))).toBe('One file')
    expect(shapeLine(item({ shape: 'folder', contains: 1204 }))).toContain('1,204 things')
    expect(shapeLine(item({ shape: 'files_inside' }))).toContain('folder itself stays')
  })

  it('needs every caution acknowledged, once, before it can go ahead', () => {
    const p = plan([item({})], [
      { kind: 'recently_changed', headline: '', count: 3 },
      { kind: 'uncertain', headline: '', count: 1 },
    ])
    expect(canConfirm(p, new Set())).toBe(false)
    expect(canConfirm(p, new Set(['recently_changed']))).toBe(false)
    expect(canConfirm(p, new Set(['recently_changed', 'uncertain']))).toBe(true)
  })

  it('never offers to confirm a move where nothing can move', () => {
    expect(canConfirm(plan([item({ status: 'refused' })]), new Set())).toBe(false)
  })

  it('names an application folder for what it is', () => {
    expect(confirmLabel(plan([item({ impact: 'application_install' })]))).toBe(
      'Move this application folder',
    )
    expect(confirmLabel(plan([item({}), item({ finding_id: 'g' })]))).toBe('Move 2 to the Drawer')
  })

  it('tells the truth about whether the Drawer will let go', () => {
    expect(restoreLines(plan([item({ kept_until_removed: true })])).join(' ')).toContain(
      'never expires',
    )
    expect(
      restoreLines(plan([item({ kept_until_removed: false, impact: 'regenerable' })])).join(' '),
    ).toContain('after 14 days')
    const lines = restoreLines(plan([item({ impact: 'application_install' })])).join(' ')
    expect(lines).toContain('cannot promise')
  })

  it('never jokes and never calls anything junk', () => {
    const text = restoreLines(plan([item({})])).join(' ') + shapeLine(item({}))
    expect(text.toLowerCase()).not.toMatch(/junk|unused|lol|!/)
  })
})
