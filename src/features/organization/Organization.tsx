import { useEffect, useRef, useState } from 'react'
import { useStore } from '@/app/store'
import { api } from '@/lib/ipc'
import { bytes } from '@/lib/format'
import type { Opportunity, OrganizationBatch, OrganizationInventory, OrganizationKind, OrganizationPlan, OrganizationPreference } from './types'
import styles from './Organization.module.css'

export function organizationError(error: unknown): string {
  return typeof error === 'object' && error !== null && 'message' in error ? String(error.message) : String(error)
}
const noun = (kind: OrganizationKind, count: number) => count === 1 ? kind.slice(0, -1) : kind
const name = (path: string) => path.split(/[/\\]/).pop() ?? path

/** Always reachable, including when the cleanup scan has no findings. */
export function OrganizationOffers({ kind }: { kind?: OrganizationKind }) {
  const { go, findings } = useStore()
  const [inventory, setInventory] = useState<OrganizationInventory | null>(null)
  const [error, setError] = useState(false)
  useEffect(() => {
    let live = true
    void api.organizationInventory().then((value) => { if (live) { setInventory(value); setError(false) } }, () => { if (live) setError(true) })
    return () => { live = false }
  }, [findings])
  return <section className={styles.offers} aria-label="Organization">
    {(['screenshots', 'installers'] as const).filter((k) => !kind || kind === k).map((k) => {
      const count = inventory?.items.filter((i) => i.kind === k).length ?? 0
      return count > 0 && <button key={k} className={styles.offer} onClick={() => go({ name: 'organize', kind: k })}>
        <span>{k === 'screenshots' ? `Gather ${count} ${count === 1 ? 'screenshot' : 'screenshots'}` : `Give ${count === 1 ? 'this installer' : `these ${count} installers`} a home`}</span>
        <strong>Organize…</strong>
      </button>
    })}
    {error && <span className={styles.muted}>Organization opportunities could not be loaded.</span>}
    {inventory && (inventory.partial || inventory.skipped_locations > 0) && <span className={styles.muted}>Some loose-file locations could not be fully checked.</span>}
    <button className={styles.link} onClick={() => go({ name: 'organization_history' })}>Organization history</button>
  </section>
}

function Thumbnail({ opportunity }: { opportunity: Opportunity }) {
  const [src, setSrc] = useState<string | null>(null)
  const host = useRef<HTMLSpanElement>(null)
  useEffect(() => {
    if (opportunity.kind !== 'screenshots') return
    let live = true
    const load = () => { void api.organizationThumbnail(opportunity.id).then((image) => { if (live) setSrc(image) }, () => {}) }
    // Decode only previews that are actually visible; never send source images
    // or arbitrary filesystem paths to the webview.
    let observer: IntersectionObserver | undefined
    if (typeof IntersectionObserver === 'undefined') load()
    else {
      observer = new IntersectionObserver((entries) => {
        if (entries.some((e) => e.isIntersecting)) { observer?.disconnect(); load() }
      }, { rootMargin: '100px' })
      if (host.current) observer.observe(host.current)
    }
    return () => { live = false; observer?.disconnect() }
  }, [opportunity.id, opportunity.kind])
  return <span className={styles.thumbnail} ref={host}>{src ? <img src={src} alt={`Preview of ${name(opportunity.path)}`} /> : <span aria-hidden="true">{opportunity.kind === 'screenshots' ? '▧' : '↓'}</span>}</span>
}

export function Organization({ kind }: { kind: OrganizationKind }) {
  const { go, refreshFindings } = useStore()
  const [inventory, setInventory] = useState<OrganizationInventory | null>(null)
  const [preference, setPreference] = useState<OrganizationPreference | null>(null)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [plan, setPlan] = useState<OrganizationPlan | null>(null)
  const [batch, setBatch] = useState<OrganizationBatch | null>(null)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const generation = useRef(0)
  const mounted = useRef(true)
  const title = useRef<HTMLHeadingElement>(null)
  useEffect(() => {
    mounted.current = true
    return () => { mounted.current = false; generation.current += 1 }
  }, [])
  useEffect(() => {
    let live = true
    void Promise.all([
      api.organizationInventory().then((inventory) => {
        if (live) { setInventory(inventory); setPicked(new Set(inventory.items.filter((o) => o.kind === kind).slice(0, 200).map((o) => o.id))) }
      }),
      api.organizationPreference(kind).then((preference) => { if (live) setPreference(preference) }),
    ]).catch((e) => { if (live) setError(organizationError(e)) })
    title.current?.focus()
    return () => { live = false }
  }, [kind])
  const items = inventory?.items.filter((o) => o.kind === kind).slice(0, 200) ?? []
  const invalidate = () => { generation.current += 1; setPlan(null) }
  const review = async () => {
    if (!preference) return
    const revision = ++generation.current
    setPending(true); setError(null); setPlan(null)
    try {
      const result = await api.planOrganization(kind, [...picked], preference.destination.id, preference.grouping)
      if (mounted.current && generation.current === revision) { setPlan(result); title.current?.focus() }
    } catch (e) { if (mounted.current && generation.current === revision) setError(organizationError(e)) }
    finally { if (mounted.current) setPending(false) }
  }
  const choose = async () => {
    invalidate(); setPending(true); setError(null)
    try {
      const destination = await api.chooseOrganizationDestination()
      if (mounted.current && destination) setPreference((p) => p ? { ...p, destination } : null)
    } catch (e) { if (mounted.current) setError(organizationError(e)) }
    finally { if (mounted.current) setPending(false) }
  }
  const start = async () => {
    if (!plan || pending) return
    setPending(true); setError(null)
    try {
      const result = await api.startOrganization(plan.id)
      if (mounted.current) { setBatch(result); setPlan(null); title.current?.focus() }
    } catch (e) { if (mounted.current) { setError(organizationError(e)); setPlan(null) } }
    finally { if (mounted.current) setPending(false) }
  }
  if (batch) return <div className={styles.page}>
    <button className={styles.link} onClick={() => go({ name: 'findings' })}>← All findings</button>
    {error && <p role="alert">{error}</p>}
    <BatchResult initial={batch} focusOnMount onFinished={() => { void refreshFindings() }} onReviewRemaining={() => {
      setPending(true)
      void api.organizationInventory().then((next) => {
        setInventory(next); setPicked(new Set(next.items.filter((o) => o.kind === kind).slice(0, 200).map((o) => o.id)))
        setBatch(null); setError(null)
      }, (e) => setError(organizationError(e))).finally(() => setPending(false))
    }} />
    <button className={styles.link} onClick={() => go({ name: 'organization_history' })}>Organization history</button>
  </div>
  const ready = plan?.items.filter((i) => !i.note).length ?? 0
  return <div className={styles.page}>
    <header className={styles.header}>
      <button className={styles.link} onClick={() => go({ name: 'findings' })}>← All findings</button>
      <p className={styles.eyebrow}>A place for everything</p>
      <h1 tabIndex={-1} ref={title}>{plan ? 'Before anything moves' : `Give these ${kind} a home`}</h1>
      <p className={styles.muted}>Loose files from Desktop, Downloads and Documents. You choose what moves.</p>
    </header>
    {error && <p role="alert" className={styles.notice}>{error}</p>}
    {!inventory && !error && <p role="status">Looking at the last rummage…</p>}
    {inventory && !preference && error && <button className={styles.link} onClick={() => { void api.chooseOrganizationDestination().then((destination) => { if (destination) { setPreference({ destination, grouping: kind === 'screenshots' ? 'month' : 'together' }); setError(null); setPicked(new Set(items.map((o) => o.id))) } }, (e) => setError(organizationError(e))) }}>Choose an available folder</button>}
    {preference && <div className={styles.destination}>
      <div><span className={styles.eyebrow}>Destination</span><p className={styles.path}>{preference.destination.path}</p></div>
      <button className={styles.link} disabled={pending} onClick={() => void choose()}>Change folder</button>
      {kind === 'screenshots' && <label>Group by{' '}<select disabled={pending} value={preference.grouping} onChange={(e) => { invalidate(); setPreference({ ...preference, grouping: e.target.value as 'month' | 'together' }) }}>
        <option value="month">Modification month</option><option value="together">Together in one folder</option>
      </select></label>}
    </div>}
    {inventory && (inventory.partial || inventory.skipped_locations > 0) && <p className={styles.notice}>This is a partial look. Some locations or files were not checked.</p>}
    {(inventory?.items.filter((o) => o.kind === kind).length ?? 0) > 200 && <p className={styles.muted}>Showing the first 200 files. Handle these to see the next ones.</p>}
    {items.length > 0 && <label className={styles.selection}><input type="checkbox" disabled={pending} checked={items.every((o) => picked.has(o.id))} onChange={(e) => { invalidate(); setPicked(new Set(e.target.checked ? items.map((o) => o.id) : [])) }} /> Select these {items.length} files</label>}
    <ul className={styles.files}>
      {items.map((o) => {
        const reviewed = plan?.items.find((i) => i.opportunity.id === o.id)
        return <li key={o.id} className={styles.file}>
          <input type="checkbox" aria-label={`Organize ${name(o.path)}`} disabled={pending} checked={picked.has(o.id)} onChange={(e) => { invalidate(); setPicked((current) => { const next = new Set(current); if (e.target.checked) next.add(o.id); else next.delete(o.id); return next }) }} />
          <Thumbnail opportunity={o} />
          <div className={styles.fileText}><strong>{name(o.path)}</strong><p className={styles.path}>{o.path}</p>
            {reviewed && <p className={styles.target}>→ {reviewed.destination}</p>}
            {reviewed?.note && <p className={styles.notice}>{reviewed.note}</p>}
          </div><span className={styles.muted}>{bytes(o.identity.size)}</span>
        </li>
      })}
    </ul>
    {inventory && items.length === 0 && <p>No loose {kind} to organize. Rummage again after adding files.</p>}
    <footer className={styles.footer}>
      <div><p>{plan ? `${ready} files ready to move.` : `${picked.size} files selected.`}</p><p className={styles.muted}>Files stay on disk. Completed moves can be undone from Organization history.</p>
        {plan?.items.some((i) => i.cross_volume && !i.note) && <p className={styles.notice}>Across drives, contents are verified before originals are removed. Extended attributes, alternate data streams and custom access rules are not preserved.</p>}
      </div>
      <button className={styles.link} disabled={pending} onClick={() => plan ? invalidate() : go({ name: 'findings' })}>{plan ? 'Back to selection' : 'Cancel'}</button>
      <button className={styles.primary} disabled={pending || !preference || (plan ? ready === 0 : picked.size === 0)} onClick={() => void (plan ? start() : review())}>{pending ? 'Checking…' : plan ? `Organize ${ready} ${noun(kind, ready)}` : 'Review organization'}</button>
    </footer>
  </div>
}

export function BatchResult({ initial, onFinished, onReviewRemaining, focusOnMount = false }: { initial: OrganizationBatch; onFinished?: () => void; onReviewRemaining?: () => void; focusOnMount?: boolean }) {
  const [batch, setBatch] = useState(initial)
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState(false)
  const finished = useRef(onFinished)
  const title = useRef<HTMLHeadingElement>(null)
  const wasRunning = useRef(initial.running)
  useEffect(() => { finished.current = onFinished }, [onFinished])
  useEffect(() => { if (focusOnMount) title.current?.focus() }, [focusOnMount])
  useEffect(() => {
    if (!batch.running) return
    let live = true
    let timer: ReturnType<typeof setTimeout>
    const poll = async () => {
      try {
        const next = await api.organizationStatus(batch.id)
        if (live) {
          setBatch((current) => next.revision > current.revision ? next : current)
          setError(null)
          if (!next.running) return
        }
      } catch (e) { if (live) setError(organizationError(e)) }
      if (live) timer = setTimeout(() => { void poll() }, 500)
    }
    timer = setTimeout(() => { void poll() }, 250)
    return () => { live = false; clearTimeout(timer) }
  }, [batch.id, batch.running])
  useEffect(() => {
    if (wasRunning.current && !batch.running) { finished.current?.(); title.current?.focus() }
    wasRunning.current = batch.running
  }, [batch.running])
  const action = async (work: () => Promise<void>) => {
    if (pending) return
    setPending(true); setError(null)
    try { await work() } catch (e) { setError(organizationError(e)) }
    finally { setPending(false) }
  }
  const moved = batch.items.filter((i) => i.status === 'moved').length
  const undone = batch.items.filter((i) => i.status === 'undone').length
  const untouched = batch.items.filter((i) => ['failed', 'cancelled', 'pending'].includes(i.status)).length
  const attention = batch.items.filter((i) => i.status === 'attention').length
  const completed = batch.items.filter((i) => !['pending', 'moving', 'undoing'].includes(i.status)).length
  return <section className={styles.result}>
    <p className={styles.eyebrow}>{new Date(batch.created_unix * 1000).toLocaleString()}</p>
    <h2 ref={title} tabIndex={-1}>{batch.running ? batch.undoing ? 'Putting files back…' : 'Giving things a home…' : batch.undoing ? `${undone} ${undone === 1 ? 'file' : 'files'} put back` : `${moved} ${noun(batch.kind, moved)} organized`}</h2>
    <p className={styles.path}>{batch.preference.destination.path}</p>
    <p role="status">{moved} organized · {undone} put back{untouched > 0 ? ` · ${untouched} left in place` : ''}{attention > 0 ? ` · ${attention} need a look` : ''}</p>
    {batch.running && <progress aria-label="Organization progress" value={batch.undoing ? undone : completed} max={batch.items.length} />}
    {(error || batch.error) && <p role="alert" className={styles.notice}>{error ?? batch.error}</p>}
    <div className={styles.actions}>
      {batch.running ? <button className={styles.link} disabled={pending} onClick={() => void action(() => api.cancelOrganization(batch.id))}>Stop after this file</button> : <>
        <button className={styles.primary} disabled={pending || moved === 0} onClick={() => void action(async () => setBatch(await api.undoOrganization(batch.id)))}>Undo</button>
        <button className={styles.link} disabled={pending} onClick={() => void action(() => api.revealOrganization(batch.id))}>Open folder</button>
        {untouched > 0 && (onReviewRemaining ? <button className={styles.link} disabled={pending} onClick={onReviewRemaining}>Review remaining files</button> : <RetryLink kind={batch.kind} />)}
      </>}
    </div>
    {!batch.running && <p className={styles.muted}>Undo checks each file and leaves edited files and occupied original paths alone.</p>}
    <details><summary>Review {batch.items.length} file results</summary><ul className={styles.files}>
      {batch.items.map((r) => <li className={styles.historyFile} key={r.file.opportunity.id}>
        <strong>{name(r.file.opportunity.path)} · {r.status.replace('_', ' ')}</strong>
        <p className={styles.path}>{r.file.opportunity.path}</p><p className={styles.target}>→ {r.file.destination}</p>
        {r.note && <p className={styles.notice}>{r.note}</p>}
      </li>)}
    </ul></details>
  </section>
}
function RetryLink({ kind }: { kind: OrganizationKind }) {
  const { go } = useStore()
  return <button className={styles.link} onClick={() => go({ name: 'organize', kind })}>Review remaining files</button>
}
export function OrganizationHistory() {
  const { go, refreshFindings } = useStore()
  const [batches, setBatches] = useState<OrganizationBatch[]>([])
  const [page, setPage] = useState(0)
  const [pending, setPending] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [retry, setRetry] = useState(0)
  useEffect(() => {
    let live = true
    void api.organizationHistory(page * 20).then((batches) => { if (live) { setBatches(batches); setError(null); setPending(false) } }, (e) => { if (live) { setError(organizationError(e)); setPending(false) } })
    return () => { live = false }
  }, [page, retry])
  return <div className={styles.page}>
    <button className={styles.link} onClick={() => go({ name: 'findings' })}>← All findings</button>
    <header className={styles.header}><h1>Organization history</h1><p className={styles.muted}>Ordinary folders. Recoverable moves. Nothing here expires.</p></header>
    {pending && <p role="status">Loading organization history…</p>}
    {error && <p role="alert">{error} <button className={styles.link} onClick={() => { setPending(true); setRetry((n) => n + 1) }}>Try again</button></p>}
    {!pending && !error && batches.length === 0 && <p>No organization moves on this page yet.</p>}
    {!pending && !error && batches.map((batch) => <BatchResult key={`${batch.id}-${retry}`} initial={batch} onFinished={() => { void refreshFindings() }} />)}
    <div className={styles.actions}>
      <button className={styles.link} disabled={page === 0 || pending} onClick={() => { setPending(true); setPage(page - 1) }}>Newer moves</button>
      <button className={styles.link} disabled={batches.length < 20 || pending} onClick={() => { setPending(true); setPage(page + 1) }}>Older moves</button>
    </div>
  </div>
}
