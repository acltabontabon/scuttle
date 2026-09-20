import { useState } from 'react'

import { api } from '@/lib/ipc'
import { isScuttleError, type DryRunReport, type DryRunRow } from '@/lib/types'

import styles from './DryRun.module.css'

/**
 * Developer mode: classify everything, change nothing.
 *
 * This is the one screen in Scuttle that shows full filesystem paths, and it
 * shows them because you explicitly asked what the detectors would do on
 * *this* machine. It is the tool for answering "why did it say that" without
 * having to trust a screenshot.
 */
export function DryRun() {
  const [report, setReport] = useState<DryRunReport | null>(null)
  const [running, setRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [developerDebris, setDeveloperDebris] = useState(false)

  const run = async () => {
    setRunning(true)
    setError(null)
    try {
      setReport(await api.dryRun({ includeDeveloperDebris: developerDebris }))
    } catch (caught) {
      setError(isScuttleError(caught) ? caught.message : String(caught))
    } finally {
      setRunning(false)
    }
  }

  return (
    <div className={styles.panel}>
      <div className={styles.controls}>
        <button className={styles.run} onClick={() => void run()} disabled={running}>
          {running ? 'Rummaging…' : 'Run a dry run'}
        </button>
        <label className={styles.status}>
          <input
            type="checkbox"
            checked={developerDebris}
            onChange={(event) => setDeveloperDebris(event.target.checked)}
          />{' '}
          include developer debris
        </label>
        <span className={styles.status}>
          Nothing is modified. Full paths are shown.
        </span>
      </div>

      {error && <p className={`${styles.notes} ${styles.reasonAgainst}`}>{error}</p>}

      {report && (
        <div className={styles.report}>
          <Group title="Would quarantine" rows={report.would_quarantine} />
          <Group title="Would put in front of you" rows={report.would_review} />
          <Group title="Would only point at" rows={report.would_surface} />

          <div className={styles.notes}>
            {report.notes.map((note) => (
              <div key={note}>{note}</div>
            ))}
            <div>
              {report.files_seen.toLocaleString()} files in {report.duration_ms} ms ·
              detectors: {report.detectors_run.join(', ')}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

function Group({ title, rows }: { title: string; rows: DryRunRow[] }) {
  return (
    <section className={styles.group}>
      <h4 className={styles.groupTitle}>
        {title}: {rows.length}
      </h4>
      {rows.slice(0, 40).map((row) => (
        <div key={row.path} className={styles.row}>
          <div className={styles.rowHead}>
            <span>{row.display_name}</span>
            <span>({row.size_human})</span>
            <span>[{row.category}]</span>
            <span>
              {row.confidence} / {row.risk} · {row.detector}
            </span>
          </div>
          <div className={styles.rowPath}>{row.path}</div>
          {row.evidence.map((reason) => (
            <div key={reason} className={styles.reason}>
              ✓ {reason}
            </div>
          ))}
          {row.negative_evidence.map((reason) => (
            <div key={reason} className={styles.reasonAgainst}>
              ✗ {reason}
            </div>
          ))}
        </div>
      ))}
      {rows.length > 40 && <div className={styles.rowPath}>… and {rows.length - 40} more</div>}
    </section>
  )
}
