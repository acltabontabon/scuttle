import { useEffect, useState } from 'react'

import { useStore } from '@/app/store'
import { api } from '@/lib/ipc'
import { bytes } from '@/lib/format'
import type { RootDescription, Settings as SettingsShape } from '@/lib/types'
import { Unavailable } from '@/components/Unavailable'
import { DryRun } from './DryRun'

import styles from './Settings.module.css'

/**
 * Settings.
 *
 * Six decisions, not a cockpit. Anything that could be inferred from the
 * machine is inferred from the machine, and anything that only exists to make
 * the screen look substantial is not here.
 */
export function Settings() {
  const { settings, updateSettings, say } = useStore()
  const [roots, setRoots] = useState<RootDescription[]>([])
  const [developerOpen, setDeveloperOpen] = useState(false)
  const [about, setAbout] = useState<{
    version: string
    platform: string
    quarantine_root: string
  } | null>(null)

  useEffect(() => {
    void api.suggestedRoots().then(setRoots)
    void api.about().then(setAbout)
  }, [])

  if (!settings) {
    return <Unavailable what="your settings" onRetry={() => window.location.reload()} />
  }

  const patch = (changes: Partial<SettingsShape>) =>
    void updateSettings({ ...settings, ...changes })

  // An empty list means "everywhere the platform suggests", which is what the
  // backend does too — so an unchecked box is stored by omission.
  const looksHere = (path: string) =>
    settings.scan_roots.length === 0 || settings.scan_roots.includes(path)

  const togglePlace = (path: string) => {
    const current =
      settings.scan_roots.length === 0 ? roots.map((root) => root.path) : settings.scan_roots
    const next = current.includes(path)
      ? current.filter((existing) => existing !== path)
      : [...current, path]
    if (next.length === 0) {
      say('Scuttle needs somewhere to look.', { tone: 'warn' })
      return
    }
    patch({ scan_roots: next.length === roots.length ? [] : next })
  }

  return (
    <div className={styles.room}>
      <div className={styles.inner}>
        <h2 className={styles.title}>Settings</h2>

        <section className={styles.group}>
          <h3 className={styles.groupTitle}>Where Scuttle looks</h3>
          <ul className={styles.places}>
            {roots.map((root) => (
              <li key={root.path} className={styles.place}>
                <button
                  className={styles.check}
                  role="checkbox"
                  aria-checked={looksHere(root.path)}
                  aria-label={root.label}
                  onClick={() => togglePlace(root.path)}
                >
                  ✓
                </button>
                <span>{root.label}</span>
                <span className={styles.placePath} title={root.path}>
                  {root.path}
                </span>
              </li>
            ))}
          </ul>
        </section>

        <section className={styles.group}>
          <h3 className={styles.groupTitle}>What Scuttle looks for</h3>

          <Toggle
            label="Developer debris"
            hint="Build output, package caches and dependency folders. Off by default, because these usually belong to something you are still working on."
            checked={settings.include_developer_debris}
            onChange={(value) => patch({ include_developer_debris: value })}
          />

          <div className={styles.row}>
            <div className={styles.rowText}>
              <div className={styles.rowLabel}>Call a file large at</div>
              <p className={styles.rowHint}>
                Anything at or above this gets mentioned. Scuttle never calls these junk.
              </p>
            </div>
            <div className={styles.choices}>
              {[512, 1024, 4096, 10240].map((mb) => {
                const value = mb * 1024 * 1024
                return (
                  <button
                    key={mb}
                    className={styles.choice}
                    aria-pressed={settings.heavy_threshold === value}
                    onClick={() => patch({ heavy_threshold: value })}
                  >
                    {bytes(value)}
                  </button>
                )
              })}
            </div>
          </div>
        </section>

        <section className={styles.group}>
          <h3 className={styles.groupTitle}>The drawer</h3>
          <div className={styles.row}>
            <div className={styles.rowText}>
              <div className={styles.rowLabel}>Hold things for</div>
              <p className={styles.rowHint}>
                Quarantined items are removed permanently after this. Until then you can put
                any of them back.
              </p>
            </div>
            <div className={styles.choices}>
              {[7, 14, 30].map((days) => (
                <button
                  key={days}
                  className={styles.choice}
                  aria-pressed={settings.quarantine_retention_days === days}
                  onClick={() => patch({ quarantine_retention_days: days })}
                >
                  {days} days
                </button>
              ))}
            </div>
          </div>
        </section>

        <section className={styles.group}>
          <h3 className={styles.groupTitle}>Appearance</h3>
          <div className={styles.row}>
            <div className={styles.rowText}>
              <div className={styles.rowLabel}>Theme</div>
            </div>
            <div className={styles.choices}>
              {(['system', 'light', 'dark'] as const).map((mode) => (
                <button
                  key={mode}
                  className={styles.choice}
                  aria-pressed={settings.appearance === mode}
                  onClick={() => patch({ appearance: mode })}
                >
                  {mode[0]!.toUpperCase() + mode.slice(1)}
                </button>
              ))}
            </div>
          </div>

          <Toggle
            label="Hold still"
            hint="Turns off Scuttle's movement. Your system's reduced-motion setting is respected either way."
            checked={settings.reduced_motion === true}
            onChange={(value) => patch({ reduced_motion: value ? true : null })}
          />
        </section>

        <section className={styles.group}>
          <h3 className={styles.groupTitle}>Things you told Scuttle to forget</h3>
          <div className={styles.row}>
            <div className={styles.rowText}>
              <p className={styles.rowHint}>
                Files, apps and piles you asked Scuttle not to mention again. Clearing this
                makes them eligible to turn up in the next rummage.
              </p>
            </div>
            <button
              className={`${styles.link} ${styles.danger}`}
              onClick={() => {
                void api.clearIgnores().then(() => say('Scuttle has a clean slate again.'))
              }}
            >
              Clear the list
            </button>
          </div>
        </section>

        <section className={styles.group}>
          <h3 className={styles.groupTitle}>Developer</h3>
          <div className={styles.row}>
            <div className={styles.rowText}>
              <div className={styles.rowLabel}>Dry run</div>
              <p className={styles.rowHint}>
                Classify everything and change nothing. Shows the verdict and the evidence
                behind every finding, with full paths — useful for working out why Scuttle
                said what it said.
              </p>
            </div>
            <button
              className={styles.link}
              onClick={() => setDeveloperOpen((open) => !open)}
              aria-expanded={developerOpen}
            >
              {developerOpen ? 'Hide' : 'Open'}
            </button>
          </div>
          {developerOpen && <DryRun />}
        </section>

        {about && (
          <footer className={styles.about}>
            Scuttle {about.version} on {about.platform}. Everything stays on this machine:
            no account, no cloud, no telemetry.
            <br />
            The drawer lives at <span className={styles.aboutPath}>{about.quarantine_root}</span>
          </footer>
        )}
      </div>
    </div>
  )
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string
  hint: string
  checked: boolean
  onChange: (value: boolean) => void
}) {
  return (
    <div className={styles.row}>
      <div className={styles.rowText}>
        <div className={styles.rowLabel}>{label}</div>
        <p className={styles.rowHint}>{hint}</p>
      </div>
      <button
        className={styles.switch}
        role="switch"
        aria-checked={checked}
        aria-label={label}
        onClick={() => onChange(!checked)}
      >
        <span className={styles.knob} />
      </button>
    </div>
  )
}
