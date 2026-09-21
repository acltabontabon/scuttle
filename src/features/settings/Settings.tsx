import { useCallback, useEffect, useState } from 'react'

import { useStore } from '@/app/store'
import { api } from '@/lib/ipc'
import { bytes, shortPath } from '@/lib/format'
import { platform } from '@/lib/platform'
import {
  CHECKS_HINT,
  expiryHint,
  NEEDS_CHECKS,
  NOTIFY_HINT,
  needsTray,
  keepInTrayHint,
  keepInTrayLabel,
  launchAtLoginHint,
  launchAtLoginTrouble,
  pauseState,
  standing,
  trouble,
} from '@/features/background/phrasing'
import type {
  BackgroundStatus,
  IgnoredEntry,
  RootDescription,
  Settings as SettingsShape,
} from '@/lib/types'
import { Unavailable } from '@/components/Unavailable'
import { AUTO_CHECK_HINT, AUTO_CHECK_LABEL, installedLine, LATER } from '@/features/updates/phrasing'
import { ReleaseNotes } from '@/features/updates/ReleaseNotes'
import { UpdateTrack } from '@/features/updates/UpdateChip'
import { useUpdates } from '@/features/updates/UpdateProvider'
import { describe as describeUpdate } from '@/features/updates/view'
import updateStyles from '@/features/updates/Updates.module.css'
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
  const {
    settings,
    updateSettings,
    say,
    background,
    pauseBackground,
    setLaunchAtLogin,
  } = useStore()
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
                label="Let rebuildable things go after"
                /*
                 * The scope here is the part worth getting right. `expires_unix`
                 * is stamped on each record when it is quarantined, and the
                 * startup sweep compares against that stored value — so
                 * changing this never moves the deadline on anything already in
                 * the drawer, in either direction.
                 */
                hint={
                  'Only caches, build output and installers that can be downloaded again ever ' +
                  'expire. Your own files, application data and anything you moved past a ' +
                  'caution stay until you remove them. Applies to items moved from now on. ' +
                  expiryHint(settings.background_mode)
                }
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

              <Toggle
                label="Quiet Scuttle"
                hint="Plain wording and no little reactions, in the window and the tray. Warnings and recovery messages are always plain either way."
                checked={settings.personality === 'quiet'}
                onChange={(value) => void patch({ personality: value ? 'quiet' : 'full' })}
              />
            </section>

            <BackgroundGroup
              settings={settings}
              status={background}
              patch={patch}
              onPause={pauseBackground}
              onLaunchAtLogin={setLaunchAtLogin}
              say={say}
            />

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
                      telemetry. The one thing Scuttle ever asks the network is
                      whether a newer version exists.
                    </p>
                  </div>
                </div>
                <UpdateRows
                  autoCheck={settings.auto_check_updates}
                  onAutoCheck={(value) => void patch({ auto_check_updates: value })}
                />
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

/**
 * Staying in the menu bar, and what may follow from it.
 *
 * Three nested decisions, each off by default and each disabled until the one
 * above it is on — because a check cannot happen if closing the window quits,
 * and there is nothing to notify about if no checks happen. The backend
 * enforces the same rule, so this is the interface agreeing rather than the
 * interface deciding.
 *
 * Launch at login sits apart on purpose. Staying in the menu bar is not
 * permission to start with the machine, and the two are never bundled.
 */
function BackgroundGroup({
  settings,
  status,
  patch,
  onPause,
  onLaunchAtLogin,
  say,
}: {
  settings: SettingsShape
  status: BackgroundStatus | null
  patch: (changes: Partial<SettingsShape>) => Promise<void>
  onPause: (paused: boolean) => Promise<void>
  onLaunchAtLogin: (enabled: boolean) => Promise<void>
  say: (text: string, options?: { tone?: 'warn' }) => void
}) {
  const os = platform()
  // The core sends its own clock with the status, so nothing here has to ask
  // the browser what time it is during a render.
  const now = status?.now_unix ?? 0
  const pause = pauseState(status?.paused_until_unix ?? 0, now)
  const wrong = status ? trouble(status, os) : null
  const loginTrouble = status ? launchAtLoginTrouble(status) : null

  /**
   * Turning the tray on explains itself once, at the moment the decision is
   * made. Explaining at the first close instead would mean either delaying
   * the close or saying it after the window had already gone.
   */
  const onMode = async (value: boolean) => {
    await patch({
      background_mode: value,
      ...(value && !settings.background_intro_seen ? { background_intro_seen: true } : {}),
    })
    if (value && !settings.background_intro_seen) {
      say(
        `Closing the window will now leave Scuttle in the ${
          os === 'windows' ? 'system tray' : 'menu bar'
        }. Quit it from there when you want it gone.`,
      )
    }
  }

  /**
   * Notifications are asked for at the moment they are switched on, which is
   * the only moment the request makes sense to anybody. A refusal leaves the
   * rest of background mode working and the toggle honestly off.
   */
  const onNotify = async (value: boolean) => {
    if (!value) {
      await patch({ background_notify: false })
      return
    }
    const allowed = await api.requestNotificationPermission()
    await patch({ background_notify: allowed })
    if (!allowed) {
      say('Your system is not allowing notifications, so that stays off.', { tone: 'warn' })
    }
  }

  return (
    <section className={styles.group}>
      <h3 className={styles.groupTitle}>When the window is closed</h3>
      <p className={styles.groupNote}>
        {status ? standing(status, now) : 'Off. Closing the window quits Scuttle.'}
      </p>

      <Toggle
        label={keepInTrayLabel(os)}
        hint={keepInTrayHint(os)}
        checked={settings.background_mode}
        onChange={(value) => void onMode(value)}
      />

      {wrong && <p className={styles.trouble}>{wrong}</p>}

      <Toggle
        label="Check occasionally in the background"
        hint={CHECKS_HINT}
        checked={settings.background_checks}
        disabled={!settings.background_mode}
        disabledHint={needsTray(os)}
        onChange={(value) => void patch({ background_checks: value })}
      />

      {settings.background_checks && (
        <div className={styles.row}>
          <div className={styles.rowText}>
            <div className={styles.rowLabel}>
              {pause.paused ? 'Paused' : 'Checking when the machine is free'}
            </div>
            <p className={styles.rowHint}>
              {pause.paused
                ? 'Nothing will be looked at until tomorrow.'
                : 'You can stop it until tomorrow without turning it off.'}
            </p>
          </div>
          <button className={styles.link} onClick={() => void onPause(!pause.paused)}>
            {pause.label}
          </button>
        </div>
      )}

      <Toggle
        label="Tell me what turned up"
        hint={NOTIFY_HINT}
        checked={settings.background_notify}
        disabled={!settings.background_checks}
        disabledHint={NEEDS_CHECKS}
        onChange={(value) => void onNotify(value)}
      />

      <Toggle
        label="Launch at login"
        hint={launchAtLoginHint(os)}
        checked={status?.launch_at_login ?? settings.launch_at_login}
        disabled={status !== null && !status.launch_at_login_available}
        disabledHint="Scuttle cannot set this up on this system."
        onChange={(value) => void onLaunchAtLogin(value)}
      />

      {loginTrouble && <p className={styles.trouble}>{loginTrouble}</p>}
    </section>
  )
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
  disabled = false,
  disabledHint,
}: {
  label: string
  hint: string
  checked: boolean
  onChange: (value: boolean) => void
  /**
   * A setting that depends on another one above it. The row stays visible so
   * the shape of the decision is legible, and says what it is waiting for
   * rather than simply refusing to respond.
   */
  disabled?: boolean
  disabledHint?: string
}) {
  return (
    <div className={styles.row} data-disabled={disabled || undefined}>
      <div className={styles.rowText}>
        <div className={styles.rowLabel}>{label}</div>
        <p className={styles.rowHint}>{disabled && disabledHint ? disabledHint : hint}</p>
      </div>
      <button
        className={styles.switch}
        role="switch"
        aria-checked={checked}
        aria-label={label}
        disabled={disabled}
        onClick={() => onChange(!checked)}
      >
        <span className={styles.knob} />
      </button>
    </div>
  )
}

/**
 * Updating, in Settings: the switch, the state in a line, and — only when
 * there is one — the update itself, its notes and what to do about it.
 *
 * Everything decided here is decided by `describe`, the same function that
 * drives the header chip, so the two can never disagree about what is going on.
 */
function UpdateRows({
  autoCheck,
  onAutoCheck,
}: {
  autoCheck: boolean
  onAutoCheck: (value: boolean) => void
}) {
  const { snapshot, act, dismiss } = useUpdates()
  const d = snapshot ? describeUpdate(snapshot) : null

  return (
    <>
      <Toggle
        label={AUTO_CHECK_LABEL}
        hint={AUTO_CHECK_HINT}
        checked={autoCheck}
        onChange={onAutoCheck}
      />
      {snapshot && d && (
        <>
          <div className={styles.row}>
            <div className={styles.rowText}>
              <div className={styles.rowLabel} role="status">
                {d.headline}
              </div>
              <p className={styles.rowHint}>
                {[installedLine(snapshot.current_version, snapshot.channel) + ' is installed.', d.detail]
                  .filter(Boolean)
                  .join(' ')}
              </p>
            </div>
            {d.action && (
              <button
                className={styles.link}
                disabled={d.working}
                onClick={() => void act(d.action!.kind)}
              >
                {d.action.label}
              </button>
            )}
          </div>
          {(d.info || d.track || d.notice || d.blocked || d.failure) && (
            <div className={updateStyles.settingsExtra}>
              {d.track && <UpdateTrack track={d.track} label={d.headline} />}
              {d.info && d.stage !== 'installing' && <ReleaseNotes info={d.info} />}
              {d.notice && <p className={updateStyles.notice}>{d.notice}</p>}
              {d.blocked && (
                <p className={updateStyles.blocked} role="status">
                  {d.blocked}
                </p>
              )}
              {d.failure && (
                <p className={updateStyles.failure} role="alert">
                  {d.failure}
                </p>
              )}
              {(d.stage === 'available' || d.stage === 'ready') && !snapshot.dismissed && (
                <div className={updateStyles.settingsActions}>
                  <button className={styles.link} onClick={() => void dismiss()}>
                    {LATER}
                  </button>
                </div>
              )}
            </div>
          )}
        </>
      )}
    </>
  )
}
