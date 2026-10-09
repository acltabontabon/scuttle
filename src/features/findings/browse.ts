import type { Candidate } from '@/lib/types'

export type SortOrder = 'scan' | 'largest' | 'name'

/** Browsing never mutates the scan's order or the user's selection. */
export function browse(items: readonly Candidate[], query: string, sort: SortOrder): Candidate[] {
  const needle = query.trim().toLocaleLowerCase()
  const matches = items.filter((item) =>
    !needle || [item.display_name, item.path, item.remark ?? ''].some((text) => text.toLocaleLowerCase().includes(needle)),
  )
  if (sort === 'largest') matches.sort((a, b) => b.size - a.size)
  if (sort === 'name') matches.sort((a, b) => a.display_name.localeCompare(b.display_name, undefined, { numeric: true }))
  return matches
}
