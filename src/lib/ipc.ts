/**
 * The only place the frontend talks to Rust.
 *
 * Two rules:
 *  - Everything is addressed by id. There is no wrapper here that takes a
 *    filesystem path and asks the backend to do something with it.
 *  - Errors come back as `ScuttleError`, so callers can react to `stale` or
 *    `refused` specifically instead of matching on prose.
 */

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

import type {
  Candidate,
  DryRunReport,
  Findings,
  HistoryEntry,
  IgnoredEntry,
  MoveRequest,
  MoveSnapshot,
  Phase,
  Progress,
  PurgeOutcome,
  QuarantineView,
  RestoreOutcome,
  RootDescription,
  RummageStarted,
  ScanSummary,
  Settings,
  SpaceOverview,
} from './types'

export const EVENTS = {
  phase: 'scuttle://phase',
  progress: 'scuttle://progress',
  found: 'scuttle://found',
  done: 'scuttle://done',
  move: 'scuttle://move',
} as const

/** Scan events all carry the id of the scan they belong to. */
type WithScan<T> = T & { scan_id: string }

export const api = {
  rummage: (options: { roots?: string[]; includeDeveloperDebris?: boolean } = {}) =>
    invoke<RummageStarted>('rummage', {
      request: {
        roots: options.roots ?? null,
        include_developer_debris: options.includeDeveloperDebris ?? null,
      },
    }),

  cancelRummage: () => invoke<boolean>('cancel_rummage'),

  findings: () => invoke<Findings>('findings'),
  finding: (id: string) => invoke<Candidate>('finding', { id }),

  /**
   * Start moving findings into the drawer. Returns as soon as the work has
   * been handed to a worker; progress arrives on the move channel, and
   * `moveStatus` tells a listener that missed it where things stand.
   *
   * Findings are named by id, never by path. Which are eligible for a sweep is
   * the core's decision, so there is no request shape that can ask for a
   * risky one to be swept up with the safe ones; a hand-picked selection may
   * name any finding the person ticked, and the core's safety gate still
   * checks every one against the live filesystem.
   */
  startMove: (request: MoveRequest) => invoke<{ job_id: number }>('start_move', { request }),
  cancelMove: () => invoke<boolean>('cancel_move'),
  /** The running move's snapshot, or the last finished one's until dismissed. */
  moveStatus: () => invoke<MoveSnapshot | null>('move_status'),
  dismissMove: () => invoke<void>('dismiss_move'),
  /**
   * Take a fresh look at findings that have changed since they were scanned,
   * so they can be reviewed and chosen again. Moves nothing.
   */
  refreshFindings: (ids: string[]) => invoke<Candidate[]>('refresh_findings', { ids }),
  quarantineList: () => invoke<QuarantineView>('quarantine_list'),
  restore: (id: string) => invoke<RestoreOutcome>('restore', { id }),
  removePermanently: (id: string) => invoke<void>('remove_permanently', { id }),
  /**
   * Empty the drawer. Takes nothing: the drawer is the set. This is the only
   * call that gives disk space back — quarantining is a move.
   */
  emptyDrawer: () => invoke<PurgeOutcome>('empty_drawer'),

  keep: (id: string) => invoke<void>('keep', { id }),
  ignore: (id: string, scope: 'path' | 'app' | 'category') =>
    invoke<void>('ignore', { id, scope }),
  ignored: () => invoke<IgnoredEntry[]>('ignored'),
  stopIgnoring: (kind: IgnoredEntry['kind'], value: string) =>
    invoke<void>('stop_ignoring', { kind, value }),
  clearIgnores: () => invoke<void>('clear_ignores'),

  reveal: (id: string) => invoke<void>('reveal', { id }),
  revealQuarantined: (id: string) => invoke<void>('reveal_quarantined', { id }),
  revealQuarantineRoot: () => invoke<void>('reveal_quarantine_root'),

  space: () => invoke<SpaceOverview>('space'),
  settings: () => invoke<Settings>('settings'),
  saveSettings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
  suggestedRoots: () => invoke<RootDescription[]>('suggested_roots'),
  history: () => invoke<HistoryEntry[]>('history'),
  dryRun: (options: { includeDeveloperDebris?: boolean } = {}) =>
    invoke<DryRunReport>('dry_run', {
      request: {
        roots: null,
        include_developer_debris: options.includeDeveloperDebris ?? null,
      },
    }),
  about: () => invoke<{ version: string; platform: string; quarantine_root: string }>('about'),
}

/** Subscribe to a whole rummage. Returns a single unsubscribe function. */
export async function watchRummage(handlers: {
  onPhase?: (phase: Phase, scanId: string) => void
  onProgress?: (progress: Progress, scanId: string) => void
  onFound?: (candidate: Candidate, scanId: string) => void
  onDone?: (summary: ScanSummary & { error?: unknown }, scanId: string) => void
}): Promise<UnlistenFn> {
  const unlisteners: UnlistenFn[] = await Promise.all([
    listen<WithScan<Phase>>(EVENTS.phase, ({ payload }) => {
      const { scan_id, ...phase } = payload
      handlers.onPhase?.(phase as Phase, scan_id)
    }),
    listen<WithScan<Progress>>(EVENTS.progress, ({ payload }) => {
      const { scan_id, ...progress } = payload
      handlers.onProgress?.(progress as Progress, scan_id)
    }),
    listen<WithScan<Candidate>>(EVENTS.found, ({ payload }) => {
      const { scan_id, ...candidate } = payload
      handlers.onFound?.(candidate as Candidate, scan_id)
    }),
    listen<WithScan<ScanSummary>>(EVENTS.done, ({ payload }) => {
      const { scan_id, ...summary } = payload
      handlers.onDone?.(summary as ScanSummary, scan_id)
    }),
  ])

  return () => unlisteners.forEach((off) => off())
}

/**
 * Subscribe to move snapshots. Delivery may be delayed, repeated or reordered;
 * the receiver is expected to keep only what is newer than what it has (see
 * `features/move/progress.ts`).
 */
export async function watchMoves(onSnapshot: (snapshot: MoveSnapshot) => void): Promise<UnlistenFn> {
  return listen<MoveSnapshot>(EVENTS.move, ({ payload }) => onSnapshot(payload))
}
