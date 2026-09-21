# Updates

Scuttle can update itself, and only ever when asked. It looks for a newer
version on its own (a switch in Settings turns that off), says so quietly when
there is one, downloads only when you choose to, and restarts only when you
choose to. This document is how that works, how to set it up, and how to test
it. For what a release contains and how to cut one, see
[`releasing.md`](releasing.md).

## What a person sees

- A small chip in the header — *Update available · 0.1.0-alpha.3* — that opens
  a panel with the installed and the available version, the release notes, and
  **Download** and **Later**. It never covers anything.
- *Later* stops the notice for that version for the rest of the run. A newer
  version, or checking by hand, brings it back. A background check that fails
  says nothing at all.
- Download progress is a thin gauge. When the server does not say how large the
  download is, the gauge travels instead of filling, and there is no percentage
  to be wrong about.
- Once downloaded and checked: **Update and restart** and **Later**, with the
  plain statement that installing closes Scuttle and opens the new version.
- Settings → About has the same information and the same buttons, the
  *Automatically check for updates* switch, and *Check for updates*, which says
  what happened whether it found something or not.

## How it works

```
   launch ──15 s──▶ check ──(daily)──▶ check …
                       │
        read  <endpoint>/<channel>.json   (HTTPS, a few hundred bytes)
                       │
       newer, and allowed on this channel?
              │                         │
             yes                        no ──▶ "up to date"
              ▼
        Available ── you: Download ──▶ Downloading ──▶ (signature checked) ──▶ Ready
                                                                                 │
                         you: Update and restart ◀───────────────────────────────┘
                                     │
                    claim the operation gate ── refused if a move, restore,
                                     │           delete or rummage is running
                                     ▼
              stop background work · fold the database's log into the file
                                     ▼
                    replace the application · relaunch
```

Everything to the left of the last three lines happens in Rust, asynchronously,
without touching the UI thread and without waiting on any file operation. The
webview holds no update logic and has **no updater permission** in its
capability file: it asks Scuttle's own commands (`update_status`,
`check_for_update`, `download_update`, `install_update`, `dismiss_update`) and
draws the snapshot they return.

The code is `src-tauri/src/updater/` (state machine, channel policy, the
plugin boundary), `src-tauri/src/commands/updates.rs` (the five commands), and
`src/features/updates/` (the chip, the panel, the phrasing).

### States

Idle, checking, up to date, available, downloading, ready, installing, and
unavailable (this copy cannot update: not an installed copy, or no signing key
compiled in). A failure is a note beside the state Scuttle fell back to, not a
state of its own: a failed download leaves the update **available**, one click
from being tried again; a failed install does the same, with the downloaded
package discarded; a failed check leaves whatever was known before. Overlapping
requests — a second check during a check, a second download during a download,
a second install during an install — are absorbed, not queued.

### Protecting file operations

The one rule: **installing never interrupts a move, restore, delete or
rummage, and nothing can start once installing has begun.**

It is enforced by the same operation gate that already makes every file
operation one-at-a-time, not by a flag in the interface and not by a second
"is Scuttle idle?" variable that could disagree with the first:

1. **Install claims the gate** (`AppState::begin_install`). The claim *is* the
   commitment. There is no window between "is it idle?" and "start installing"
   because they are one step under one lock; an operation that starts a
   microsecond earlier holds the gate and the install is refused, one that
   starts a microsecond later is refused as busy.
2. If something holds the gate, the install is refused with words naming it —
   *Scuttle can't restart while it's moving files into the Drawer. Try again once
   that finishes.* — and the update stays ready. Finishing the work never
   installs by itself.
3. A **background check** is the one holder that stands aside, exactly as it
   does for anything a person asks for. A rummage a person started is *not*
   thrown away: it blocks the install like any other work.
4. With the gate held, the background scheduler is stopped, the database's
   write-ahead log is folded into the file, and the last snapshot is sent —
   **before** the application is replaced, because on Windows the plugin's
   `install()` ends the process once the installer has started.
5. While installing, a Quit request is deferred rather than obeyed, so the
   application is never torn apart halfway through being replaced. The install's
   own restart is let through: on macOS Tauri routes it through the same exit
   request, tagged with a restart code, and deferring that would leave Scuttle
   stuck in *installing* (`updater::defers_exit`, tested).
6. If the install fails and the process is still running, the gate is released,
   the scheduler restarts and the update goes back to *available*.

The tests for this (`src-tauri/tests/updater.rs`) run against the real gate,
including a test that hammers file operations and installs from several
threads at once and asserts that they never hold the gate together.

### What is not persisted, on purpose

A downloaded update is held in memory only. After a restart it is found again
and downloaded again, which costs a few megabytes and means *ready to install*
is never shown for a file nothing has re-verified.

## Channels

There is no channel setting and no channel switcher. An installation's channel
follows from its own version:

| Installed | Channel | Is offered |
| --- | --- | --- |
| `0.1.0-alpha.2` (any prerelease) | **alpha** | the newest release of any kind — a newer alpha, or the stable release it led up to |
| `0.1.0` (no prerelease) | **stable** | only newer **stable** releases |

Nobody is ever offered something that is not newer than what they have, and
nobody is ever downgraded. Versions are compared by semantic-version
precedence, never as strings: `alpha.10` is newer than `alpha.9`, `0.10.0` is
newer than `0.9.0`, and `0.1.0` is newer than every `0.1.0-alpha.N`. Build
metadata is ignored. The policy is `src-tauri/src/updater/channel.rs`, is
applied again by the state machine on whatever the plugin returns, and has the
same rules in `scripts/update-manifest.mjs` for deciding what the release
workflow may publish.

While every release is an alpha this means: alpha users receive alphas, and
when `0.1.0` ships alpha users receive it and stable users, of whom there are
none yet, receive it as their first update.

## Where the manifests live

`releases/latest/download/…` on GitHub ignores prereleases, and every Scuttle
release so far is a prerelease, so it cannot serve alpha users. Instead there
is one rolling GitHub release, `updater-channels`, that holds nothing but two
small files, and the application reads whichever is its own:

```
https://github.com/acltabontabon/scuttle/releases/download/updater-channels/stable.json
https://github.com/acltabontabon/scuttle/releases/download/updater-channels/alpha.json
```

The address is in `src-tauri/tauri.conf.json` as
`…/updater-channels/{{channel}}.json`; Scuttle fills in the channel. No server
of our own is involved. `updater-channels` is a prerelease, never "latest", and
says on its page that it is not a download; the website's download button skips
it.

Each real release also carries the manifest it was built with, as `latest.json`
beside its installers. That copy never changes; the channel pointers do, but
only ever to something newer (or, with `force`, on purpose — see *Rolling
back*).

### Manifest structure

```json
{
  "version": "0.1.0-alpha.3",
  "notes": "### Added\n- …",
  "pub_date": "2026-09-21T12:00:00.000Z",
  "platforms": {
    "darwin-aarch64": {
      "signature": "<contents of Scuttle-…-macos-apple-silicon.app.tar.gz.sig>",
      "url": "https://github.com/acltabontabon/scuttle/releases/download/v0.1.0-alpha.3/Scuttle-0.1.0-alpha.3-macos-apple-silicon.app.tar.gz"
    },
    "darwin-x86_64":  { "signature": "…", "url": "…/Scuttle-0.1.0-alpha.3-macos-intel.app.tar.gz" },
    "windows-x86_64": { "signature": "…", "url": "…/Scuttle-0.1.0-alpha.3-windows-x64-setup.exe" }
  }
}
```

`notes` is the release's section of `CHANGELOG.md`. Exactly these three
platforms, no more and no fewer: they are the whole supported matrix.

## Release artifacts

For each release, besides the installers people download:

| File | Purpose |
| --- | --- |
| `Scuttle-<v>-macos-apple-silicon.app.tar.gz` and `.sig` | update package, Apple silicon |
| `Scuttle-<v>-macos-intel.app.tar.gz` and `.sig` | update package, Intel |
| `Scuttle-<v>-windows-x64-setup.exe.sig` | signature for the installer, which is also the Windows update package |
| `latest.json` | this release's manifest |

Update packages are `.app.tar.gz` on macOS because the updater replaces the
application bundle in place and cannot use a disk image. On Windows the NSIS
installer is run in *passive* mode, per-user, with no elevation prompt.

## The three kinds of signing

They are separate, they are not substitutes for one another, and only the first
is new.

1. **Update signing (Tauri / minisign)** — what this document is about. A
   keypair *we* generate. The private half signs each update package in CI; the
   public half is compiled into every copy of Scuttle, which refuses any package
   that does not verify against it, and refuses one whose signature does not say
   it was made for the version the manifest announces (`requireSignedVersion`).
   This is what stops a compromised network, a mis-uploaded file, or a tampered
   manifest from installing anything.
2. **macOS code signing / notarization.** Unchanged: builds are ad-hoc signed
   (`"signingIdentity": "-"`), not signed with a Developer ID and not
   notarized. The updater does not change that and cannot substitute for it.
3. **Windows code signing (Authenticode).** Unchanged: the installer is
   unsigned.

Nothing here bypasses an operating-system control. Because the application
downloads the package itself, no browser marks it as quarantined, so an update
does not trigger the Gatekeeper or SmartScreen prompt a first manual install
does — that is a property of how the file arrives, not something Scuttle
circumvents, and if it ever changes the prompts will simply appear. There is no
privileged helper or background service; on macOS Scuttle refuses to update an
installation whose folder it cannot write to rather than asking the system for
administrator rights, and says to install by hand.

**One consequence to test before announcing an update.** An ad-hoc signature
identifies a build by its own hash, so it differs for every version. macOS may
therefore treat the updated application as a new one for privacy permissions
that were granted to the old one (for example Full Disk Access or per-folder
access). This is expected, and it is one of the things the end-to-end test below
checks; if it turns out to matter, the fix is a Developer ID, not an updater
change.

## Setting it up (once)

**1. Generate the update signing keypair**

```sh
npm run tauri signer generate -- -w ~/.tauri/scuttle-updater.key
```

Choose a password. This prints the **public key** and writes two files: the
private key (`scuttle-updater.key`) and `scuttle-updater.key.pub`.

**2. Back it up.** Store the private key file *and* its password somewhere
offline and durable (a password manager plus an encrypted copy elsewhere). If
either is lost, no future release can be signed with this key, and because the
public key is compiled into every installed copy, **no installed copy will ever
accept an update signed with a different one** — every user would have to
install by hand again. Losing this key is the one unrecoverable mistake in this
system. Never commit it, paste it into an issue, or put it in a workflow file.

**3. Put the public key in the application.** Paste the contents of
`scuttle-updater.key.pub` (the whole base64 line) into
`src-tauri/tauri.conf.json` as `plugins.updater.pubkey`, replacing the
`UNSET: …` placeholder, and commit it. The public key is public.

**4. Give CI the private key.** In the repository: *Settings → Secrets and
variables → Actions → New repository secret*:

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | the contents of the private key file, pasted as the secret's value |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the password from step 1 |

`node scripts/update-manifest.mjs check-config` reports whether the checkout is
ready, and the release workflow runs it first: a placeholder or malformed key
stops a release before anything is built.

The secrets are read by exactly one step, the build, and only when update
artifacts are being built. They are never printed, never written to disk by the
workflow, and never part of any published file. Nothing in the frontend, the
Rust code or the release assets contains the private key.

**5. Allow the workflow to write releases** (it already does, for the existing
release pipeline): the `update-channels` workflow uses the same
`contents: write` token to maintain the `updater-channels` release.

## What the release workflow does

`release.yml`, on a version tag (it fails, and publishes nothing, if any check
fails):

1. **validate** — the tag, version and changelog agree; the signing secret
   exists; the public key in `tauri.conf.json` is real, the endpoint is HTTPS
   with a `{{channel}}` and `requireSignedVersion` is on.
2. **checks** — everything CI runs.
3. **build** — per target, with the updater overlay
   (`src-tauri/tauri.updater.conf.json` → `createUpdaterArtifacts`): the
   installers **and** signed update packages. Each signature is verified on the
   machine that made it, against the public key in the application and bound to
   this version, and the macOS archive is opened to confirm it holds the right
   version of the application.
4. **publish** — every expected file and `.sig` must exist. Then
   `latest.json` is built from the signatures and **verified against the
   artifacts themselves**: the whole matrix, no extras, every address the one
   this release will have, every signature valid for the exact bytes about to be
   uploaded. Only then is the draft created and made public.
5. **channels** (`update-channels.yml`) — *after* the release is public:
   downloads `latest.json` and every artifact from the public addresses
   installed copies will use, verifies them all again, and only then moves the
   pointers, and only to something newer. It reads them back afterwards.

So a new manifest is never visible before its files are, and a release with a
missing platform, a missing signature, a signature from the wrong key or for
the wrong version never becomes offerable.

CI that is not a release (`ci.yml`, the *Build desktop* workflow run by hand
without `updater`) never touches the key and builds exactly what it did before.

### If the `channels` job fails

The release is published and its installers work, but nobody is being offered
the update yet. Fix the cause and run **Actions → Update channels → Run
workflow** with the tag. No rebuild.

### Rolling back

Run **Update channels** with the tag of the release to point at and `force`
ticked. New checks then read that release's manifest. This does *not* downgrade
anybody already on the bad release — the updater never installs an older
version — so a bad release is fixed by publishing a newer one, and rolling back
the pointer only stops further people receiving it.

## Data across an update

Settings, findings, drawer records and history live in Scuttle's data directory
(`~/Library/Application Support/Scuttle`, `%LOCALAPPDATA%\Scuttle`), which an
update does not touch: the macOS package replaces only the `.app`, and the
Windows installer is per-user and never removes that directory.

- **New setting.** `auto_check_updates` is a new field in the settings blob; the
  blob is `serde(default)`, so a database written by an older version loads with
  it on. There is a test for exactly that.
- **Schema migrations** run at the next launch, each in its own transaction,
  the same as for any release. Nothing about updating changes them.
- **Before replacing anything**, Scuttle folds the write-ahead log into the
  database file, so the file the new version opens is whole.
- **No downgrade guard.** `migrations::apply` does not refuse a database from a
  *newer* schema. The updater never downgrades, so it cannot cause this, but a
  person who installs an older build by hand over a newer database gets no
  warning. Not changed here; worth a guard before the first schema change that
  is not backward-compatible.
- **Nothing is running.** No move, restore or delete is in flight when the
  application is replaced — that is what the gate is for — so there is no
  half-finished journal for the new version to reconcile because of an update.

### Rollback

There is **no automatic rollback**. On macOS the plugin moves the old
application aside while the new one is put in place, but that is the plugin's
own arrangement, its cleanup is not something Scuttle has verified, and it is
not claimed as a feature. If an install fails, Scuttle stays running and the
update goes back to *available*; if a new version is bad, publish a newer one.

## Testing

### What is automated

| Risk | Where |
| --- | --- |
| state transitions, retry after failure, failed background vs manual check | `src-tauri/tests/updater.rs` |
| concurrent checks, downloads and installs collapse to one | same |
| install blocked by each kind of file operation, named in the message, and by a rummage; goes ahead when it ends; does not happen by itself | same |
| a background check stands aside for an install | same |
| **no file operation can start once an install is committed**; and a stress test that file operations and installs never hold the gate together | same |
| a failed install leaves the gate free and the app usable, and can be retried | same |
| a package that fails its signature is never *ready* and never installable | same (mock), `updater/tauri_backend.rs` (mapping of the plugin's errors) |
| malformed metadata, no entry for this platform, offline, timeout | same |
| channel and version selection | `updater/channel.rs`, `scripts/update-manifest.test.mjs` |
| manifests: full matrix, https, right addresses, right key, bound to the version, valid for the file, no downgrade of a pointer | `scripts/update-manifest.test.mjs` (real signatures from real Tauri keys) |
| settings default and back-compat, checkpoint | `src-tauri/src/storage/mod.rs` |
| what the interface shows for each state | `src/features/updates/*.test.ts` |

`npm run check` runs all of it.

**What these do not prove.** The updater plugin is mocked at its boundary in the
Rust tests, so they show that Scuttle decides correctly *given* what the plugin
says. They do not show that the plugin downloads, verifies or replaces
anything, that the relaunch works, or that Windows behaves as documented. Only
the end-to-end test does.

### End-to-end, with two signed builds

Do this on **macOS** and on **Windows**, with a throwaway key so nothing real is
involved. It publishes nothing.

**Set up (each OS).**

1. Generate a throwaway key: `npm run tauri signer generate -- -w e2e.key`
   (any password). Note the public key.
2. Make an overlay, `e2e.conf.json`, outside the repository:

   ```json
   {
     "plugins": {
       "updater": {
         "pubkey": "<the throwaway public key>",
         "endpoints": ["http://127.0.0.1:8787/{{channel}}.json"],
         "dangerousInsecureTransportProtocol": true
       }
     }
   }
   ```

3. Choose two versions on the same channel, e.g. **1.** `0.1.0-alpha.90` and
   **2.** `0.1.0-alpha.91` (`npm run version:set -- <v>` before each build; do not
   commit).

**Build both** (macOS shown; the target is `aarch64-apple-darwin` or
`x86_64-apple-darwin`, and on Windows `x86_64-pc-windows-msvc` with
`--bundles nsis`):

```sh
export TAURI_SIGNING_PRIVATE_KEY="$(cat e2e.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=…
npm run version:set -- 0.1.0-alpha.90
npm run tauri build -- --target aarch64-apple-darwin --bundles app,dmg \
  --config src-tauri/tauri.apple-silicon.conf.json \
  --config src-tauri/tauri.updater.conf.json --config e2e.conf.json
# keep the .dmg / installer of this build, then:
npm run version:set -- 0.1.0-alpha.91
# …build again the same way; keep the .app.tar.gz and .sig of this one
```

(On Windows PowerShell: `$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content e2e.key -Raw`.)

**Serve build 2.** Put the second build's update package and its `.sig`, named
as the release workflow names them (`Scuttle-0.1.0-alpha.91-macos-apple-silicon.app.tar.gz`
and `.sig`, or `…-windows-x64-setup.exe` and `.sig`), in a folder, then:

```sh
node scripts/update-manifest.mjs build 0.1.0-alpha.91 <folder> \
  --base-url http://127.0.0.1:8787 > <folder>/alpha.json
python3 -m http.server 8787 --directory <folder>
```

The manifest builder needs a signature for all three platforms; for a
single-OS test, copy one platform's `.sig` under the other names — the updater
only reads its own platform's entry.

**Run the checks** on build 1, installed the way a user would (open the `.dmg`
and drag to Applications; run the Windows installer):

| # | Do | Expect |
| --- | --- | --- |
| 1 | Launch. Wait ~15 s. Settings → About. | *Update available · 0.1.0-alpha.91* in the header; version 0.1.0-alpha.90 installed; the release notes shown as text. Launching was not delayed. |
| 2 | Click *Later*. Re-check by hand. | Chip disappears on *Later*; a manual check brings it back. |
| 3 | Click **Download**. Keep using Scuttle. | A gauge (a travelling one if the size is unknown); the window stays responsive; nothing installed. |
| 4 | Before installing, make some state: change a setting, put an item in the Drawer, note the history. | — |
| 5 | Start a **large move** (several GB, or a slow volume) and, while it runs, click **Update and restart**. | Refused with *Scuttle can't restart while it's moving files into the Drawer…*; the move continues and completes; the update stays ready; no restart. |
| 6 | Try to start another move / restore during the refusal. | Refused as busy, as always. |
| 7 | When the move has finished, nothing restarts on its own. Click **Update and restart**. | Scuttle closes and reopens as 0.1.0-alpha.91 (Windows: a small installer progress window first). |
| 8 | After relaunch. | Settings, the Drawer's item and the history are all still there; the version in About is the new one; no *update available*. |
| 9 | On macOS: check that Files/Full Disk Access style permissions you had granted are still granted. | Note the result; see *The three kinds of signing*. |
| 10 | Failure paths (rebuild the manifest to provoke): edit one character of the `.sig`; or serve a package that was signed with a different key; or stop the server mid-download. | *did not pass Scuttle's signature check… Nothing was installed* / *The download stopped partway…*; Scuttle keeps running on the old version; retry works once fixed. |
| 11 | Turn *Automatically check for updates* off, restart. | No check occurs; *Check for updates* still works. |
| 12 | Install build 2 by hand over the update. | Still runs; data intact. |

### A test release

To exercise the real pipeline without announcing anything, after the setup
above, push a prerelease tag (say `v0.1.0-alpha.91`) from a branch you are
willing to see published, or run **Build desktop** from the Actions tab with
`updater` ticked to check the build, signing and archive steps without
publishing. Then install `v0.1.0-alpha.90` from the releases page and let it
update itself through the real `updater-channels` release.

## What has and has not been verified

Reported honestly, because the difference matters.

**Executed while building this** (on macOS/arm64):

- The Rust suite (354 unit tests, plus the integration suites, including 33
  updater tests and the gate stress test), with clippy denying warnings and
  `cargo fmt --check`.
- The frontend type-check, lint and unit tests (197 in all, which include the
  manifest script's 41), run against **real** signatures and keys made with the Tauri CLI; the
  script's native minisign check against a real `tauri signer sign` output and
  against the signature the bundler itself produced (which records
  `version:…`, so `requireSignedVersion` is compatible with how releases are
  built).
- `actionlint` on every workflow (syntax and shell only).
- A real `tauri build` with a throwaway key and the updater overlay: it produced
  `Scuttle.app.tar.gz` and its `.sig`; the archive holds `Scuttle.app/` with the
  expected version; the signature verifies against the key and not against
  another.
- A **discovery run of the real bundled application**: build 1 was built against
  a local server and started with a throwaway `HOME` (so it could not touch real
  data). About 15 seconds after launch it requested `alpha.json` — the channel
  chosen from its own `-alpha` version, the `{{channel}}` endpoint filled in — and
  its debug log showed `checking` then `available` for the newer version served
  by the local manifest. That confirms the plugin is configured correctly in a
  packaged app and that the check runs without delaying startup.
- The chip, panel and their states in the design workbench
  (`/preview.html?update=<available|dismissed|downloading|downloading-unknown|ready|blocked|installing|failed|current>`),
  looked at in the browser pane. Settings → About could not be rendered there
  (it needs the core), so those rows are checked by the type-checker and by
  the shared `describe()` tests only.

**Not executed, and needing another OS or an external setup:**

- **Any Windows build**, the passive NSIS update, and the process exit and
  relaunch there. The Windows-only code paths compile only in CI's Windows job.
- **Download, signature check, install and relaunch of a real update in the
  packaged app**, and the file-operation gating there, i.e. steps 3 to 12 of the
  table above. The discovery run stopped at *available*: driving the webview's
  buttons needs a GUI session, and the OS's accessibility scripting could not see
  the window. So the plugin's download, its signature verification, its
  replacement of the bundle, the relaunch (including the single-instance
  hand-off), and *installing refused while a move runs* have been tested only
  against mocks and the real gate, not end to end.
- **The GitHub workflows.** They are syntax- and shell-checked but have not run:
  that needs the signing secret, a real public key and a tag.
- Intel macOS (cross-compiled in CI, never run).
- The `updater-channels` release, which does not exist until the first release
  that goes through the new pipeline.
- Whether macOS privacy grants survive an ad-hoc-signed update (step 9).

## Getting existing installs onto this

Builds released before this feature contain no updater. Nothing can update
them from the inside. **Everyone on `v0.1.0-alpha.1` and `v0.1.0-alpha.2`
needs to install the first release that includes updates by hand, once**, from
the releases page. After that they update themselves. The release notes say so.

## Remaining distribution requirements

- A real update signing key, backed up (above). Nothing ships without it.
- Optional and unchanged by this work: an Apple Developer ID plus notarization,
  and a Windows code-signing certificate. Until then first installs still meet
  Gatekeeper and SmartScreen, per [`packaging.md`](packaging.md). If you add
  them, the update signing key stays separate and stays.
- If the repository is ever made to require approval for workflow runs from
  environments, or the secrets are moved into an environment, `build-desktop`'s
  `secrets:` mapping in `release.yml` is the one place to update.
