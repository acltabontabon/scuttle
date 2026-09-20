import { useCallback, useEffect, useState } from 'react'

import { useStore } from '@/app/store'
import { api } from '@/lib/ipc'
import { bytes, shortPath } from '@/lib/format'
import type {
  IgnoredEntry,
  RootDescription,
  Settings as SettingsShape,
} from '@/lib/types'
import { Unavailable } from '@/components/Unavailable'
import { DryRun } from './DryRun'
import { Ignored } from './Ignored'

import styles from './Settings.module.css'

/**
 * Settings.
 *
 * Six decisions, not a cockpit. Anything that could be inferred from the
 * machine is inferred from the machine, and anything that only exists to make
 * the screen look substantial is not here.
 *
 * Two columns with a shared top baseline: scanning and file handling on the
 * left, preferences and tools on the right. They are real columns rather than
 * a `columns: 2` flow, so the order on the page is the order a person reads
 * and the order the keyboard walks.
 */
export function Settings() {
  const { settings, updateSettings, say } = useStore()
  const [roots, setRoots] = useState<RootDescription[]>([])
  const [ignored, setIgnored] = useState<IgnoredEntry[] | null>(null)
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false)
  const [failed, setFailed] = useState<string | null>(null)
  const [about, setAbout] = useState<{
    version: string
    platform: string
    quarantine_root: string
  } | null>(null)

  // A rejected load would otherwise leave this on "Looking…" for good.
  const loadIgnored = useCallback(() => {
    void api
      .ignored()
      .then(setIgnored)
      .catch(() => setIgnored([]))
  }, [])

  useEffect(() => {
    void api.suggestedRoots().then(setRoots)
    void api.about().then(setAbout)
    loadIgnored()
  }, [loadIgnored])

  if (!settings) {
    return <Unavailable what="your settings" onRetry={() => window.location.reload()} />
  }

  /**
   * Every change is written straight through, so there is no save button and
   * no unsaved state to lose. A failure is reported here rather than only in
   * a toast at the edge of the screen, because the control that did not take
   * is the thing somebody is looking at.
   */
  const patch = async (changes: Partial<SettingsShape>) => {
    setFailed(null)
    const ok = await updateSettings({ ...settings, ...changes })
    if (!ok) setFailed('That did not save. Your previous setting is still in effect.')
  }

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
    void patch({ scan_roots: next.length === roots.length ? [] : next })
  }

  return (
    <div className={styles.room}>
      <div className={styles.inner}>
        <header className={styles.masthead}>
          <h2 className={styles.title}>Settings</h2>
          <p className={styles.autosave} role="status">
            {failed ?? 'Changes save as you make them.'}
          </p>
        </header>

        <div className={styles.columns}>
          {/* Left: what Scuttle does with your disk. */}
          <div className={styles.column}>
            <section className={styles.group}>
              <h3 className={styles.groupTitle}>Where Scuttle looks</h3>
              <p className={styles.groupNote}>
                Folders included in a rummage. Changes apply to the next one.
              </p>
              <ul className={styles.places}>
                {roots.map((root) => (
                  <Place
                    key={root.path}
                    root={root}
                    checked={looksHere(root.path)}
                    onToggle={() => togglePlace(root.path)}
                  />
                ))}
              </ul>
            </section>

            <section className={styles.group}>
              <h3 className={styles.groupTitle}>What Scuttle looks for</h3>
              <p className={styles.groupNote}>Applies to the next rummage.</p>

              <Toggle
                label="Developer build artefacts"
                hint={
                  'Finds build output, package caches and dependency folders — node_modules, ' +
                  'target, .gradle and the like. Off by default because they usually belong ' +
                  'to something you are still working on. Turning this on only surfaces them ' +
                  'for review; nothing is removed.'
                }
                checked={settings.include_developer_debris}
                onChange={(value) => void patch({ include_developer_debris: value })}
              />

              <Choice
                label="Large files start at"
                hint="Files this size or larger are listed for you to review. Being large is not a reason to remove something, and Scuttle never suggests it."
                options={[512, 1024, 4096, 10240].map((mb) => ({
                  value: mb * 1024 * 1024,
                  label: bytes(mb * 1024 * 1024),
                }))}
                current={settings.heavy_threshold}
                onPick={(value) => void patch({ heavy_threshold: value })}
              />
            </section>

            <section className={styles.group}>
              <h3 className={styles.groupTitle}>The drawer</h3>
              <Choice
                label="Keep items in the drawer for"
                /*
                 * The scope here is the part worth getting right. `expires_unix`
                 * is stamped on each record when it is quarantined, and the
                 * startup sweep compares against that stored value — so
                 * changing this never moves the deadline on anything already in
                 * the drawer, in either direction.
                 */
                hint="Applies to items quarantined from now on. Anything already in the drawer keeps the date it was given. Expired items are deleted the next time Scuttle starts."
                options={[7, 14, 30].map((days) => ({ value: days, label: `${days} days` }))}
                current={settings.quarantine_retention_days}
                onPick={(value) => void patch({ quarantine_retention_days: value })}
              />
            </section>
          </div>

          {/* Right: preferences and the tools around them. */}
          <div className={styles.column}>
            <section className={styles.group}>
              <h3 className={styles.groupTitle}>Appearance</h3>
              <p className={styles.groupNote}>Applies immediately.</p>

              <Choice
                label="Theme"
                options={(['system', 'light', 'dark'] as const).map((mode) => ({
                  value: mode,
                  label: mode[0]!.toUpperCase() + mode.slice(1),
                }))}
                current={settings.appearance}
                onPick={(value) => void patch({ appearance: value })}
              />

              <Toggle
                label="Hold still"
                /*
                 * `data-motion="still"` is read by every module that animates,
                 * not only the mascot — the settling heaps, the gauge, the
                 * drawer's tray. The old wording undersold it.
                 */
                hint="Stops Scuttle's animations — the mascot, the settling piles, everything decorative. Your system's reduced-motion setting is always respected, whether this is on or not."
                checked={settings.reduced_motion === true}
                onChange={(value) => void patch({ reduced_motion: value ? true : null })}
              />
            </section>

            <Ignored entries={ignored} onChanged={loadIgnored} say={say} />

            <section className={styles.group}>
              <h3 className={styles.groupTitle}>Diagnostics</h3>
              <div className={styles.row}>
                <div className={styles.rowText}>
                  <div className={styles.rowLabel}>Dry run</div>
                  <p className={styles.rowHint}>
                    Classifies everything and changes nothing.
                  </p>
                </div>
                <button
                  className={styles.link}
                  onClick={() => setDiagnosticsOpen((open) => !open)}
                  aria-expanded={diagnosticsOpen}
                >
                  {diagnosticsOpen ? 'Close' : 'Open diagnostics'}
                </button>
              </div>
              {diagnosticsOpen && <DryRun />}
            </section>

            {about && (
              <section className={styles.group}>
                <h3 className={styles.groupTitle}>About</h3>
                <div className={styles.row}>
                  <div className={styles.rowText}>
                    <div className={styles.rowLabel}>
                      Scuttle {about.version} · {about.platform}
                    </div>
                    <p className={styles.rowHint}>
                      Everything stays on this machine. No account, no cloud, no
                      telemetry.
                    </p>
                  </div>
                </div>
                <div className={styles.row}>
                  <div className={styles.rowText}>
                    <div className={styles.rowLabel}>Drawer folder</div>
                    <p className={styles.aboutPath} title={about.quarantine_root}>
                      {shortPath(about.quarantine_root)}
                    </p>
                  </div>
                  <button
                    className={styles.link}
                    onClick={() => void api.revealQuarantineRoot()}
                  >
                    Open folder
                  </button>
                </div>
              </section>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

/**
 * One scan location.
 *
 * The folder name leads and the path sits underneath in secondary text, so a
 * long path can no longer squeeze a two-word label like "Steam library" into
 * two lines. The path is its own button: it expands to the full thing, which
 * makes it reachable by keyboard, and clicking it does not toggle the
 * checkbox next to it.
 */
function Place({
  root,
  checked,
  onToggle,
}: {
  root: RootDescription
  checked: boolean
  onToggle: () => void
}) {
  const [open, setOpen] = useState(false)
  const short = shortPath(root.path)
  const shortened = short !== root.path

  return (
    <li className={styles.place}>
      <button
        className={styles.check}
        role="checkbox"
        aria-checked={checked}
        onClick={onToggle}
      >
        <span className={styles.checkMark} aria-hidden="true">
          ✓
        </span>
        <span className={styles.placeLabel}>{root.label}</span>
      </button>

      {shortened ? (
        <button
          className={styles.placePath}
          onClick={() => setOpen((on) => !on)}
          aria-expanded={open}
          aria-label={`Full path for ${root.label}`}
          title={root.path}
          data-open={open || undefined}
        >
          {open ? root.path : short}
        </button>
      ) : (
        <span className={styles.placePathPlain}>{root.path}</span>
      )}
    </li>
  )
}

/** A segmented control. Same shape everywhere it appears. */
function Choice<T extends string | number>({
  label,
  hint,
  options,
  current,
  onPick,
}: {
  label: string
  hint?: string
  options: { value: T; label: string }[]
  current: T
  onPick: (value: T) => void
}) {
  return (
    <div className={styles.row}>
      <div className={styles.rowText}>
        <div className={styles.rowLabel} id={`${label}-label`}>
          {label}
        </div>
        {hint && <p className={styles.rowHint}>{hint}</p>}
      </div>
      <div className={styles.choices} role="group" aria-labelledby={`${label}-label`}>
        {options.map((option) => (
          <button
            key={String(option.value)}
            className={styles.choice}
            aria-pressed={current === option.value}
            onClick={() => onPick(option.value)}
          >
            {option.label}
          </button>
        ))}
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
