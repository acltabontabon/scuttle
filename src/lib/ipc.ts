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
  GroupOutcome,
  KeepChoice,
  Findings,
  HistoryEntry,
  Phase,
  Progress,
  QuarantineRecord,
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

  quarantine: (id: string) => invoke<QuarantineRecord>('quarantine', { id }),
  quarantineMember: (id: string, memberIndex: number) =>
    invoke<QuarantineRecord>('quarantine_member', { id, memberIndex }),
  quarantineGroup: (id: string, keep: KeepChoice) =>
    invoke<GroupOutcome>('quarantine_group', { id, keep }),
  quarantineList: () => invoke<QuarantineView>('quarantine_list'),
  restore: (id: string) => invoke<RestoreOutcome>('restore', { id }),
  removePermanently: (id: string) => invoke<void>('remove_permanently', { id }),

  keep: (id: string) => invoke<void>('keep', { id }),
  ignore: (id: string, scope: 'path' | 'app' | 'category') =>
    invoke<void>('ignore', { id, scope }),
  clearIgnores: () => invoke<void>('clear_ignores'),

  reveal: (id: string) => invoke<void>('reveal', { id }),
  revealQuarantined: (id: string) => invoke<void>('reveal_quarantined', { id }),

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
