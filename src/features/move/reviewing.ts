/**
 * The words of the review a move gets before anything moves.
 *
 * Plain on purpose. This is the moment a person decides what happens to their
 * files, so nothing here is a joke, nothing claims more than the core said,
 * and nothing calls a download junk or an old file unused.
 */

import type { CautionKind, Impact, MovePlan, PlannedItem } from '@/lib/types'

/** What the move takes, as a person would check it. */
export function shapeLine(item: PlannedItem): string {
  switch (item.shape) {
    case 'file':
      return 'One file'
    case 'folder':
      if (item.contains === null) return 'A whole folder, with everything in it'
      if (item.contains === 0) return 'A whole folder (empty)'
      return `A whole folder, with ${count(item.contains, 'thing')} in it`
    case 'files_inside':
      return 'The files Scuttle reviewed inside this folder. The folder itself stays.'
  }
}

/** What moving it could disrupt. Never "safe", never "junk". */
export const IMPACT_WORDS: Record<Impact, string> = {
  regenerable: 'Rebuilt by its owner when needed',
  redownloadable: 'Can be downloaded again',
  personal_file: 'Your file',
  application_data: "An application's data",
  application_install: 'Part of an application',
}

/** Short labels for the acknowledgement checkboxes. */
export const CAUTION_LABEL: Record<CautionKind, string> = {
  recently_changed: 'Changed recently',
  may_be_your_work: 'May be your work',
  in_use: 'In use',
  application_data: 'Application data',
  breaks_application: 'Will likely break an application',
  uncertain: 'Scuttle is unsure',
}

/** Every caution in the plan is acknowledged, and something can move. */
export function canConfirm(plan: MovePlan, acknowledged: ReadonlySet<CautionKind>): boolean {
  return plan.ready > 0 && plan.cautions.every((c) => acknowledged.has(c.kind))
}

/**
 * The confirming button's words. An application folder gets its own, so it
 * is never mistaken for tidying.
 */
export function confirmLabel(plan: MovePlan): string {
  const ready = plan.items.filter((i) => i.status === 'ready')
  if (ready.length === 1 && ready[0]!.impact === 'application_install') {
    return 'Move this application folder'
  }
  if (ready.length === 1) return 'Move it to the Drawer'
  return `Move ${ready.length} to the Drawer`
}

/** How getting it back works, and whether the Drawer will ever let go of it. */
export function restoreLines(plan: MovePlan): string[] {
  const ready = plan.items.filter((i) => i.status === 'ready')
  const lines = [
    'Nothing is deleted. You can put anything back from the Drawer, to where it was. ' +
      'If something new is there by then, Scuttle puts it beside it as “(restored)” and never ' +
      'overwrites.',
  ]
  const kept = ready.filter((i) => i.kept_until_removed).length
  const expiring = ready.length - kept
  if (kept > 0 && expiring === 0) {
    lines.push('It stays in the Drawer until you remove it. It never expires by itself.')
  } else if (kept === 0 && expiring > 0) {
    lines.push(
      `It can be rebuilt or fetched again, so the Drawer lets it go after ${plan.retention_days} ` +
        'days unless you restore it first.',
    )
  } else if (kept > 0) {
    const stay = kept === 1 ? 'One item stays' : `${count(kept, 'item')} stay`
    const go =
      expiring === 1
        ? 'The one that can be rebuilt or fetched again leaves'
        : `The ${count(expiring, 'item')} that can be rebuilt or fetched again leave`
    lines.push(`${stay} until you remove ${kept === 1 ? 'it' : 'them'}. ${go} the Drawer after ${plan.retention_days} days.`)
  }
  if (ready.some((i) => i.impact === 'application_install')) {
    lines.push(
      'Putting an application folder back usually makes the application work again, but ' +
        'Scuttle cannot promise it: the application may have changed or updated meanwhile.',
    )
  }
  return lines
}

function count(n: number, noun: string): string {
  return `${n.toLocaleString()} ${noun}${n === 1 ? '' : 's'}`
}
