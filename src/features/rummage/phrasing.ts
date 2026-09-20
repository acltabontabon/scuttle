import type { Phase } from '@/lib/types'

/**
 * What Scuttle says while it works.
 *
 * Every line maps onto a phase the core actually reported, so the narration
 * is always true. There is no fake progress here and no filler: if the core
 * stops sending phases, the line stops changing.
 */
export function phaseLine(phase: Phase): string {
  switch (phase.phase) {
    case 'preparing':
      return 'Working out what is installed…'
    case 'rummaging':
      return `Rummaging through ${phase.area}…`
    case 'examining':
      return `${phase.note}…`
    case 'considering':
      return `${phase.note}…`
    case 'finished':
      return 'Done.'
  }
}

/**
 * The line shown once a rummage ends. Empty results get the most attention,
 * because that is the moment a cleanup app usually starts lying.
 */
export function outcomeLine(found: number, cancelled: boolean): string {
  if (cancelled) return 'Stopped.'
  if (found === 0) return 'Nothing interesting.'
  if (found === 1) return 'Found something.'
  return 'Found a few things.'
}

export function outcomeAside(found: number, cancelled: boolean): string {
  // Two facts the stopped state has to carry: the sweep was partial, and
  // nothing was acted on. Neither is obvious from "Stopped." alone.
  if (cancelled) return 'A partial look. What turned up is still there, untouched.'
  if (found === 0) return 'Your computer is suspiciously tidy.'
  return 'Have a look at what turned up.'
}
