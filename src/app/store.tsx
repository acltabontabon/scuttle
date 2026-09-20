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

import { describeReport } from '@/features/move/phrasing'
import { applySnapshot, isTerminal, seedJob } from '@/features/move/progress'
import { bytes } from '@/lib/format'
import { api, watchMoves, watchRummage } from '@/lib/ipc'
import {
  isScuttleError,
  type Candidate,
  type Category,
  type Findings,
  type KeepChoice,
  type MoveRequest,
  type MoveSnapshot,
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

  /**
   * The move into the drawer, if there is one to speak of. `snapshot` is the
   * running job or, once it has finished, its outcome — kept until dismissed,
   * so failures stay reachable after the toast is gone. `pending` covers the
   * moment between a click and the first word from the worker, so the
   * interface responds to the click itself.
   */
  move: { snapshot: MoveSnapshot | null; pending: boolean }
  /** True from the click until the outcome is in. Prevents a second start. */
  moving: boolean
  cancelMove: () => Promise<void>
  dismissMove: () => Promise<void>
  /** Try the unfinished findings of the last move again; never the moved ones. */
  retryMove: () => Promise<void>
  /** Take a fresh look at findings that changed, so they can be chosen again. */
  reviewAgain: (ids: string[]) => Promise<void>
  /** Whether the details of the last move are open. */
  moveDetailsOpen: boolean
  setMoveDetailsOpen: (open: boolean) => void

  /**
   * All of these start a move and return once it has *started*. What happens
   * next is reported through `move`.
   */
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
  const [moveSnapshot, setMoveSnapshot] = useState<MoveSnapshot | null>(null)
  const [movePending, setMovePending] = useState(false)
  const [moveDetailsOpen, setMoveDetailsOpen] = useState(false)

  const noteId = useRef(0)
  const noteTimer = useRef<number | null>(null)
  const activeScan = useRef<string | null>(null)
  // Mirrors of the move state for use inside event handlers, which must see
  // the latest value without being re-created on every progress event.
  const moveRef = useRef<MoveSnapshot | null>(null)
  const pendingRef = useRef(false)
  // The newest job whose outcome has been dismissed. Nothing at or below it is
  // taken back in: a straggling event from a job someone has already let go
  // of must not bring it back on screen.
  const dismissedJob = useRef(0)

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

  const restore = useCallback<Store['restore']>(
    async (id) => {
      try {
        const outcome = await api.restore(id)
        await Promise.all([refreshDrawer(), refreshFindings()])
        if (outcome.remaining > 0) {
          const all = outcome.restored + outcome.remaining
          say(
            `Put back ${outcome.restored} of ${all}. ${outcome.remaining} could not go back and ${outcome.remaining === 1 ? 'is' : 'are'} still in the drawer.`,
            { tone: 'warn' },
          )
          return false
        }
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

  /**
   * Take a snapshot from the worker — live, or fetched to catch up — and fold
   * it into what is known. Only a live transition into a finished state speaks
   * and refreshes: a snapshot fetched later about something that ended earlier
   * updates the picture and stays quiet.
   */
  const applyMove = useCallback(
    (incoming: MoveSnapshot, live: boolean) => {
      if (incoming.job_id <= dismissedJob.current) return
      const before = moveRef.current
      const next = applySnapshot(before, incoming)
      if (next === before || next === null) return
      moveRef.current = next
      setMoveSnapshot(next)
      pendingRef.current = false
      setMovePending(false)

      const justFinished =
        isTerminal(next.phase) &&
        !(before !== null && before.job_id === next.job_id && isTerminal(before.phase))
      if (!live || !justFinished) return

      void Promise.all([refreshFindings(), refreshDrawer()])
      if (next.report) {
        const outcome = describeReport(next.report)
        const only = next.report.findings.length === 1 ? next.report.findings[0]! : null
        const undoable =
          next.report.outcome === 'completed' && only?.record_id && only.unit === 'item'
            ? only.record_id
            : null
        say(outcome.headline, {
          tone: outcome.tone,
          action: undoable
            ? { label: 'Put it back', run: () => void restore(undoable) }
            : outcome.hasDetails
              ? { label: 'Details', run: () => setMoveDetailsOpen(true) }
              : undefined,
        })
        // A clean finish leaves nothing to look at, so nothing is kept. Anything
        // else stays in the header until it is dealt with or dismissed.
        if (next.report.outcome === 'completed' && !outcome.hasDetails) {
          dismissedJob.current = next.job_id
          moveRef.current = null
          setMoveSnapshot(null)
          void api.dismissMove().catch(() => undefined)
        }
      }
    },
    [refreshDrawer, refreshFindings, restore, say],
  )

  // One subscription for the app's lifetime, plus a catch-up on load and
  // whenever the window comes back: events are not replayed, so a listener
  // that was not there for them asks where things stand instead.
  useEffect(() => {
    let dispose: (() => void) | undefined
    let stopped = false
    const catchUp = () => {
      void api
        .moveStatus()
        .then((snapshot) => {
          if (snapshot && !stopped) applyMove(snapshot, false)
        })
        .catch(() => undefined)
    }

    watchMoves((snapshot) => applyMove(snapshot, true)).then((off) => {
      if (stopped) off()
      else {
        dispose = off
        catchUp()
      }
    })
    const onVisible = () => {
      if (document.visibilityState === 'visible') catchUp()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      stopped = true
      dispose?.()
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [applyMove])

  /**
   * Start a move. The interface answers the click at once — `pending` flips
   * before anything is sent — and a second start while one is running is
   * refused here as well as in the core.
   */
  const startMove = useCallback(
    async (request: MoveRequest): Promise<boolean> => {
      const running = moveRef.current !== null && !isTerminal(moveRef.current.phase)
      if (running || pendingRef.current) return false
      pendingRef.current = true
      setMovePending(true)
      setDetail(null)
      setMoveDetailsOpen(false)
      try {
        const { job_id } = await api.startMove(request)
        // Events may already have arrived, even the last one; only fill in
        // what is still missing.
        const seeded = job_id > dismissedJob.current ? seedJob(moveRef.current, job_id) : moveRef.current
        if (seeded !== moveRef.current) {
          moveRef.current = seeded
          setMoveSnapshot(seeded)
        }
        return true
      } catch (error) {
        pendingRef.current = false
        setMovePending(false)
        say(readError(error), { tone: 'warn' })
        if (isScuttleError(error) && error.code === 'busy') {
          void api
            .moveStatus()
            .then((snapshot) => snapshot && applyMove(snapshot, false))
            .catch(() => undefined)
        }
        return false
      }
    },
    [applyMove, say],
  )

  const cancelMove = useCallback(async () => {
    try {
      await api.cancelMove()
    } catch (error) {
      say(readError(error), { tone: 'warn' })
    }
  }, [say])

  const dismissMove = useCallback(async () => {
    const current = moveRef.current
    if (current === null || !isTerminal(current.phase)) return
    dismissedJob.current = current.job_id
    moveRef.current = null
    setMoveSnapshot(null)
    setMoveDetailsOpen(false)
    try {
      await api.dismissMove()
    } catch {
      /* the interface has already let go; the core forgets on the next job */
    }
  }, [])

  const retryMove = useCallback(async () => {
    const report = moveRef.current?.report
    if (!report) return
    const ids = describeReport(report).retryIds
    if (ids.length === 0) return
    setMoveDetailsOpen(false)
    await startMove({ kind: 'selection', ids, retry: true })
  }, [startMove])

  const reviewAgain = useCallback(
    async (ids: string[]) => {
      if (ids.length === 0) return
      try {
        await api.refreshFindings(ids)
        await refreshFindings()
        say('Had another look. They are up to date, and yours to choose again.')
      } catch (error) {
        say(readError(error), { tone: 'warn' })
      }
    },
    [refreshFindings, say],
  )

  const quarantine = useCallback<Store['quarantine']>(
    async (candidate, memberIndex) => {
      await startMove(
        memberIndex === undefined
          ? { kind: 'selection', ids: [candidate.id] }
          : { kind: 'member', id: candidate.id, member_index: memberIndex },
      )
    },
    [startMove],
  )

  const quarantineGroup = useCallback<Store['quarantineGroup']>(
    async (candidate, keepWhich) => {
      await startMove({ kind: 'group', id: candidate.id, keep: keepWhich })
    },
    [startMove],
  )

  const quarantineConfident = useCallback<Store['quarantineConfident']>(
    async (category) => {
      await startMove({ kind: 'confident', category })
    },
    [startMove],
  )

  const quarantineAllConfident = useCallback<Store['quarantineAllConfident']>(async () => {
    await startMove({ kind: 'confident' })
  }, [startMove])

  const quarantineMany = useCallback<Store['quarantineMany']>(
    async (ids) => {
      if (ids.length === 0) return
      await startMove({ kind: 'selection', ids })
    },
    [startMove],
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

  const move = useMemo(
    () => ({ snapshot: moveSnapshot, pending: movePending }),
    [moveSnapshot, movePending],
  )
  const moving =
    movePending || (moveSnapshot !== null && !isTerminal(moveSnapshot.phase))

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
      move,
      moving,
      cancelMove,
      dismissMove,
      retryMove,
      reviewAgain,
      moveDetailsOpen,
      setMoveDetailsOpen,
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
      dismissNote, move, moving, cancelMove, dismissMove, retryMove, reviewAgain,
      moveDetailsOpen, quarantine, quarantineGroup, quarantineConfident,
      quarantineAllConfident, quarantineMany, emptyDrawer, keep, ignore, restore,
      removePermanently, reveal,
    ],
  )

  return <StoreContext.Provider value={value}>{children}</StoreContext.Provider>
}

export function useStore(): Store {
  const store = useContext(StoreContext)
  if (!store) throw new Error('useStore was called outside the provider')
  return store
}
