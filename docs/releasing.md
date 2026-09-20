# Releasing

Cutting a release is: write down what changed, set the version, push a tag.
Everything after the tag is automatic, and everything before it is a commit
like any other. No workflow ever bumps a version or writes a commit — the tag
is built exactly as it was pushed.

There are **no signing credentials to arrange.** Releases are ad-hoc signed on
macOS and unsigned on Windows, deliberately; see
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

`.github/workflows/release.yml`, in four steps, each of which stops the release
if it does not hold:

1. **Validate** — the tag matches the version in all five files, the changelog
   has a section for it, and the tag is not one that has already been
   published.
2. **Checks** — everything CI runs, on the tagged commit.
3. **Build** — three installers, on native runners, each inspected where it was
   built: the `.dmg` is mounted and the application inside it is read back
   (version, identifier, icon, architecture, ad-hoc signature), and the Windows
   installer's size and version resource are checked.
4. **Publish** — a draft gathers the assets, and becomes a release only once
   every expected file is present.

Expect, for version `X`:

| Asset | |
| --- | --- |
| `Scuttle-X-macos-apple-silicon.dmg` | arm64, minimum macOS 11.0 |
| `Scuttle-X-macos-intel.dmg` | x86_64, minimum macOS 10.15 |
| `Scuttle-X-windows-x64-setup.exe` | NSIS, per-user, unsigned |
| `*.sha256` beside each, and `SHA256SUMS.txt` | |

The two macOS builds both come off the Apple Silicon runner: Apple ships both
slices' SDKs on every Mac, so the Intel build is an ordinary supported cross
compile rather than a second machine. There is no Linux download, because the
Linux platform layer is a stub that knows nothing about a Linux system.

## Before announcing it

The pipeline proves the installers are well-formed. It does not prove they
install. Download the `.dmg` from the release page — the real download, not a
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

The release workflow uses the automatic `GITHUB_TOKEN` and nothing else. No
secrets need to be configured for a release, and none are required for
packaging — that is the whole point of the distribution mode described in
[`packaging.md`](packaging.md).

Scuttle publishes no Docker image, and there is no Docker Hub account to set
up. [`docker.md`](docker.md) says why.
