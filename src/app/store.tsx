import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'

import { bytes } from '@/lib/format'
import { api, watchRummage } from '@/lib/ipc'
import {
  isScuttleError,
  type Candidate,
  type Category,
  type Findings,
  type KeepChoice,
  type Phase,
  type Progress,
  type QuarantineView,
  type RootDescription,
  type ScanSummary,
  type Settings,
  type SpaceOverview,
} from '@/lib/types'

/**
 * Application state.
 *
 * Deliberately a single context over `useState` rather than a state library:
 * the Rust core is the source of truth for everything that matters, and what
 * lives here is which view is open, what the running scan has said so far,
 * and a cache of the last answer from each command. Reaching for Redux to
 * hold six values would be its own kind of mess.
 */

export type View =
  | { name: 'home' }
  | { name: 'findings' }
  /**
   * `preselect` opens the pile with Scuttle's suggested items already ticked.
   * It is how "Review suggestion" on the floor stays a *review*: the same
   * pile, the same list, the same explicit button to move anything — just
   * with the choosing already done. Turning that link into a file operation
   * would be a different promise entirely.
   */
  | { name: 'pile'; category: Category; preselect?: 'suggested' }
  | { name: 'drawer' }
  | { name: 'space' }
  | { name: 'settings' }

export type ScanStatus = 'idle' | 'running' | 'done' | 'cancelled' | 'failed'

export interface ScanState {
  status: ScanStatus
  scanId: string | null
  phase: Phase
  progress: Progress
  roots: RootDescription[]
  /** The most recent thing to turn up. */
  latest: Candidate | null
  /** How many have turned up so far. */
  foundCount: number
  /** The largest single finding so far. */
  heaviestBytes: number
  summary: ScanSummary | null
  error: string | null
}

const EMPTY_SCAN: ScanState = {
  status: 'idle',
  scanId: null,
  phase: { phase: 'preparing' },
  progress: { files_seen: 0, bytes_seen: 0, candidates_found: 0, current_area: '' },
  roots: [],
  latest: null,
  foundCount: 0,
  heaviestBytes: 0,
  summary: null,
  error: null,
}

/** A short-lived line of feedback. Never a notification, never persistent. */
export interface Note {
  id: number
  text: string
  tone: 'plain' | 'warn'
  /** Optional single action, e.g. undoing a quarantine. */
  action?: { label: string; run: () => void }
}

export interface Store {
  view: View
  go: (view: View) => void

  scan: ScanState
  rummage: () => Promise<void>
  cancel: () => Promise<void>

  findings: Findings | null
  refreshFindings: () => Promise<boolean>

  detail: Candidate | null
  openDetail: (candidate: Candidate | null) => void

  drawer: QuarantineView | null
  refreshDrawer: () => Promise<boolean>

  space: SpaceOverview | null
  refreshSpace: () => Promise<boolean>

  settings: Settings | null
  /** Reports whether the write actually landed. */
  updateSettings: (next: Settings) => Promise<boolean>

  note: Note | null
  say: (text: string, options?: { tone?: Note['tone']; action?: Note['action'] }) => void
  dismissNote: () => void

  quarantine: (candidate: Candidate, memberIndex?: number) => Promise<void>
  quarantineGroup: (candidate: Candidate, keep: KeepChoice) => Promise<void>
  quarantineConfident: (category: Category) => Promise<void>
  /** Sweep every pile at once, from the findings floor. */
  quarantineAllConfident: () => Promise<void>
  /** Move a selection the user ticked by hand. */
  quarantineMany: (ids: string[]) => Promise<void>
  /** Permanently remove everything in the drawer. The only bulk deletion. */
  emptyDrawer: () => Promise<void>
  keep: (candidate: Candidate) => Promise<void>
  ignore: (candidate: Candidate, scope: 'path' | 'app' | 'category') => Promise<void>
  /**
   * Both report whether the operation actually completed, so a caller can
   * keep a control disabled until it has, and say what failed where it
   * failed. Neither resolves before the file has moved.
   */
  restore: (id: string) => Promise<boolean>
  removePermanently: (id: string) => Promise<boolean>
  reveal: (candidate: Candidate) => Promise<void>
}

/**
 * Exported so the dev-only design workbench (`src/dev`) can supply fixture
 * state. Nothing in the shipping application uses it directly.
 */
export const StoreContext = createContext<Store | null>(null)

/**
 * What to say when things land in the drawer.
 *
 * States the consequence at the moment it happens rather than leaving it for
 * whenever somebody next opens the drawer: nothing has been deleted, and the
 * bytes are not back yet.
 */
function heldNote(count: number, movedBytes: number): string {
  const what = `${count} ${count === 1 ? 'thing' : 'things'}`
  return `${what} moved to the drawer, ${bytes(movedBytes)}. Nothing deleted — empty the drawer to free it.`
}

/** Turn whatever came back from IPC into something worth showing a person. */
function readError(error: unknown): string {
  if (isScuttleError(error)) return error.message
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  return 'Something went wrong, and Scuttle is not sure what.'
}

export function StoreProvider({ children }: { children: ReactNode }) {
  const [view, setView] = useState<View>({ name: 'home' })
  const [scan, setScan] = useState<ScanState>(EMPTY_SCAN)
  const [findings, setFindings] = useState<Findings | null>(null)
  const [detail, setDetail] = useState<Candidate | null>(null)
  const [drawer, setDrawer] = useState<QuarantineView | null>(null)
  const [space, setSpace] = useState<SpaceOverview | null>(null)
  const [settings, setSettings] = useState<Settings | null>(null)
  const [note, setNote] = useState<Note | null>(null)

  const noteId = useRef(0)
  const noteTimer = useRef<number | null>(null)
  const activeScan = useRef<string | null>(null)

  const say = useCallback<Store['say']>((text, options) => {
    noteId.current += 1
    setNote({ id: noteId.current, text, tone: options?.tone ?? 'plain', action: options?.action })
    if (noteTimer.current) window.clearTimeout(noteTimer.current)
    // An action needs longer to reach for than a statement does.
    noteTimer.current = window.setTimeout(() => setNote(null), options?.action ? 9000 : 4600)
  }, [])

  const dismissNote = useCallback(() => setNote(null), [])

  const refreshFindings = useCallback(async () => {
    try {
      setFindings(await api.findings())
      return true
    } catch (error) {
      say(readError(error), { tone: 'warn' })
      return false
    }
  }, [say])

  const refreshDrawer = useCallback(async () => {
    try {
      setDrawer(await api.quarantineList())
      return true
    } catch (error) {
      say(readError(error), { tone: 'warn' })
      return false
    }
  }, [say])

  const refreshSpace = useCallback(async () => {
    try {
      setSpace(await api.space())
      return true
    } catch (error) {
      say(readError(error), { tone: 'warn' })
      return false
    }
  }, [say])

  // One subscription for the whole app lifetime. Events carry a scan id and
  // anything from a superseded scan is dropped.
  useEffect(() => {
    let dispose: (() => void) | undefined
    let cancelled = false

    watchRummage({
      onPhase: (phase, scanId) => {
        if (activeScan.current !== scanId) return
        setScan((previous) => ({ ...previous, phase }))
      },
      onProgress: (progress, scanId) => {
        if (activeScan.current !== scanId) return
        setScan((previous) => ({ ...previous, progress }))
      },
      onFound: (candidate, scanId) => {
        if (activeScan.current !== scanId) return
        setScan((previous) => ({
          ...previous,
          latest: candidate,
          foundCount: previous.foundCount + 1,
          heaviestBytes: Math.max(previous.heaviestBytes, candidate.size),
        }))
      },
      onDone: (summary, scanId) => {
        if (activeScan.current !== scanId) return
        activeScan.current = null
        const failed = 'error' in summary && summary.error !== undefined
        setScan((previous) => ({
          ...previous,
          status: failed ? 'failed' : summary.cancelled ? 'cancelled' : 'done',
          summary: failed ? null : summary,
          error: failed ? readError((summary as { error: unknown }).error) : null,
        }))
        void refreshFindings()
      },
    }).then((off) => {
      if (cancelled) off()
      else dispose = off
    })

    return () => {
      cancelled = true
      dispose?.()
    }
  }, [refreshFindings])

  // Initial load.
  useEffect(() => {
    void (async () => {
      try {
        setSettings(await api.settings())
      } catch (error) {
        say(readError(error), { tone: 'warn' })
      }
      await refreshFindings()
    })()
  }, [refreshFindings, say])

  const rummage = useCallback(async () => {
    setScan({ ...EMPTY_SCAN, status: 'running' })
    try {
      const started = await api.rummage()
      activeScan.current = started.scan_id
      setScan((previous) => ({
        ...previous,
        scanId: started.scan_id,
        roots: started.roots,
      }))
    } catch (error) {
      activeScan.current = null
      setScan({ ...EMPTY_SCAN, status: 'failed', error: readError(error) })
    }
  }, [])

  const cancel = useCallback(async () => {
    await api.cancelRummage()
  }, [])

  const updateSettings = useCallback<Store['updateSettings']>(
    async (next) => {
      try {
        setSettings(await api.saveSettings(next))
        return true
      } catch (error) {
        say(readError(error), { tone: 'warn' })
        return false
      }
    },
    [say],
  )

  const quarantine = useCallback<Store['quarantine']>(
    async (candidate, memberIndex) => {
      try {
        const record =
          memberIndex === undefined
            ? await api.quarantine(candidate.id)
            : await api.quarantineMember(candidate.id, memberIndex)
        setDetail(null)
        await Promise.all([refreshFindings(), refreshDrawer()])
        say(`${record.display_name} is in the drawer.`, {
          action: {
            label: 'Put it back',
            run: () => {
              void (async () => {
                try {
                  await api.restore(record.id)
                  await Promise.all([refreshFindings(), refreshDrawer()])
                  say(`${record.display_name} is back where it was.`)
                } catch (error) {
                  say(readError(error), { tone: 'warn' })
                }
              })()
            },
          },
        })
      } catch (error) {
        // `stale` and `refused` are the interesting ones: they mean the safety
        // layer did its job, and the user deserves the real reason.
        say(readError(error), { tone: 'warn' })
        if (isScuttleError(error) && error.code === 'stale') {
          await refreshFindings()
          setDetail(null)
        }
      }
    },
    [refreshDrawer, refreshFindings, say],
  )

  const quarantineGroup = useCallback<Store['quarantineGroup']>(
    async (candidate, keepWhich) => {
      try {
        const outcome = await api.quarantineGroup(candidate.id, keepWhich)
        setDetail(null)
        await Promise.all([refreshFindings(), refreshDrawer()])

        // Partial success is the normal case, so the report has to be able to
        // describe one rather than pretending everything worked.
        const moved = `${outcome.held.length} ${outcome.held.length === 1 ? 'copy' : 'copies'} in the drawer, ${outcome.kept} kept.`
        if (outcome.refused.length > 0) {
          say(`${moved} ${outcome.refused.length} left alone — ${outcome.refused[0]!.reason}`, {
            tone: 'warn',
          })
        } else {
          say(moved)
        }
      } catch (error) {
        say(readError(error), { tone: 'warn' })
      }
    },
    [refreshDrawer, refreshFindings, say],
  )

  const quarantineConfident = useCallback<Store['quarantineConfident']>(
    async (category) => {
      try {
        const outcome = await api.quarantineConfident(category)
        if (outcome.held.length === 0 && outcome.refused.length === 0) {
          say('Nothing in that pile was confident enough to move.')
          return
        }
        setDetail(null)
        await Promise.all([refreshFindings(), refreshDrawer()])

        const moved = heldNote(outcome.held.length, outcome.bytes)
        if (outcome.refused.length > 0) {
          say(`${moved} ${outcome.refused.length} left alone — ${outcome.refused[0]!.reason}`, {
            tone: 'warn',
          })
        } else {
          say(moved)
        }
      } catch (error) {
        say(readError(error), { tone: 'warn' })
      }
    },
    [refreshDrawer, refreshFindings, say],
  )

  const quarantineAllConfident = useCallback<Store['quarantineAllConfident']>(async () => {
    try {
      const outcome = await api.quarantineAllConfident()
      if (outcome.held.length === 0 && outcome.refused.length === 0) {
        say('Nothing on the floor was confident enough to move on its own.')
        return
      }
      setDetail(null)
      await Promise.all([refreshFindings(), refreshDrawer()])
      const moved = heldNote(outcome.held.length, outcome.bytes)
      if (outcome.refused.length > 0) {
        say(`${moved} ${outcome.refused.length} left alone.`, { tone: 'warn' })
      } else {
        say(moved)
      }
    } catch (error) {
      say(readError(error), { tone: 'warn' })
    }
  }, [refreshDrawer, refreshFindings, say])

  const quarantineMany = useCallback<Store['quarantineMany']>(
    async (ids) => {
      if (ids.length === 0) return
      try {
        const outcome = await api.quarantineMany(ids)
        setDetail(null)
        await Promise.all([refreshFindings(), refreshDrawer()])
        const moved = heldNote(outcome.held.length, outcome.bytes)
        if (outcome.refused.length > 0) {
          say(`${moved} ${outcome.refused.length} would not go — ${outcome.refused[0]!.reason}`, {
            tone: 'warn',
          })
        } else {
          say(moved)
        }
      } catch (error) {
        say(readError(error), { tone: 'warn' })
      }
    },
    [refreshDrawer, refreshFindings, say],
  )

  const emptyDrawer = useCallback<Store['emptyDrawer']>(async () => {
    try {
      const outcome = await api.emptyDrawer()
      await Promise.all([refreshDrawer(), refreshSpace()])
      if (outcome.failed.length > 0) {
        say(
          `${bytes(outcome.bytes)} freed. ${outcome.failed.length} would not go — ${outcome.failed[0]!.reason}`,
          { tone: 'warn' },
        )
      } else {
        say(`${bytes(outcome.bytes)} freed.`)
      }
    } catch (error) {
      say(readError(error), { tone: 'warn' })
    }
  }, [refreshDrawer, refreshSpace, say])

  const keep = useCallback<Store['keep']>(
    async (candidate) => {
      try {
        await api.keep(candidate.id)
        setDetail(null)
        await refreshFindings()
        say(`Keeping ${candidate.display_name}.`)
      } catch (error) {
        say(readError(error), { tone: 'warn' })
      }
    },
    [refreshFindings, say],
  )

  const ignore = useCallback<Store['ignore']>(
    async (candidate, scope) => {
      try {
        await api.ignore(candidate.id, scope)
        setDetail(null)
        await refreshFindings()
        const what =
          scope === 'path'
            ? candidate.display_name
            : scope === 'app'
              ? (candidate.associated_app ?? candidate.display_name)
              : 'that whole pile'
        say(`Scuttle will not bring up ${what} again.`)
      } catch (error) {
        say(readError(error), { tone: 'warn' })
      }
    },
    [refreshFindings, say],
  )

  const restore = useCallback<Store['restore']>(
    async (id) => {
      try {
        const outcome = await api.restore(id)
        await Promise.all([refreshDrawer(), refreshFindings()])
        say(
          outcome.renamed
            ? 'Something was already there, so it went back under a new name.'
            : 'Back where it came from.',
        )
        return true
      } catch (error) {
        say(readError(error), { tone: 'warn' })
        return false
      }
    },
    [refreshDrawer, refreshFindings, say],
  )

  const removePermanently = useCallback<Store['removePermanently']>(
    async (id) => {
      try {
        await api.removePermanently(id)
        await refreshDrawer()
        say('Gone for good.')
        return true
      } catch (error) {
        say(readError(error), { tone: 'warn' })
        return false
      }
    },
    [refreshDrawer, say],
  )

  const reveal = useCallback<Store['reveal']>(
    async (candidate) => {
      try {
        await api.reveal(candidate.id)
      } catch (error) {
        say(readError(error), { tone: 'warn' })
      }
    },
    [say],
  )

  const go = useCallback<Store['go']>((next) => {
    setDetail(null)
    setView(next)
  }, [])

  const value = useMemo<Store>(
    () => ({
      view,
      go,
      scan,
      rummage,
      cancel,
      findings,
      refreshFindings,
      detail,
      openDetail: setDetail,
      drawer,
      refreshDrawer,
      space,
      refreshSpace,
      settings,
      updateSettings,
      note,
      say,
      dismissNote,
      quarantine,
      quarantineGroup,
      quarantineConfident,
      quarantineAllConfident,
      quarantineMany,
      emptyDrawer,
      keep,
      ignore,
      restore,
      removePermanently,
      reveal,
    }),
    [
      view, go, scan, rummage, cancel, findings, refreshFindings, detail, drawer,
      refreshDrawer, space, refreshSpace, settings, updateSettings, note, say,
      dismissNote, quarantine, quarantineGroup, quarantineConfident, quarantineAllConfident,
      quarantineMany, emptyDrawer, keep, ignore, restore, removePermanently, reveal,
    ],
  )

  return <StoreContext.Provider value={value}>{children}</StoreContext.Provider>
}

export function useStore(): Store {
  const store = useContext(StoreContext)
  if (!store) throw new Error('useStore was called outside the provider')
  return store
}
