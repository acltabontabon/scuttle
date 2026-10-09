import { useEffect, useRef, useState } from 'react'

import { api } from '@/lib/ipc'
import { bytes, shortPath } from '@/lib/format'
import { isScuttleError, type DependencyCacheKind, type DependencyCacheReport, type Settings } from '@/lib/types'
import type { SettingsChange } from '@/app/store'

import styles from './Settings.module.css'
import reportStyles from './DryRun.module.css'

const kinds: { value: DependencyCacheKind; label: string; hint: string }[] = [
  { value: 'maven', label: 'Maven', hint: 'Choose the local repository directory, usually .m2/repository.' },
  { value: 'gradle', label: 'Gradle', hint: 'Choose the Gradle user home directory, usually .gradle.' },
  { value: 'npm', label: 'npm', hint: 'Choose the _cacache directory inside the npm cache.' },
  { value: 'pnpm', label: 'pnpm', hint: 'Choose the pnpm store directory containing its versioned stores.' },
  { value: 'yarn', label: 'Yarn', hint: 'Choose the Yarn cache directory. Project caches are inspected only when added here.' },
]

export function DependencyCaches({ settings, patch, commonProjects }: {
  settings: Settings
  patch: (changes: SettingsChange) => Promise<boolean>
  commonProjects: string[]
}) {
  const preferences = settings.dependency_caches
  const [kind, setKind] = useState<DependencyCacheKind>('maven')
  const [result, setResult] = useState<{ optionsKey: string; report: DependencyCacheReport } | null>(null)
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [page, setPage] = useState(0)
  const [filter, setFilter] = useState('all')
  const generation = useRef(0)
  const optionsKey = JSON.stringify([preferences, settings.developer_roots])
  const optionsRef = useRef(optionsKey)
  const report = result?.optionsKey === optionsKey ? result.report : null

  // Results belong to the exact preferences used. Never display an old
  // report under a newly selected retention period or project boundary.
  useEffect(() => { optionsRef.current = optionsKey; generation.current += 1 }, [optionsKey])
  useEffect(() => () => { generation.current += 1 }, [])

  const fail = (caught: unknown) => setError(isScuttleError(caught) ? caught.message : String(caught))
  const run = async () => {
    const runGeneration = ++generation.current
    const runKey = optionsKey
    setRunning(true); setError(null); setResult(null); setPage(0)
    try {
      const next = await api.dependencyCachePreview()
      if (runGeneration === generation.current && runKey === optionsRef.current) setResult({ optionsKey: runKey, report: next })
    } catch (caught) { if (runGeneration === generation.current) fail(caught) }
    finally { setRunning(false) }
  }
  const addCache = async () => {
    setError(null)
    try {
      const path = await api.chooseDeveloperRoot()
      if (path) await patch((latest) => ({ dependency_caches: { ...latest.dependency_caches,
        locations: [...latest.dependency_caches.locations.filter((l) => l.path !== path || l.kind !== kind), { kind, path }],
      } }))
    } catch (caught) { fail(caught) }
  }
  const addProject = async () => {
    setError(null)
    try {
      const path = await api.chooseDeveloperRoot()
      if (path) await patch((latest) => ({ developer_roots: [...new Set([
        ...(latest.developer_roots.length ? latest.developer_roots : commonProjects), path,
      ])] }))
    } catch (caught) { fail(caught) }
  }
  const filtered = report?.entries.filter((entry) => filter === 'all' || entry.kind === filter || entry.decision === filter) ?? []

  return <section className={styles.group}>
    <h3 className={styles.groupTitle}>Dependency caches</h3>
    <div className={styles.row}>
      <div className={styles.rowText}>
        <div className={styles.rowLabel}>Preview dependency caches</div>
        <p className={styles.rowHint}>Inspect Maven, Gradle, npm, pnpm and Yarn caches. Older versions may still be needed. This preview moves and deletes nothing.</p>
      </div>
      <button className={styles.switch} role="switch" aria-checked={preferences.enabled}
        aria-label="Preview dependency caches" disabled={running}
        onClick={() => void patch((latest) => ({ dependency_caches: { ...latest.dependency_caches, enabled: !latest.dependency_caches.enabled } }))}>
        <span className={styles.knob} />
      </button>
    </div>
    {preferences.enabled && <>
      <div className={styles.row}>
        <div className={styles.rowText}>
          <label className={styles.rowLabel} htmlFor="dependency-retention">Preserve recent entries for</label>
          <p className={styles.rowHint}>Recent changes protect an entry. Old modification dates do not establish when a dependency was last used.</p>
        </div>
        <select className={styles.cacheSelect} id="dependency-retention" disabled={running} value={preferences.retention_days}
          onChange={(event) => void patch((latest) => ({ dependency_caches: { ...latest.dependency_caches,
            retention_days: Number(event.target.value) as 90 | 180 | 365,
          } }))}>
          {[90, 180, 365].map((days) => <option key={days} value={days}>{days} days</option>)}
        </select>
      </div>
      <p className={styles.rowHint}>Project folders: {(settings.developer_roots.length ? settings.developer_roots : commonProjects).map((path) => shortPath(path)).join(', ') || 'No project folders configured.'} These are shared with developer build artefacts.</p>
      <div className={styles.developerActions}>
        <button className={styles.link} disabled={running} onClick={() => void addProject()}>Add dependency project folder</button>
        {settings.developer_roots.length > 0 && <button className={styles.link} disabled={running}
          onClick={() => void patch({ developer_roots: [] })}>Use common project folders</button>}
      </div>
      <p className={styles.rowHint}>Default cache locations are included. Add custom locations if your tools use another directory.</p>
      <ul className={styles.places}>
        {preferences.locations.map((location) => <li key={`${location.kind}:${location.path}`} className={styles.developerPlace}>
          <span className={styles.aboutPath} title={location.path}>{location.kind} · {shortPath(location.path)}</span>
          <button className={styles.link} disabled={running} aria-label={`Remove cache ${location.path}`}
            onClick={() => void patch((latest) => ({ dependency_caches: { ...latest.dependency_caches,
              locations: latest.dependency_caches.locations.filter((l) => l.path !== location.path || l.kind !== location.kind),
            } }))}>Remove</button>
        </li>)}
      </ul>
      <div className={styles.developerActions}>
        <label>Cache type{' '}<select className={styles.cacheSelect} value={kind} disabled={running} onChange={(event) => setKind(event.target.value as DependencyCacheKind)}>
          {kinds.map((k) => <option key={k.value} value={k.value}>{k.label}</option>)}
        </select></label>
        <button className={styles.link} disabled={running || preferences.locations.length >= 10} onClick={() => void addCache()}>Add cache location</button>
      </div>
      <p className={styles.rowHint}>{kinds.find((k) => k.value === kind)?.hint}</p>
      <div className={reportStyles.panel}>
        <div className={reportStyles.controls}>
          <button className={reportStyles.run} disabled={running} onClick={() => void run()}>{running ? 'Inspecting caches…' : 'Inspect dependency caches'}</button>
          {running && <button className={styles.link} onClick={() => {
            void api.cancelDependencyCachePreview().catch(fail)
          }}>Stop cache inspection</button>}
          <span className={reportStyles.status}>Read only. Full paths are shown.</span>
        </div>
        {error && <p className={styles.trouble} role="alert">{error}</p>}
        {report && <div className={reportStyles.report} aria-live="polite">
          <p>{report.complete ? '' : 'Partial inventory · at least '}{bytes(report.bytes)} across {report.entries.length} recognized entries.</p>
          <p>{report.kept} kept · {report.insufficient_evidence} with insufficient evidence · {report.eligible} eligible for review.</p>
          <p>Policy {report.policy_version} · {report.retention_days} days · inspected {new Date(report.evaluated_unix * 1000).toLocaleString()}</p>
          {report.notes.map((note) => <p key={note} className={reportStyles.notes}>{note}</p>)}
          <details><summary>Repositories and project coverage</summary>
            {report.repositories.map((r) => <p key={`${r.kind}:${r.path}`} className={reportStyles.rowPath}>
              {r.kind} · {r.path} · {r.present ? `${r.entries} entries, ${r.complete ? '' : 'at least '}${bytes(r.bytes)}` : 'Not found or not inspected'}
            </p>)}
            {!report.projects.length && <p>No projects discovered; dependency coverage is unknown.</p>}
            {report.projects.map((p) => <div key={`${p.ecosystem}:${p.path}`} className={reportStyles.row}>
              <p>{p.ecosystem} · {p.path} · incomplete coverage</p>
              {p.notes.map((note) => <p key={note}>{note}</p>)}
            </div>)}
          </details>
          <label>Show{' '}<select className={styles.cacheSelect} value={filter} onChange={(event) => { setFilter(event.target.value); setPage(0) }}>
            <option value="all">All entries</option>
            {kinds.map((k) => <option key={k.value} value={k.value}>{k.label}</option>)}
            <option value="kept">Kept</option><option value="insufficient_evidence">Insufficient evidence</option>
          </select></label>
          {filtered.slice(page * 40, (page + 1) * 40).map((entry) => <div className={reportStyles.row} key={entry.id}>
            <div className={reportStyles.rowHead}>{entry.kind} · {entry.artifact}{entry.version && ` @ ${entry.version}`} · {entry.complete ? '' : 'at least '}{bytes(entry.bytes)}</div>
            <p>{entry.decision === 'kept' ? 'Kept' : entry.decision === 'eligible' ? 'Eligible for review' : 'Insufficient evidence'} · origin: {entry.origin}</p>
            <p className={reportStyles.rowPath}>{entry.path}</p>
            <p>Last use: {entry.last_used_unix === null ? 'unknown' : new Date(entry.last_used_unix * 1000).toLocaleString()}</p>
            <p>Last change: {entry.newest_modified_unix === null ? 'unknown' : new Date(entry.newest_modified_unix * 1000).toLocaleString()}</p>
            {entry.explanations.map((reason) => <p key={reason}>{reason}</p>)}
            {entry.projects.map((path) => <p className={reportStyles.rowPath} key={path}>Referenced by {path}</p>)}
          </div>)}
          {!filtered.length && <p>No recognized entries in this view.</p>}
          {filtered.length > 40 && <div className={reportStyles.controls}>
            <button className={styles.link} disabled={page === 0} onClick={() => setPage(page - 1)}>Previous entries</button>
            <span>{page + 1} / {Math.ceil(filtered.length / 40)}</span>
            <button className={styles.link} disabled={(page + 1) * 40 >= filtered.length} onClick={() => setPage(page + 1)}>Next entries</button>
          </div>}
        </div>}
      </div>
    </>}
  </section>
}
