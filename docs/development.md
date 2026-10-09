# Development

## Prerequisites

- **Rust** 1.89 or newer — [rustup.rs](https://rustup.rs). The locked dependencies
  require this minimum; CI pins the exact toolchain in `.github/workflows/ci.yml`.
- **Node** 22.12 or newer
- **macOS**: Xcode Command Line Tools (`xcode-select --install`)
- **Windows**: Microsoft Visual Studio C++ Build Tools, and the WebView2
  runtime (already present on Windows 11 and up-to-date Windows 10)

Users of a released build need none of this — Tauri bundles the runtime and
uses the system webview.

## Day to day

```sh
npm ci

npm run app:dev       # the app, with hot reload on the frontend
npm run dev           # just the Vite server (for /preview.html)

npm test              # frontend tests (vitest)
npm run typecheck     # TypeScript
npm run rust:test     # Rust unit + integration tests
npm run rust:clippy   # Rust lints, warnings denied
npm run rust:fmt      # rustfmt
npm run check         # all app checks, including release-manifest tests

npm run app:build     # installable artifact for the current platform
```

Changing Rust restarts the backend automatically. Changing TypeScript or CSS
hot-reloads.

## Verifying a change

Run `npm run check` before preparing a release. It checks version consistency,
types, lint, frontend and release-manifest tests, the production frontend build,
Rust formatting, Clippy and the Rust unit and integration suites.

The frontend store tests simulate completion before the scan-start response,
rapid preference changes, failed saves and overlapping space measurements. The
Rust fixtures cover changed files, hard links, links swapped into scanned paths,
cancelled hashing, partial measurements and scan failures that must release the
operation gate. File-changing tests use temporary trees.

Run the checks on both macOS and Windows through CI. A passing local macOS run
does not verify Windows filesystem or installer behavior. After those checks,
follow the native smoke tests in [releasing.md](releasing.md), including putting
fixture files in the drawer and restoring them, and the update checks in
[updates.md](updates.md). Browser previews cannot establish those guarantees.

Some existing platform smoke tests inspect the live process list and disk
metadata. A restricted development sandbox can block those reads. Run them in
a normal local terminal or CI rather than treating an empty process list as a
successful test.

## The design workbench

With the dev server running, open
[http://localhost:5273/preview.html](http://localhost:5273/preview.html).

It mounts the real interface against fixture data, with a scene picker for
every screen and every awkward state — nothing found, a folder with save data
in it, a screenshot burst, a duplicate group — plus a light/dark switch.

This is how to work on the interface. Waiting for a real rummage to produce a
high-risk oddment so you can check its layout is not a good use of an
afternoon.

It is dev-only: `preview.html` is not an input to the production build, so
`src/dev/` never reaches a shipped bundle.

## The dry run

```
Settings → Diagnostics → Open diagnostics
```

Classifies everything and changes nothing. Prints what would be quarantined,
what would be reviewed, what would merely be surfaced, and the evidence behind
each, with full paths shown because the report was explicitly requested.
The dependency cache preview in Settings also shows full paths.

From a terminal, use `scuttle --dry-run`, or
`scuttle --dry-run --developer-debris` to include build output. These commands
classify files and print a report; they do not move files or save a scan.
Both desktop and terminal dry runs include the dependency cache preview when
its separate setting is enabled, and honor saved Keep/ignore decisions.
The desktop command runs on a blocking worker while holding the operation gate,
so cache inspection cannot freeze the window or overlap a move.

The browser workbench uses sample data. Its controls verify presentation;
filesystem operations, operating-system permissions and updater installation
need a native build. See [releasing.md](releasing.md) for the release checks.

It also tells you when the ground truth was missing: an unreadable process
list, or no installed applications discovered. In that state ghost detection
cannot be trusted, and knowing that up front saves debugging the wrong thing.

## Dependency cache preview

Settings → Dependency caches → Preview dependency caches → Inspect dependency
caches starts a foreground, read-only inspection. Enable it separately from
Developer build artefacts. It uses the shared project-folder preferences, plus
known cache locations and explicitly added custom locations. It never runs in
background checks, executes project scripts or downloads dependencies.

Regression fixtures cover queued cancellation and retry, disappearing entries,
Keep decisions inside version directories, plugin/dependency nesting, shared
lockfile references, pnpm executable blobs and recovery of legacy drawer items.
Reference indexes are built once per inspection; matching each cache entry does
not walk every project lockfile again. Metadata classification uses the existing
stat result rather than reopening each entry for its identity.

Current adapters produce kept or insufficient-evidence results only. Do not
turn incomplete coverage or modification dates into unused-dependency evidence.
Supported layouts and remaining limits are described in
[README](../README.md#dependency-cache-preview) and [safety.md](safety.md).

## Logging

```sh
SCUTTLE_LOG=scuttle_core=debug npm run app:dev
```

Paths are deliberately absent from normal-level logs. See
[privacy.md](privacy.md).

## Testing philosophy

This application moves and deletes files, so tests are written as the failure
they prevent, not as the feature they cover. Compare:

```rust
#[test]
fn sibling_directories_are_not_contained() { … }

#[test]
fn dotdot_cannot_escape_a_protected_ancestor() { … }

#[test]
fn certainty_about_a_risky_thing_is_not_permission() { … }

#[test]
fn restoring_never_overwrites_something_that_arrived_in_the_meantime() { … }
```

Three layers:

- **Unit tests** live beside the code. Path arithmetic, the protected table,
  the evidence arithmetic, each detector, the store, quarantine.
- **Integration tests** (`src-tauri/tests/`) run the whole pipeline over
  fixture filesystems built in a temp directory: a healthy system, an abandoned
  game, old installers, duplicates, dangerous paths, a developer project, a
  symlink loop.
- **Frontend tests** cover formatting, voice and colour. The voice tests check
  that the outcome copy contains no exclamation marks, no shouting and no
  manufactured urgency. The palette tests read `tokens.css` itself and assert
  every text colour clears WCAG AA against its surface in both themes — the
  light palette once shipped small text at 2.76:1.

File-changing tests use temporary fixture trees, never real user files.
If you need a new tree, extend `src-tauri/tests/fixtures.rs` or the dependency
cache fixture harness. The platform smoke tests make the narrow read-only
host checks described above.

## Testing platform assumptions

Some behaviour can only be verified on the platform it targets, so those tests
are `#[cfg(target_os = ...)]` and run in CI on both. Assumptions worth
re-checking when you change the platform layer:

| Assumption | macOS | Windows |
| --- | --- | --- |
| Filesystem is case-insensitive | Yes (APFS default) | Yes |
| Paths pass through symlinks above home | `/var`, `/tmp` | `C:\Users\Public\…` junctions |
| Installed apps are discoverable | `.app` bundles + `Info.plist` | Uninstall registry, **both** the 64- and 32-bit views |
| App data lives in | `~/Library/…` | `%LOCALAPPDATA%`, `%APPDATA%`, `LocalLow` |
| Screenshots land in | Desktop by default; `defaults read com.apple.screencapture location` | `Pictures\Screenshots`, `Videos\Captures` |
| Installer formats | `.dmg`, `.pkg`, `.mpkg` | `.exe`, `.msi`, `.msix`, `.appx` |
| Reveal in file manager | `open -R` | `explorer /select,` (exits non-zero on success) |

The Windows registry detail is not a footnote: an application registered only
in the `WOW6432Node` view would look uninstalled, and Scuttle would call its
data a ghost. Both views are read.

## Project layout

```
src/                     React
  app/                   shell, state
  features/              one directory per experience
  visuals/               the creature, glyphs, category metadata
  lib/                   IPC, types, formatting
  styles/                tokens, base
  dev/                   the design workbench (dev-only)

src-tauri/src/           Rust
  commands/              IPC surface, app state, dry run
  scanning/              traversal and orchestration
  detectors/             one file per detector
  dependency_cache/      bounded Maven/Gradle/Node inventory and retention policy
  evidence/              observations → verdicts
  safety/                paths, protected table, action gate
  quarantine/            move, restore, remove
  storage/               SQLite behind a repository
  platform/              macOS, Windows, and the fixture platform
  space/                 the storage explanation
  background/            the optional scheduler: when to look, what to say
  tray.rs                the menu bar / system tray icon and its menu
  window.rs              show, hide, quit

src-tauri/tests/         end-to-end, over fixture filesystems
```

Background mode has one rule worth knowing before touching it: all the
judgement about *when* to look lives in `background::schedule::decide`, a pure
function of the clock, the persisted state and what the machine reports. The
thread does nothing but call it and act on the answer. Anything that needs
testing against time belongs in there, not in the thread.

The tray glyphs under `src-tauri/icons/tray/` are generated and committed:

```
node scripts/tray-icons.mjs
```

It needs no dependency beyond Node, and the build never runs it — the PNGs in
the repository are what ship.
