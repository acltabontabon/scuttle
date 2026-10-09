# Changelog

All notable changes to Scuttle are documented here, newest first.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
Scuttle uses [semantic versioning](https://semver.org/spec/v2.0.0.html). While the
major version is 0, a minor bump may change behaviour.

## [Unreleased]

## [0.2.0] - 2026-10-10

**Give things a home.**

A clearer desktop starts with deciding where things belong. Scuttle 0.2.0 adds
organization for the files you want to keep, a refreshed way to browse findings,
and more visibility into a developer’s disk.

### Highlights

- **Screenshots, gathered.** Collect loose screenshots from Desktop, Downloads
  and Documents into monthly folders—or keep them together. See thumbnails,
  choose a destination, and review every before-and-after path.
- **Installers with a home.** Bring scattered installers into one folder while
  preserving their names. Existing files are never overwritten; any numbered
  suffix is shown before you confirm.
- **A way back, even after a restart.** Organization history keeps each result
  and its Undo action. Changed files and occupied original paths are left alone.
  Organizing keeps your files; it does not reclaim storage.
- **A clearer rummage.** Browse illustrated categories, search and sort files,
  and get practical guidance when some locations could not be scanned.
- **See what development leaves behind.** Opt in to verified stale build-output
  suggestions and inspect Maven, Gradle, npm, pnpm and Yarn dependency caches.
  Dependency cache previews are read-only and explain why entries are preserved.

![Review screenshot thumbnails and exact monthly destinations before organizing. Sample files in the design preview.](https://raw.githubusercontent.com/acltabontabon/scuttle/v0.2.0/docs/media/organization-review.jpg)

**Try it:** Rummage → **Organize…** → choose files and a destination → review →
**Organize**. Find **Organization history** in Findings whenever you need Undo.

### Added

- Screenshot and installer organization: shallow discovery of loose files in
  Desktop, Downloads and Documents, local thumbnail previews, configurable
  destinations, monthly screenshot grouping, and exact before/after review.
- Persistent organization history with per-file outcomes, cancellation, safe
  retries and content-verified Undo. Organization moves never expire or enter
  the Drawer; interrupted moves are reconciled on startup.

- Opt-in developer cleanup discovers common workspaces and custom folders, with
  14, 30, or 60 days of inactivity before verified build output is suggested.
- Opt-in, read-only dependency cache preview for Maven, Gradle, npm, pnpm and
  Yarn, with custom cache locations, project references, preservation reasons
  and paginated entries in Settings.
- Dependency cache retention choices of 90, 180 or 365 days, independent of
  build output. Recent changes, local Maven installs, snapshots and Keep
  decisions preserve entries; old modification dates never establish last use.

### Changed

- Refreshed home and Findings screens, searchable and sortable pile browsing,
  and actionable notices for skipped scan locations.
- Updated website previews and device-aware downloads; mobile visitors get the
  release page instead of an arbitrary desktop installer.
- README now leads with the product experience, key features and downloads;
  detailed installation and usage instructions live in linked guides.

- Cargo, Next.js, and .NET output now requires project, repository, activity,
  and running-tool checks, repeated before moving to the drawer. Dependencies
  and uncertain output remain manual choices without automatic expiry.
- Existing developer items in the drawer are kept until manually removed.
- Managed dependency stores are protected from generic cache and parent-folder
  moves even when preview is off. Previously held cache items remain restorable.
- Dependency references and npm integrity hashes are indexed once per
  inspection, avoiding repeated lockfile scans. Metadata classification reuses
  its stat result instead of performing redundant filesystem reads.
- Desktop dry runs run on a worker under the operation gate. Desktop and
  command-line dry runs include the enabled cache preview and honor saved
  Keep/ignore decisions; cache-only terminal previews work without ordinary
  scan roots.
- Developer, architecture, privacy and safety documentation describes the cache
  preview, its supported layouts and its limits.

### Fixed

- Missing entries, unknown modification times and manifests removed during
  inspection mark aggregate totals partial rather than presenting exact sizes.
- Keeping a file inside an inventoried version preserves the whole version.
- Cancellation issued while a cache inspection is queued survives worker
  startup, without cancelling a later retry.
- Nested Maven plugin dependencies no longer replace the plugin's own
  dependency coordinates during POM inspection.
- pnpm executable content blobs are included in recognized cache sizes.

### Known limitations

- Organization has been tested on macOS, including simulated cross-drive
  transfers. Manual Windows interaction testing is pending; native filesystem tests run in CI.

- Dependency cache entries remain inspection-only. Cleanup requires verified
  usage tracking, dependency coverage, tool coordination and recovery. Maven
  resolution, pnpm/Yarn lockfile references and other branches remain partial.
- Dependency cache preview has been validated on macOS; manual Windows interaction
  testing is pending; native tests run in CI.

## [0.1.1] - 2026-10-05

**A steadier rummage. A clearer picture of your disk.**

Scuttle finds forgotten files, explains why they caught its eye, and lets you
review what to keep. This maintenance release makes scanning more dependable,
duplicate detection more careful, and everyday controls easier to recover from
when something goes wrong.

### Highlights

- **Rummages that keep up.** Even a scan that finishes immediately reaches the
  findings screen; repeated clicks leave the active rummage running properly.
- **More trustworthy duplicates.** Hard links are counted as one file, and
  files that change or become links during a scan are rejected.
- **Honest space figures.** Incomplete measurements say **at least**, refresh
  with your findings, and stop between entries after a five-second budget.
- **Preferences that stick.** Quick changes save in order, while unreadable
  lists and failed actions give you a clear error and a way to try again.

### Fixed

- Fast rummages no longer lose progress or completion events that arrive before
  the start response. Repeated clicks cannot replace a running scan's state.
- Failed database registration, worker startup, and worker panics release the
  scan's operation gate, allowing a later rummage to run.
- Duplicate detection excludes hard links and repeated paths, rejects changed
  file sizes and links swapped in after traversal, and responds to cancellation
  while reading large files.
- Space measurements mark unopened, protected and depth-limited folders as
  partial, and count empty directories against their work budget. Empty scans
  are recognised as completed scans.
- Space measurements stop between entries after a five-second time budget.
  Fixture-platform measurements use the fixture home instead of the real home.
- Space figures refresh when findings change; simultaneous requests share one
  measurement, and older responses cannot overwrite figures for a newer scan.
- Rapid preference changes preserve earlier saves, including folder selections
  and launch-at-login changes.
- Unreadable ignored-item lists and scan folders show failures with retry
  controls. Failed notification requests, ignore clearing, cancellation and
  folder reveals report what happened.
- The drawer's Open folder action opens the holding folder itself.

### Changed

- Development instructions use locked installs, the current Diagnostics path,
  and the dependencies' actual minimums: Rust 1.89 and Node 22.12.

## [0.1.0] - 2026-09-21

The first stable release.

### Added

- **Review before moving.** Every move opens a review first: the exact path,
  whether it's a file or a whole folder, why Scuttle noticed it, and how to get
  it back.
- **Scuttle's burrow.** Click the tray icon for a quick look at where things
  stand, with shortcuts to open Scuttle, the drawer, a rummage or settings.
  Right-click still opens the menu.
- **Quiet Scuttle** in Settings, for plain wording and no animations.
- **Updates.** Scuttle checks about once a day and always asks before
  downloading or restarting. Updates are signature-verified.
- `scuttle --drawer-report` lists what the drawer holds, read-only.

### Changed

- **Your files, your call.** Downloads, screenshots, documents and recently
  edited files can always be moved. Anything worth knowing is said once for the
  whole move.
- **Installed apps are left alone.** Applications are recognised by how they're
  built, never suggested for cleanup and never included in a batch.
- **The drawer keeps what can't be rebuilt.** Only caches and re-downloadable
  installers expire on their own; everything else stays until you remove it.
- Quitting while files are moving finishes safely first.

### Fixed

- Installer detection no longer picks up programs inside application folders.
- Folders containing protected files, and folders directly on a drive, can no
  longer be moved.
- Restores check the way back and the held copy before putting anything back,
  and never overwrite.
- Copies across drives are verified in full before the original is removed.

### Known limitations

- `0.1.0-alpha.1` and `alpha.2` have no updater; install 0.1.0 by hand once.
- Builds aren't signed with a paid certificate, so the first launch shows a
  system warning.
- Folders on a different drive from the drawer can't be moved as a whole.
- Linux isn't supported yet.

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

[Unreleased]: https://github.com/acltabontabon/scuttle/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/acltabontabon/scuttle/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/acltabontabon/scuttle/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/acltabontabon/scuttle/compare/v0.1.0-alpha.2...v0.1.0
[0.1.0-alpha.2]: https://github.com/acltabontabon/scuttle/compare/v0.1.0-alpha.1...v0.1.0-alpha.2
[0.1.0-alpha.1]: https://github.com/acltabontabon/scuttle/releases/tag/v0.1.0-alpha.1
