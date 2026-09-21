# Changelog

All notable changes to Scuttle are documented here, newest first.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
Scuttle uses [semantic versioning](https://semver.org/spec/v2.0.0.html). While the
major version is 0, a minor bump may change behaviour.

## [Unreleased]

## [0.1.0] - 2026-09-21

The first stable release. It exists because of a Windows test that went
wrong: Scuttle offered something that looked like a finished Discord installer,
and moving it broke Discord. Most of this release is making sure nothing like
that can happen again, and that when you *do* choose to move something, you
see exactly what will move and can always get it back.

### Fixed

- **Scuttle no longer mistakes an installed application for its installer.**
  The installer check accepted any `.exe` over 512 KB anywhere it looked —
  including inside `%LOCALAPPDATA%`. For an application that installs itself
  per user, like Discord, its own `app-1.0.x\Discord.exe` and its `Update.exe`
  were rated as confident, low-risk installers ("Discord is already
  installed"), which put them in *Move everything confident*. Moving them broke
  the application. Installers are now only looked for outside the folders
  applications keep for themselves, never inside anything shaped like an
  installation, and a bare `.exe` has to be named like an installer before it
  is considered at all.
- **Applications are recognised by their shape, not by a registry.** A folder
  with an updater beside versioned `app-*` folders, an Electron
  `resources/app.asar`, a program beside its libraries, its own uninstaller, or
  a macOS `.app` bundle is an application — whether or not uninstall records
  exist, and whether or not it is running right now. Nothing inside one, and no
  folder that contains one, is ever suggested or included in a batch.
- A leftover-application finding no longer counts "nothing is using it" as
  evidence, and says "Scuttle found no installed application called …" rather
  than claiming the application is gone.
- Two findings could be given the same id when one replaced another for the
  same path, so an action could name a different finding from the one shown.
- On Windows, `C:\Users`, `C:\Windows` and other folders directly on a drive
  were not treated as structural. They are now, as is the root of a mounted
  drive on macOS.
- A folder that *contains* something protected (SSH keys, a password vault,
  Scuttle's own Drawer) can no longer be moved — moving it would move that too.
- Folders to look in are now checked: a drive root, a system folder or a
  protected place is refused rather than becoming the area Scuttle may act in.

### Changed

- **Confidence, impact and permission are separate.** Scuttle used to fold
  everything into one "risk" rating, which both let an application through and
  stopped people moving their own files. Now every finding says how sure
  Scuttle is about what it is, what moving it could disrupt (nothing, a
  download, your file, an application's data, an application), and which ways
  of asking may move it.
- **Your own files are yours to move.** Downloads, screenshots, archives,
  documents and recently edited files can be moved after a short, specific
  caution — "changed 2 days ago", "Scuttle is unsure what this is" — accepted
  once for the whole move, not file by file. Nothing claims a file is unused or
  junk.
- **Applications and their data are never swept.** They are never suggested,
  never in a select-all or a group action, and an application folder can only
  be moved by opening it and choosing it on its own, with a plain warning that
  it will probably stop working until it is put back.
- **Every move is reviewed first.** Before anything moves you see the exact
  path of each item, whether it is one file or a whole folder (and how much is
  in it), why Scuttle noticed it, what will be refused and why, where it goes,
  and how putting it back works.
- The safety gate in the core recomputes all of this from the stored evidence
  and the live filesystem for every item, immediately before it moves. The
  interface cannot unlock anything by what it sends.
- **The Drawer keeps what cannot be rebuilt.** Only caches, build output and
  re-downloadable installers expire after the retention period. Your own
  files, application data and anything moved past a caution stay until you
  remove them. Everything already in the Drawer from an alpha is kept this way
  too, except caches.
- Emptying the Drawer leaves alone anything flagged after an interruption.

### Added

- **Scuttle's burrow.** A left click on the tray icon opens a small window
  beside it with where things stand, one short line from Scuttle, and buttons
  to open Scuttle, view the Drawer, rummage, open Settings or quit. Scuttle
  peeks up once when it opens. Right click still opens the plain menu, which
  now also has *Open the Drawer*. Opening either never starts anything.
- *Quiet Scuttle* in Settings: plain wording and no little reactions.
  Warnings and recovery messages are always plain regardless.
- Quitting while files are moving now stops at the next safe point, says so,
  and quits once the Drawer is settled. Quitting again does not wait.
- `scuttle --drawer-report` lists everything the Drawer holds or has held —
  where it came from, whether the held copy is still on disk — from a
  read-only view of the database. Useful for answering "what happened to…?".

### Security and recovery

- Restoring re-checks the way back: if a folder on the path has been replaced
  by a link or junction, or has become protected, the item stays safely in the
  Drawer instead.
- Restoring checks the held copy against the fingerprint taken when it went in,
  and refuses — flagging it — if it no longer matches.
- Every copy across drives is now verified by re-reading and hashing it,
  whatever its size (previously only up to 128 MB), before the original is
  removed. A stop during verification leaves the original untouched.
- A damaged Drawer record can no longer point a restore outside its folder.

### Also in this release

#### Added

- **Scuttle can update itself, when you ask it to.** It checks shortly after
  starting and about once a day (*Automatically check for updates* in Settings
  turns that off; *Check for updates* still works), and says so with a small,
  dismissible notice in the header. Nothing downloads until you choose to, with
  a gauge that does not pretend to know a size it does not; nothing restarts
  until you choose *Update and restart*, and it says plainly that this closes
  Scuttle and opens the new version. Release notes and the installed and
  available versions are shown.
- **An update never interrupts a move, restore, delete or rummage.** Installing
  goes through the same operation gate as every file operation: if one is
  running, the install is refused with a sentence naming it, the update stays
  ready, and nothing restarts by itself when the work ends. Once installing has
  begun nothing that changes files can start. Only a background check stands
  aside.
- Updates are verified with a signature whose public key is compiled into
  Scuttle, and which must be bound to the version being offered. A package that
  does not verify is thrown away and nothing is installed.
- Alpha builds are offered alphas and the stable release they led up to; stable
  builds are never offered a prerelease, and nobody is ever offered an older
  version. Versions are compared by semantic-version precedence.
- The release workflow now builds signed update packages and an update manifest,
  refuses to publish unless the whole platform matrix is present and every
  signature verifies against its file, and only then points installed copies at
  the new release. See `docs/updates.md`.

#### Changed

- Scuttle now makes one kind of network request — asking GitHub whether a newer
  version exists — where it previously made none. `docs/privacy.md` and
  `SECURITY.md` say so.

#### Known limitations

- Builds from before this release (`0.1.0-alpha.1`, `0.1.0-alpha.2`) have no
  updater. Install 0.1.0 by hand, once.
- There is no automatic rollback of a failed or unwanted update. A failed
  install leaves Scuttle running on the old version; a bad release is fixed by
  publishing a newer one.
- macOS builds are still ad-hoc signed, so the system may treat an updated copy
  as a new application for privacy permissions granted to the old one.
- A downloaded update is held in memory only, so it is downloaded again after a
  restart.
- The Windows tray icon follows the taskbar's light or dark setting as it was
  at launch; it does not change until Scuttle restarts.
- Folders on a different drive from the Drawer cannot be moved; single files
  can, with a verified copy. Moving the files inside a folder one by one is
  the workaround.


## [0.1.0-alpha.2] - 2026-09-21

### Added

- **Scuttle can stay in the menu bar** (system tray on Windows) instead of
  quitting when you close the window. Off by default. ⌘Q, logging out and
  shutting down still quit properly.
- **Optional background checks.** At most one a day, and only when the machine
  is on mains power and not busy. They read names, sizes and dates and never
  open a file, so they cannot find duplicates or near-identical screenshots —
  the findings screen says so. Nothing is moved or removed.
- **Optional notifications.** One summary a day at most, only when something
  new turned up, and never containing a filename. Pause until tomorrow from
  the menu or Settings.
- **Launch at login**, as an ordinary per-user login item. Separate setting.
- **Progress while moving.** A progress track with real counts and Cancel, an
  indicator in the header, and anything left unfinished stays reachable with
  what happened and what to do about it.
- **Recovery from an interrupted move.** Scuttle settles what really moved on
  the next start, and deletes nothing it is unsure of.

### Changed

- Expired drawer items are now removed while Scuttle is running, not only at
  startup. How long things last is unchanged.

### Fixed

- Moving a folder that is in use no longer fails with "modified since Scuttle
  found it". This is what made the Windows temp folder impossible to move.
- A failed folder move could lose files. It no longer can.
- Moving no longer freezes the window.
- Nothing is ever overwritten by a move or a restore.
- Moving, restoring, emptying the drawer and scanning no longer run over each
  other.
- Starting a rummage while a background check was running could still be
  refused as busy, if the check was slow to stand down. A rummage now displaces
  it outright.
- On Windows, long-path handling turned any path containing a forward slash
  into one the system could not open. Nothing shipped was affected — Scuttle
  builds its own paths a piece at a time — but every move failed under test,
  which is how it was found.

### Known limitations

- **None of the background behaviour has been tested on Windows.** It builds
  and its tests pass in CI, but the tray icon, power checks, login item and
  notifications were only tried by hand on macOS.
- Background checks cannot find duplicates or near-identical screenshots.
- Hiding the window does not give the memory back. Idle CPU is unmeasurable
  either way; the webview stays resident.
- Scuttle cannot tell a free machine from a merely quiet one, so expect it to
  skip days.
- On Windows the tray icon picks light or dark once at startup.
- On macOS the dock icon disappears while the window is hidden.

## [0.1.0-alpha.1] - 2026-09-21

The first public build. Scuttle rummages through the forgotten corners of your
computer, shows you what turned up and why it noticed, and never removes
anything on its own.

An alpha because of where it has been, not what it does: everything below is
finished and tested, on two operating systems, over fixture filesystems that
cover the awkward cases. What it has not had is a few hundred real machines,
which is the only thing that finds the rest. Treat emptying the drawer with
the seriousness the confirmation asks for — it is the one action that cannot
be undone.

### Added

- **Rummage.** One button walks the places a computer accumulates things —
  Downloads, Desktop, application support and cache folders, screenshot folders,
  and Steam and Epic libraries where they exist — and reports what it found
  without touching any of it.
- **Seven detectors.** Leftovers from software that is no longer installed,
  screenshots nobody looked at again, installers that already did their job,
  duplicate files, caches attributed to the application that made them, large
  strays, and developer build debris. Each finding carries the evidence behind
  it, and anything that cannot be named lands in Oddments rather than being
  guessed at.
- **Findings you can argue with.** Every finding shows what was observed, a
  confidence and a risk, and reasons to leave it alone where they exist. Nothing
  is presented as disposable just because it was found.
- **The drawer.** Removing something moves it to a holding folder inside
  Scuttle's own data directory and records where it came from, in the database
  and in a readable manifest beside the item. Nothing is deleted until you empty
  the drawer. Items expire after a retention window you choose, and are removed
  the next time Scuttle starts.
- **Restore.** Anything in the drawer goes back where it came from. Restoring
  recreates a parent folder that has since been deleted, and never overwrites
  something that arrived in the meantime — it restores alongside it and says so.
- **A safety layer that refuses.** System locations, credential stores, browser
  profiles, mail and message stores, and anything reached through a symbolic
  link are refused outright. Protected findings are still shown, so you know they
  exist, but Scuttle will not act on them.
- **Space.** A breakdown of what is actually using the disk, so a rummage's
  findings sit in proportion to the volume they came from.
- **Ignore lists** for a single item, an application, or a whole category.
- **`--dry-run`.** A terminal report of everything Scuttle would classify and
  what it would recommend, changing nothing. The one place full paths are printed.
- **Local by default.** No account, no network calls, no telemetry. Findings and
  drawer records live in a SQLite database on the machine that made them.
- macOS and Windows support, from one platform layer with a real implementation
  on each: `.app` bundles and `Info.plist` on macOS, the uninstall registry in
  both WOW64 views on Windows.
- A recorded run of the whole thing, filmed from the real application against
  300,000 invented files — it leads this release, and the
  [README](https://github.com/acltabontabon/scuttle#readme).

### Known limitations

- Releases are **not signed with an Apple Developer ID and not notarized** on
  macOS, and the Windows installer is **unsigned**. Both operating systems will
  warn you the first time; [the README](README.md#installing) explains what you
  will see and what to do about it.
- **Linux is not supported.** The platform layer compiles as a stub that knows
  about nothing, so a rummage there would find almost nothing and misjudge what
  it did find. There is no Linux download.
- **Nothing goes to the Trash or the Recycle Bin.** Emptying the drawer deletes
  permanently, which is what the confirmation says.
- A drawer on a different volume from the item being held turns the move into a
  copy, which needs room for both copies while it runs and reports no progress
  while it does. Most commonly a Steam library on a second drive on Windows.
- **There is no automatic updater.** New versions are downloaded from the
  releases page by hand, on purpose.
- Scuttle asks for no special permissions, so folders that need Full Disk Access
  on macOS are simply skipped and counted as places that could not be read.
- On Windows there are **no cache rules for Chrome or Edge**. Both keep their
  cache inside the browser profile directory, which Scuttle protects outright
  because it also holds logins, cookies and history. Firefox, which keeps its
  cache somewhere else, is covered.

[Unreleased]: https://github.com/acltabontabon/scuttle/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/acltabontabon/scuttle/compare/v0.1.0-alpha.2...v0.1.0
[0.1.0-alpha.2]: https://github.com/acltabontabon/scuttle/compare/v0.1.0-alpha.1...v0.1.0-alpha.2
[0.1.0-alpha.1]: https://github.com/acltabontabon/scuttle/releases/tag/v0.1.0-alpha.1
