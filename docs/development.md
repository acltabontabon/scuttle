# Development

## Prerequisites

- **Rust** 1.82 or newer — [rustup.rs](https://rustup.rs)
- **Node** 20 or newer
- **macOS**: Xcode Command Line Tools (`xcode-select --install`)
- **Windows**: Microsoft Visual Studio C++ Build Tools, and the WebView2
  runtime (already present on Windows 11 and up-to-date Windows 10)

Users of a released build need none of this — Tauri bundles the runtime and
uses the system webview.

## Day to day

```sh
npm install

npm run app:dev       # the app, with hot reload on the frontend
npm run dev           # just the Vite server (for /preview.html)

npm test              # frontend tests (vitest)
npm run typecheck     # TypeScript
npm run rust:test     # Rust unit + integration tests
npm run rust:clippy   # Rust lints, warnings denied
npm run rust:fmt      # rustfmt

npm run app:build     # installable artifact for the current platform
```

Changing Rust restarts the backend automatically. Changing TypeScript or CSS
hot-reloads.

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
Developer mode → Dry run
```

Classifies everything and changes nothing. Prints what would be quarantined,
what would be reviewed, what would merely be surfaced, and the evidence behind
each — with full paths, which is the only place they appear.

It also tells you when the ground truth was missing: an unreadable process
list, or no installed applications discovered. In that state ghost detection
cannot be trusted, and knowing that up front saves debugging the wrong thing.

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

No test touches a real home directory. If you need a new tree, extend
`src-tauri/tests/fixtures.rs`.

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
  evidence/              observations → verdicts
  safety/                paths, protected table, action gate
  quarantine/            move, restore, remove
  storage/               SQLite behind a repository
  platform/              macOS, Windows, and the fixture platform
  space/                 the storage explanation

src-tauri/tests/         end-to-end, over fixture filesystems
```
