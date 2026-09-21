# Releasing

Cutting a release is: write down what changed, set the version, push a tag.
Everything after the tag is automatic, and everything before it is a commit
like any other. No workflow ever bumps a version or writes a commit — the tag
is built exactly as it was pushed.

There is **one signing credential to arrange**, once: the updater key that lets
installed copies of Scuttle verify an update. Setting it up, backing it up and
what it is (and is not) are in [`updates.md`](updates.md); without it the
release workflow refuses to start. Apart from that, releases are ad-hoc signed
on macOS and unsigned on Windows, deliberately; see
[`packaging.md`](packaging.md). Nothing here waits on an Apple Developer
account or a Windows certificate.

## The short version

```sh
# 1. describe the release in CHANGELOG.md (see below), then
npm run version:set -- 0.2.0
npm run check
git commit -am "Scuttle 0.2.0"
git push
# wait for CI on main to go green, then
git tag v0.2.0
git push origin v0.2.0
```

## 1. Write the changelog entry

Move what has accumulated under `## [Unreleased]` into a new dated section:

```markdown
## [Unreleased]

Nothing yet.

## [0.2.0] - 2026-10-04

### Added
- ...

### Fixed
- ...

### Known limitations
- ...
```

and add the compare link at the foot of the file, next to the others.

Write it for somebody deciding whether to download this, not for somebody
reading a diff. Describe only what is actually implemented, keep the material
limitations in — the things people will otherwise discover for themselves —
and use today's date, the day you are cutting it.

This section becomes the release body. There is nowhere else to write release
notes, which is the point: `scripts/release-notes.mjs` reads this file, adds
the download table, the Gatekeeper and SmartScreen instructions and the
checksums, and that is the whole release page. Preview it at any time:

```sh
node scripts/release-notes.mjs 0.2.0 | less
```

A tag whose version has no changelog section fails validation and publishes
nothing.

## 2. Set the version

```sh
npm run version:set -- 0.2.0
```

The version lives in five places — `package.json`, `package-lock.json` twice,
`src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `tauri.conf.json` — and only
the Cargo one reaches the About screen, so drift is invisible until someone
reports the wrong number. That command writes all five; `npm run version:check`
asserts they agree, and runs first in CI.

Semantic versions, tagged `vX.Y.Z`. A prerelease is `0.2.0-rc.1`, tagged
`v0.2.0-rc.1`: the workflow marks it as a prerelease on GitHub and never makes
it the latest release. While the major version is 0, a minor bump is allowed to
change behaviour.

Do not jump to 1.0.0 to mark an occasion.

## 3. Run the checks locally

```sh
npm run check
```

Version consistency, typecheck, lint, frontend tests, frontend build, `cargo
fmt --check`, Clippy with warnings denied, and the Rust suite — the same things
CI runs, minus the other platform. Every filesystem test builds its own
temporary tree; none of them go near your home directory.

To exercise packaging before committing to a tag, either run the
[**Build desktop**](../.github/workflows/build-desktop.yml) workflow from the
Actions tab (it takes a branch or commit, builds all three installers, inspects
each one and uploads them as workflow artifacts — and publishes nothing), or
build one locally:

```sh
npm run tauri build -- --target aarch64-apple-darwin --bundles dmg \
  --config src-tauri/tauri.apple-silicon.conf.json
```

## 4. Commit, push, and let CI settle

Push the version bump to `main` and wait for CI. The release run repeats these
checks on the tagged commit, so a red `main` only means finding out twice.

## 5. Tag

```sh
git tag v0.2.0
git push origin v0.2.0
```

## What the tag sets off

`.github/workflows/release.yml`, in five steps, each of which stops the release
if it does not hold:

1. **Validate** — the tag matches the version in all five files, the changelog
   has a section for it, the tag is not one that has already been published,
   and the updater is configured (the signing secret exists and the public key
   in `tauri.conf.json` is a real one).
2. **Checks** — everything CI runs, on the tagged commit.
3. **Build** — three installers, on native runners, each inspected where it was
   built: the `.dmg` is mounted and the application inside it is read back
   (version, identifier, icon, architecture, ad-hoc signature), and the Windows
   installer's size and version resource are checked. Alongside them, the
   signed update packages: every signature is verified against the public key
   in the application, bound to this version, and the macOS archive is opened
   to confirm it holds this version of the application.
4. **Publish** — the update manifest (`latest.json`) is built and checked
   against the artifacts themselves, then a draft gathers the assets, and it
   becomes a release only once every expected file is present.
5. **Channels** — only after the release is public, and its files are fetched
   back from the addresses installed copies use and verified again, the
   `updater-channels` pointers are moved (`update-channels.yml`). Until then
   nobody is offered the new version. If this step fails, the release stands;
   rerun **Update channels** from the Actions tab. See
   [`updates.md`](updates.md) for what the channels are.

Expect, for version `X`:

| Asset | |
| --- | --- |
| `Scuttle-X-macos-apple-silicon.dmg` | arm64, minimum macOS 11.0 |
| `Scuttle-X-macos-intel.dmg` | x86_64, minimum macOS 10.15 |
| `Scuttle-X-windows-x64-setup.exe` | NSIS, per-user, unsigned |
| `Scuttle-X-macos-apple-silicon.app.tar.gz`, `Scuttle-X-macos-intel.app.tar.gz` | update packages for the in-app updater |
| a `.sig` beside each update package, and beside the Windows installer | Tauri update signatures |
| `latest.json` | this release's update manifest |
| `*.sha256` beside each download, and `SHA256SUMS.txt` | |

The two macOS builds both come off the Apple Silicon runner: Apple ships both
slices' SDKs on every Mac, so the Intel build is an ordinary supported cross
compile rather than a second machine. There is no Linux download, because the
Linux platform layer is a stub that knows nothing about a Linux system.

## Before announcing it

The pipeline proves the installers are well-formed and that the update packages
verify. It does not prove they install, or that an installed copy can update
itself: that is the end-to-end test in [`updates.md`](updates.md), which is done
by hand, with two signed builds, on each OS, before the first release that
offers updates and whenever the updater or the packaging changes.

For the installers: Download the `.dmg` from the release page — the real download, not a
local build — open it, drag Scuttle to Applications, and click through the
Gatekeeper prompt the way the README describes. If macOS says **damaged**
rather than unverified, stop: that is a packaging defect or a corrupted
download, not the expected prompt, and the release should be pulled rather than
explained away.

## When something goes wrong

**Rerunning a failed release is safe and expected.** Assets are assembled on a
draft, which is not on the releases page and is not "latest", so a run that
died halfway leaves nothing half-published and the next run picks the same
draft up. Reruns replace a partial upload rather than duplicating it.

**A tag that is already published will not be rebuilt.** Validation refuses it,
rather than silently replacing binaries people may already have downloaded. If
a published release is wrong, publish a new version.

**A tag pushed by mistake, before anything was published**, can be deleted and
redone:

```sh
gh release delete v0.2.0 --yes      # only if a draft was created
git push --delete origin v0.2.0
git tag -d v0.2.0
```

**Version does not match the tag.** Either the tag or the files are wrong; the
error says which values it saw. Fix with `npm run version:set`, commit, and tag
again.

**A build failed on one platform.** Rerun the failed jobs from the Actions tab.
Nothing is published until all three succeed, so there is no partial release to
clean up. A macOS build that fails at `codesign --verify` is a real defect and
not something to retry past.

**Nothing at all happened when I pushed the tag.** The trigger only matches
`vX.Y.Z` and `vX.Y.Z-something`. `v0.2` and `0.2.0` do not match.

## Credentials

The release workflow uses the automatic `GITHUB_TOKEN`, and **two repository
secrets** for the updater: `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. They sign update packages; they are not an
Apple or Windows code-signing credential, and packaging the installers still
needs neither of those — that part of the distribution mode described in
[`packaging.md`](packaging.md) is unchanged. Generating, backing up and
installing the key is in [`updates.md`](updates.md#setting-it-up-once). Only the
build step can read the key, and only for a release (or a hand-run *Build
desktop* with `updater` ticked); ordinary CI never sees it.

Scuttle publishes no Docker image, and there is no Docker Hub account to set
up. [`docker.md`](docker.md) says why.
