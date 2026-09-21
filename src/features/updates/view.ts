import type { UpdateInfo, UpdateSnapshot } from '@/lib/types'
import {
  CHECK,
  CHECK_AGAIN,
  checkedAgo,
  DOWNLOAD,
  INSTALL,
  KEEP_WORKING,
  NOT_YET_DOWNLOADED,
  percent,
  RESTARTING_NOTICE,
  RESTART_NOTICE,
  sizeLine,
  TRY_AGAIN,
} from './phrasing'

/**
 * What the interface shows for a given snapshot, decided in one place.
 *
 * The components are drawing only. Which button is offered, whether the chip
 * is there at all, and what it says are decisions with rules attached — a
 * dismissed update stays quiet, a failed background check stays quiet, a
 * download of unknown size does not get a percentage — and rules are easiest to
 * keep, and to test, as plain functions of the snapshot.
 */

export type ActionKind = 'check' | 'download' | 'install'

export interface Action {
  kind: ActionKind
  label: string
}

export interface Track {
  /** Indeterminate when the size is not known, rather than a made-up fraction. */
  mode: 'indeterminate' | 'determinate'
  fraction: number
}

// ---- the header chip ------------------------------------------------------

export interface Chip {
  text: string
  tone: 'plain' | 'warn'
  /** 'done' is a still dot: there is something to see, and nothing in motion. */
  mode: 'indeterminate' | 'determinate' | 'done'
  fraction: number
}

/**
 * The subtle notice in the header, or nothing.
 *
 * Nothing is the usual answer. It appears for an update nobody has waved away
 * yet, for anything in motion (a download the person asked for, an install),
 * and for a failure of something they asked for. It does *not* appear for a
 * dismissed version, for "up to date", or for a background check that failed:
 * none of those is worth an interruption.
 */
export function chipFor(snapshot: UpdateSnapshot | null): Chip | null {
  if (!snapshot) return null
  const { state, error, blocked, dismissed } = snapshot

  if (state.phase === 'installing') {
    return { text: 'Installing…', tone: 'plain', mode: 'indeterminate', fraction: 0 }
  }

  if (state.phase === 'downloading') {
    const pct = percent(state.received, state.total)
    return pct === null
      ? { text: 'Downloading update…', tone: 'plain', mode: 'indeterminate', fraction: 0 }
      : {
          text: `Downloading update · ${pct}%`,
          tone: 'plain',
          mode: 'determinate',
          fraction: pct / 100,
        }
  }

  // Something the person started did not finish. Shown even if the version was
  // dismissed: they were just here.
  if (error && error.stage !== 'check') {
    return { text: 'Update did not finish', tone: 'warn', mode: 'done', fraction: 0 }
  }

  // They asked to install and it was refused. The reason is in the panel.
  if (state.phase === 'ready' && blocked) {
    return { text: 'Update is waiting', tone: 'plain', mode: 'done', fraction: 0 }
  }

  if (dismissed) return null

  if (state.phase === 'available') {
    return { text: `Update available · ${state.info.version}`, tone: 'plain', mode: 'done', fraction: 0 }
  }
  if (state.phase === 'ready') {
    return { text: 'Update ready to install', tone: 'plain', mode: 'done', fraction: 0 }
  }
  return null
}

// ---- the description, for the panel and for Settings ----------------------

export interface Description {
  stage: UpdateSnapshot['state']['phase']
  headline: string
  detail: string | null
  info: UpdateInfo | null
  track: Track | null
  /** The one thing worth offering to do next. */
  action: Action | null
  /** Said beside installing, because it is the surprising part. */
  notice: string | null
  /** Why installing was refused a moment ago. */
  blocked: string | null
  /** What went wrong, when it is worth saying. */
  failure: string | null
  /** A check, download or install is under way, so nothing else is offered. */
  working: boolean
}

export function describe(snapshot: UpdateSnapshot, now = Date.now()): Description {
  const { state, error, blocked } = snapshot

  // A failed background check is not worth saying; every other failure is
  // something the person asked for.
  const failure = error && (error.manual || error.stage !== 'check') ? error.message : null

  const base: Description = {
    stage: state.phase,
    headline: '',
    detail: null,
    info: null,
    track: null,
    action: null,
    notice: null,
    blocked: null,
    failure,
    working: false,
  }

  switch (state.phase) {
    case 'idle':
      return {
        ...base,
        headline: 'Not checked yet',
        action: { kind: 'check', label: failure ? TRY_AGAIN : CHECK },
      }

    case 'checking':
      return { ...base, headline: 'Looking for updates…', working: true, failure: null }

    case 'up_to_date':
      return {
        ...base,
        headline: 'Scuttle is up to date',
        detail: `Checked ${checkedAgo(state.checked_unix, now)}.`,
        action: { kind: 'check', label: failure ? TRY_AGAIN : CHECK_AGAIN },
      }

    case 'available':
      return {
        ...base,
        headline: `${state.info.version} is available`,
        detail: NOT_YET_DOWNLOADED,
        info: state.info,
        action: { kind: 'download', label: failure ? TRY_AGAIN : DOWNLOAD },
      }

    case 'downloading': {
      const pct = percent(state.received, state.total)
      return {
        ...base,
        headline: `Downloading ${state.info.version}`,
        detail: `${sizeLine(state.received, state.total)}. ${KEEP_WORKING}`,
        info: state.info,
        track:
          pct === null
            ? { mode: 'indeterminate', fraction: 0 }
            : { mode: 'determinate', fraction: pct / 100 },
        working: true,
        failure: null,
      }
    }

    case 'ready':
      return {
        ...base,
        headline: `${state.info.version} is ready`,
        detail: 'Downloaded, and checked against Scuttle’s signature.',
        info: state.info,
        action: { kind: 'install', label: blocked ? TRY_AGAIN : INSTALL },
        notice: RESTART_NOTICE,
        blocked,
      }

    case 'installing':
      return {
        ...base,
        headline: `Installing ${state.info.version}`,
        info: state.info,
        notice: RESTARTING_NOTICE,
        working: true,
        failure: null,
      }

    case 'unavailable':
      return {
        ...base,
        headline: 'Updates are not available here',
        detail: state.reason,
        failure: null,
      }
  }
}

// ---- release notes --------------------------------------------------------

export interface NoteLine {
  kind: 'heading' | 'item' | 'text'
  text: string
}

/**
 * Release notes are a section of the changelog, which is Markdown. Scuttle does
 * not render Markdown; it reads just enough of it — headings, list items,
 * paragraphs — to show the notes as they were written, without a parser and
 * without ever putting release text into the page as HTML.
 */
export function parseNotes(notes: string | null): NoteLine[] {
  if (!notes) return []
  const lines: NoteLine[] = []
  for (const raw of notes.split(/\r?\n/)) {
    const line = raw.trim()
    if (line === '') continue
    const heading = /^#{1,6}\s+(.*)$/.exec(line)
    if (heading) {
      lines.push({ kind: 'heading', text: stripInline(heading[1] ?? '') })
      continue
    }
    const item = /^[-*+]\s+(.*)$/.exec(line)
    if (item) {
      lines.push({ kind: 'item', text: stripInline(item[1] ?? '') })
      continue
    }
    lines.push({ kind: 'text', text: stripInline(line) })
  }
  return lines
}

/** Backticks, emphasis and links are shown as their words. */
function stripInline(text: string): string {
  return text
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/[`*_]{1,3}([^`*_]+)[`*_]{1,3}/g, '$1')
}
