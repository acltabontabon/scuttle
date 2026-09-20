/**
 * Building a selection by hand.
 *
 * A pile can hold two hundred things, so picking them one at a time is not a
 * selection tool, it is a chore. A run — click one, shift-click another,
 * everything between them follows — is what makes a field of four hundred
 * screenshots something a person can actually deal with.
 *
 * The rule lives here rather than inside the click handler because it has
 * real edges: which end was clicked first, whether the run turns things on or
 * off, and what happens when there is nothing to measure from.
 */

/** Anything with an id, in the order it is shown. */
interface Pickable {
  id: string
}

/**
 * Apply one click to a selection.
 *
 * `from` is the previously clicked item, or `null` for a plain click and for
 * a shift-click with nothing to reach back to. When there is a run, whether
 * it adds or removes is decided by the item that was clicked: shift-clicking
 * into an unpicked item picks the whole run, and into a picked one drops it.
 */
export function applyPick(
  order: readonly Pickable[],
  picked: ReadonlySet<string>,
  id: string,
  from: string | null,
): ReadonlySet<string> {
  const next = new Set(picked)

  if (from !== null && from !== id) {
    const a = order.findIndex((item) => item.id === from)
    const b = order.findIndex((item) => item.id === id)

    // Either end may have gone — the list is rebuilt after every rummage, and
    // the anchor is only a remembered id. A run that cannot be measured falls
    // back to an ordinary toggle rather than doing nothing.
    if (a >= 0 && b >= 0) {
      const turningOn = !next.has(id)
      for (let i = Math.min(a, b); i <= Math.max(a, b); i += 1) {
        const at = order[i]!.id
        if (turningOn) next.add(at)
        else next.delete(at)
      }
      return next
    }
  }

  if (!next.delete(id)) next.add(id)
  return next
}
