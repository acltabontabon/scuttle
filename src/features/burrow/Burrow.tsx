import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useCallback, useEffect, useRef, useState, type Ref } from 'react'

import { bytes } from '@/lib/format'
import { Scuttle } from '@/visuals/Scuttle'

import { burrowLine, lastLook, type BurrowStatus } from './phrasing'
import styles from './Burrow.module.css'

export type Action = 'open' | 'drawer' | 'rummage' | 'settings' | 'quit' | 'close'

/** Hand a choice to the core. Nothing here moves a file. */
function act(action: Action) {
  void invoke('burrow_act', { action }).catch(() => undefined)
}

/** How long the little peek lasts. Once per opening, never on a loop. */
const REACTION_MS = 700

/**
 * Scuttle's burrow: what the tray icon opens.
 *
 * It shows where things stand and offers the ways onward. Opening it starts
 * nothing — every button hands over to the main window or quits, and none of
 * them moves a file. Scuttle peeks up once when it opens (unless motion is
 * reduced or Scuttle is set to be quiet) and then holds still.
 */
export function Burrow() {
  const [status, setStatus] = useState<BurrowStatus | null>(null)
  const [line, setLine] = useState('')
  const [reacting, setReacting] = useState(false)
  const firstButton = useRef<HTMLButtonElement>(null)
  const timer = useRef<number | null>(null)

  const refresh = useCallback(async () => {
    try {
      const next = await invoke<BurrowStatus>('burrow_status')
      setStatus(next)
      setLine(burrowLine(next, Math.random()))
      applyLook(next)
      const still =
        next.personality === 'quiet' ||
        next.reduced_motion === true ||
        window.matchMedia('(prefers-reduced-motion: reduce)').matches
      if (!still) {
        setReacting(true)
        if (timer.current) window.clearTimeout(timer.current)
        timer.current = window.setTimeout(() => setReacting(false), REACTION_MS)
      }
    } catch {
      setStatus(null)
      setLine('Scuttle could not check just now. The main window has the details.')
    }
    firstButton.current?.focus()
  }, [])

  useEffect(() => {
    let off: (() => void) | undefined
    let stopped = false
    // Refresh from the event handler rather than the effect body: the burrow
    // is shown and hidden, not recreated, so opening is what asks.
    listen('scuttle://burrow-opened', () => void refresh()).then((unlisten) => {
      if (stopped) unlisten()
      else off = unlisten
    })
    const first = window.setTimeout(() => void refresh(), 0)
    return () => {
      stopped = true
      off?.()
      window.clearTimeout(first)
      if (timer.current) window.clearTimeout(timer.current)
    }
  }, [refresh])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') act('close')
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  return (
    <BurrowView status={status} line={line} reacting={reacting} onAct={act} firstButton={firstButton} />
  )
}

/**
 * What the burrow shows, given the facts. Kept apart from the part that talks
 * to the core so the design workbench can show every state of it.
 */
export function BurrowView({
  status,
  line,
  reacting,
  onAct,
  firstButton,
}: {
  status: BurrowStatus | null
  line: string
  reacting: boolean
  onAct: (action: Action) => void
  firstButton?: Ref<HTMLButtonElement>
}) {
  const busy = status?.busy ?? null

  return (
    <main className={styles.burrow} aria-label="Scuttle's burrow">
      <header className={styles.head}>
        <div className={styles.creature} data-reacting={reacting || undefined} aria-hidden="true">
          <Scuttle mood={reacting ? 'found' : 'idle'} size={64} className={reacting ? undefined : styles.holdStill} />
        </div>
        <p className={styles.line} role="status">
          {line}
        </p>
      </header>

      <dl className={styles.facts}>
        <dt>Last look</dt>
        <dd>{status ? lastLook(status) : '—'}</dd>
        {status?.has_looked && (
          <>
            <dt>Worth a look</dt>
            <dd>
              {status.found}
              {status.suggested > 0 && ` · ${status.suggested} suggested`}
            </dd>
          </>
        )}
        <dt>In the Drawer</dt>
        <dd>
          {status && status.drawer_items > 0
            ? `${status.drawer_items} · ${bytes(status.drawer_bytes)}`
            : 'Nothing'}
        </dd>
        {busy && (
          <>
            <dt>Right now</dt>
            <dd>{busy}</dd>
          </>
        )}
      </dl>

      <nav className={styles.actions}>
        <button ref={firstButton} className={styles.primary} onClick={() => onAct('open')}>
          Open Scuttle
        </button>
        <div className={styles.row}>
          <button onClick={() => onAct('drawer')}>View the Drawer</button>
          <button
            onClick={() => onAct('rummage')}
            disabled={busy !== null}
            title={busy ? `Scuttle is ${busy}.` : 'Opens Scuttle and starts looking. Nothing moves.'}
          >
            Rummage now
          </button>
        </div>
        <div className={styles.row}>
          <button onClick={() => onAct('settings')}>Settings</button>
          <button onClick={() => onAct('quit')}>Quit Scuttle</button>
        </div>
      </nav>
    </main>
  )
}

/** Match the main window's theme and motion settings. */
function applyLook(status: BurrowStatus) {
  const root = document.documentElement
  if (status.appearance === 'light' || status.appearance === 'dark') {
    root.setAttribute('data-theme', status.appearance)
  } else {
    root.removeAttribute('data-theme')
  }
  if (status.reduced_motion === true) root.setAttribute('data-motion', 'still')
  else root.removeAttribute('data-motion')
}
