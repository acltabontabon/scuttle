# Changelog

All notable changes to Scuttle are documented here, newest first.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
Scuttle uses [semantic versioning](https://semver.org/spec/v2.0.0.html). While the
major version is 0, a minor bump may change behaviour.

## [Unreleased]

Nothing yet.

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

[Unreleased]: https://github.com/acltabontabon/scuttle/compare/v0.1.0-alpha.1...HEAD
[0.1.0-alpha.1]: https://github.com/acltabontabon/scuttle/releases/tag/v0.1.0-alpha.1
