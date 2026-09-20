import { describe, expect, it } from 'vitest'

import { applyPick } from './selection'

const LIST = [{ id: 'a' }, { id: 'b' }, { id: 'c' }, { id: 'd' }, { id: 'e' }]
const set = (...ids: string[]) => new Set(ids)
const sorted = (s: ReadonlySet<string>) => [...s].sort()

/**
 * Selecting a run is the difference between a pile of four hundred
 * screenshots being workable and being a chore nobody finishes.
 */
describe('applyPick', () => {
  it('toggles a single item when there is no run', () => {
    expect(sorted(applyPick(LIST, set(), 'b', null))).toEqual(['b'])
    expect(sorted(applyPick(LIST, set('b'), 'b', null))).toEqual([])
  })

  it('picks everything between the two ends', () => {
    expect(sorted(applyPick(LIST, set('b'), 'd', 'b'))).toEqual(['b', 'c', 'd'])
  })

  it('reads a run backwards as happily as forwards', () => {
    expect(sorted(applyPick(LIST, set('d'), 'b', 'd'))).toEqual(['b', 'c', 'd'])
  })

  it('drops a run when the item shift-clicked into was already picked', () => {
    const all = set('a', 'b', 'c', 'd', 'e')
    expect(sorted(applyPick(LIST, all, 'd', 'b'))).toEqual(['a', 'e'])
  })

  it('keeps what was already picked outside the run', () => {
    expect(sorted(applyPick(LIST, set('a', 'c'), 'e', 'c'))).toEqual(['a', 'c', 'd', 'e'])
  })

  it('falls back to a toggle when the anchor is gone', () => {
    // The anchor is only a remembered id, and the list is rebuilt after every
    // rummage. An unmeasurable run must still do the obvious thing.
    expect(sorted(applyPick(LIST, set(), 'c', 'vanished'))).toEqual(['c'])
  })

  it('treats a run of one as a toggle', () => {
    expect(sorted(applyPick(LIST, set(), 'c', 'c'))).toEqual(['c'])
  })

  it('does not mutate the set it was given', () => {
    const before = set('a')
    applyPick(LIST, before, 'd', 'a')
    expect(sorted(before)).toEqual(['a'])
  })
})
