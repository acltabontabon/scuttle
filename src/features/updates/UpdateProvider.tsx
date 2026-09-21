import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'

import { api, watchUpdates } from '@/lib/ipc'
import type { UpdateSnapshot } from '@/lib/types'
import type { ActionKind } from './view'

/**
 * The window's view of updating.
 *
 * It holds no update logic. The state machine, the channel policy and the
 * decision about whether an install may go ahead are all in Rust; this keeps
 * the latest snapshot the core sent, whether the little panel is open, and
 * turns a click into the matching command. Every command's answer is a
 * snapshot, and so is every event, so there is nothing to reconcile.
 *
 * Deliberately not part of the main store. That one is the state of the
 * findings and the Drawer; this is a different subject with a different
 * lifetime, and it would only make the larger file larger.
 */
interface Updates {
  snapshot: UpdateSnapshot | null
  panelOpen: boolean
  setPanelOpen: (open: boolean) => void
  /** Look now. Only a person asking calls this, so failure is worth showing. */
  check: () => Promise<void>
  download: () => Promise<void>
  install: () => Promise<void>
  /** "Later": stop showing this version until a newer one turns up. */
  dismiss: () => Promise<void>
  act: (kind: ActionKind) => Promise<void>
}

const UpdatesContext = createContext<Updates | null>(null)

export function useUpdates(): Updates {
  const value = useContext(UpdatesContext)
  if (!value) throw new Error('useUpdates needs an UpdateProvider')
  return value
}

/**
 * `live` is false only in the design workbench, which has no Rust behind it:
 * it shows the snapshot it is given and every action is a no-op.
 */
export function UpdateProvider({
  children,
  initial = null,
  live = true,
}: {
  children: ReactNode
  initial?: UpdateSnapshot | null
  live?: boolean
}) {
  const [snapshot, setSnapshot] = useState<UpdateSnapshot | null>(initial)
  const [panelOpen, setPanelOpen] = useState(false)

  // Events are broadcast and a hidden window misses them, so the current
  // snapshot is fetched on start and whenever the window comes back.
  useEffect(() => {
    if (!live) return
    let stopped = false
    let dispose: (() => void) | undefined

    const catchUp = () => {
      api
        .updateStatus()
        .then((next) => {
          if (!stopped) setSnapshot(next)
        })
        .catch(() => undefined)
    }

    void watchUpdates((next) => {
      if (!stopped) setSnapshot(next)
    })
      .then((off) => {
        if (stopped) off()
        else {
          dispose = off
          catchUp()
        }
      })
      .catch(() => undefined)

    const onVisible = () => {
      if (document.visibilityState === 'visible') catchUp()
    }
    document.addEventListener('visibilitychange', onVisible)

    return () => {
      stopped = true
      dispose?.()
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [live])

  // A command's answer is a snapshot. When one is refused — the install is
  // waiting on a move, say — the reason is already in the snapshot the core
  // published, so the rejection only needs to trigger a fresh look.
  const run = useCallback(
    async (command: () => Promise<UpdateSnapshot>) => {
      if (!live) return
      try {
        setSnapshot(await command())
      } catch {
        try {
          setSnapshot(await api.updateStatus())
        } catch {
          // The core is gone or going; there is nothing more to show.
        }
      }
    },
    [live],
  )

  const check = useCallback(() => run(() => api.checkForUpdate(true)), [run])
  const download = useCallback(() => run(api.downloadUpdate), [run])
  const install = useCallback(() => run(api.installUpdate), [run])
  const dismiss = useCallback(async () => {
    setPanelOpen(false)
    await run(api.dismissUpdate)
  }, [run])

  const act = useCallback(
    (kind: ActionKind) => {
      switch (kind) {
        case 'check':
          return check()
        case 'download':
          return download()
        case 'install':
          return install()
      }
    },
    [check, download, install],
  )

  const value = useMemo<Updates>(
    () => ({ snapshot, panelOpen, setPanelOpen, check, download, install, dismiss, act }),
    [snapshot, panelOpen, check, download, install, dismiss, act],
  )

  return <UpdatesContext.Provider value={value}>{children}</UpdatesContext.Provider>
}
