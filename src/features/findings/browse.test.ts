import { describe, expect, it } from 'vitest'
import { CANDIDATES } from '@/dev/fixtures'
import { browse } from './browse'

describe('browsing findings', () => {
  it('finds names and paths regardless of case or surrounding whitespace', () => {
    const item = CANDIDATES[0]!
    expect(browse(CANDIDATES, ` ${item.display_name.toUpperCase()} `, 'scan')).toContain(item)
    expect(browse(CANDIDATES, item.path, 'scan')).toContain(item)
    expect(browse(CANDIDATES, 'no-match-at-all-123456', 'scan')).toEqual([])
  })

  it('sorts by size without reordering the source findings', () => {
    const before = CANDIDATES.map((item) => item.id)
    const result = browse(CANDIDATES, '', 'largest')
    expect(result.map((item) => item.size)).toEqual(CANDIDATES.map((item) => item.size).sort((a, b) => b - a))
    expect(CANDIDATES.map((item) => item.id)).toEqual(before)
  })

  it('keeps natural filename order and restores scan order', () => {
    const item = CANDIDATES[0]!
    const items = [{ ...item, display_name: 'capture 10' }, { ...item, display_name: 'capture 2' }]
    expect(browse(items, '', 'name').map((item) => item.display_name)).toEqual(['capture 2', 'capture 10'])
    expect(browse(items, ' ', 'scan')).toEqual(items)
  })
})
