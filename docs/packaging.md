# Packaging

What a build actually produces, and what each platform then says about it.
[`releasing.md`](releasing.md) is the process around this; here is the thing
itself.

```sh
npm run app:build
```

Artifacts land in `src-tauri/target/release/bundle/`, or in
`src-tauri/target/<triple>/release/bundle/` when `--target` is given — which
the release workflow always does, so the two macOS architectures do not
overwrite each other.

`bundle.targets` is `["dmg", "nsis"]`: one download per platform. The `.app` is
built on the way to the `.dmg` and cleaned up afterwards, and the MSI was
dropped because an unsigned second Windows installer only makes people pick.

Users need no developer runtime. Tauri bundles what it needs and uses the
system webview — WebKit on macOS, WebView2 on Windows.

## How Scuttle is actually distributed

Without an Apple Developer ID certificate, Apple notarization credentials or a
Windows code-signing certificate. That is the intended mode, not a gap waiting
to be filled: releases are ad-hoc signed on macOS and unsigned on Windows, and
the README tells people plainly what each operating system will say. Nothing in
CI depends on a signing secret, and no job fails because one is absent.

The sections below on Developer ID signing and Windows certificates describe
what *would* change if those certificates ever existed. They are not
prerequisites for a release today.

## macOS

Produces `Scuttle.app` and a `.dmg`. The Intel build declares a minimum of
macOS 10.15; the Apple Silicon build declares 11.0, because that is where Apple
Silicon starts and claiming otherwise would be a lie in the Info.plist. That is
the entire contents of `tauri.apple-silicon.conf.json`, passed with `--config`
for the `aarch64-apple-darwin` build only.

### Ad-hoc signing, which is what releases actually use

`bundle.macOS.signingIdentity` is `"-"`. That asks the bundler for an *ad-hoc*
signature: `codesign --force -s -`, applied inside-out across the bundle, with
no certificate, no account and no secret involved.

This matters more than it sounds. On Apple Silicon, macOS refuses to execute a
binary with no valid signature at all — and an unsigned bundle downloaded from
the internet is reported as **damaged**, which is a dead end for the person who
downloaded it. An ad-hoc signature turns that into the ordinary "developer
cannot be verified" prompt, which they can get past once through System
Settings → Privacy & Security.

What ad-hoc signing is **not**: it is not a verified publisher identity, it
carries no team, and it is not notarization. It says the bundle is internally
consistent and nothing more. Never describe it as signed in the sense anyone
means by that word.

```sh
codesign --verify --deep --strict --verbose=2 Scuttle.app
codesign -dvv Scuttle.app 2>&1 | grep Signature   # Signature=adhoc
spctl --assess --verbose=4 --type execute Scuttle.app   # rejected: expected
```

`spctl` rejecting the bundle is the correct outcome for an unnotarized app and
is not a defect. `codesign --verify` failing *is* a defect, and the release
workflow fails the build on it.

### Signing and notarisation, if a Developer ID ever exists

An unsigned build runs locally but Gatekeeper will refuse it on anyone else's
machine. For distribution you need an Apple Developer account and a
**Developer ID Application** certificate.

```sh
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="app-specific-password"   # not your Apple ID password
export APPLE_TEAM_ID="TEAMID"

npm run app:build
```

Tauri signs, submits for notarisation and staples the ticket. Verify with:

```sh
spctl -a -vvv -t install "src-tauri/target/release/bundle/macos/Scuttle.app"
xcrun stapler validate "src-tauri/target/release/bundle/dmg/Scuttle_0.1.0_aarch64.dmg"
```

### Entitlements

Scuttle is not sandboxed. A sandboxed app cannot read Downloads, Desktop and
application support directories without a per-folder user selection, which
would make a one-button rummage impossible.

It requests no special entitlements: no Full Disk Access, no accessibility. The
system's own consent prompts for Downloads, Desktop and Documents apply, and
Scuttle reports the places it could not read rather than demanding more.

### Universal binaries

```sh
rustup target add x86_64-apple-darwin aarch64-apple-darwin
npm run app:build -- --target universal-apple-darwin
```

## Windows

Produces an NSIS installer (`*-setup.exe`). `installMode` is `currentUser`, so
no elevation is required: Scuttle only ever touches the current user's files,
and asking for admin rights would be asking for more than it needs. It also
means the SmartScreen prompt is the only thing in the way, rather than a UAC
prompt stacked on top of it.

### WebView2

`webviewInstallMode` is `downloadBootstrapper`, set explicitly rather than left
to the default so that it reads as a decision. Windows 10 from the April 2018
release onwards, and every Windows 11, ship the WebView2 runtime as part of the
operating system, so on any machine Scuttle supports the bootstrapper does
nothing at all. On the rare one that lacks it, it fetches the runtime silently.

The alternative, `offlineInstaller`, embeds the runtime and adds roughly 127 MB
to a download that is otherwise a few. For an unsigned installer that is the
worst possible trade: SmartScreen reputation is per file hash, so a very large
download would carry exactly the same warning as a small one.

`skip` is never used. It produces an installer that succeeds and then an
application that will not open.

### Signing, which releases do not do

Released installers are unsigned, so SmartScreen shows "Windows protected your
PC" on first run; **More info → Run anyway** gets past it, and the README says
so. Because reputation is accumulated per file hash, every new release starts
from nothing — that does not improve with downloads and is simply the cost of
distributing without a certificate.

If that ever changes, you need a code-signing certificate — an EV certificate
builds SmartScreen reputation immediately; a standard OV certificate
accumulates it over time.

Configure the certificate through Tauri's Windows signing settings
(`bundle.windows.certificateThumbprint`, or `signCommand` for a cloud signer).
Do **not** use `TAURI_SIGNING_PRIVATE_KEY` for it: that variable is the
*updater* key, a different thing entirely (see [`updates.md`](updates.md)), and
a `.pfx` in it would sign nothing the operating system trusts.

### Uninstall behaviour

The installer registers a proper uninstall entry. Uninstalling removes the
application but **leaves Scuttle's data directory in place**
(`%LOCALAPPDATA%\Scuttle`), because it may still contain quarantined files the
user has not decided about. That is deliberate, and the uninstaller says so.

There is an irony here that is worth stating plainly: a utility that finds
leftovers from uninstalled software should not leave a pile of its own. It
leaves exactly one directory, it tells you where, and the Settings screen shows
the path.

## What has and has not been tested

Worth keeping these apart, because "it built" gets reported as "it works" far
too easily:

| | Covered by |
| --- | --- |
| **Compilation** | `cargo clippy --all-targets` and `cargo test` on macOS and Windows, every pull request |
| **Packaging** | the `Build desktop` workflow, on main and on every release tag: a real `.dmg` and a real `-setup.exe`, each inspected where it was built |
| **Installation** | not automated. Opening a downloaded `.dmg`, dragging to Applications and clicking through the Gatekeeper prompt is done by hand before announcing a release |
| **Runtime smoke test** | not automated. `--dry-run` against a fixture home exercises the whole scan pipeline headlessly; the window itself is opened by hand |

The packaging checks read the artifact rather than trusting the build log: the
`.dmg` is mounted and the application inside it is inspected — version,
identifier, executable name, icon, architecture and signature — and on Windows
the installer's size and the binary's version resource are read back.

## Reproducibility

`Cargo.lock` and `package-lock.json` are committed. The release profile uses
`lto = true`, `codegen-units = 1`, `opt-level = "s"` and `strip = true` — the
binary is small, and a filesystem utility is I/O-bound rather than CPU-bound.

## Updates

Scuttle updates itself through the official `tauri-plugin-updater`, driven from
Rust. All of it is in [`updates.md`](updates.md); the parts that touch
packaging are:

- Signature verification is mandatory and is not configurable: the public key is
  in `tauri.conf.json` and a package that does not verify against it is thrown
  away, as is one whose signature is not bound to the announced version.
- Nothing downloaded is run before its signature has been verified.
- Update signing is **separate** from macOS code signing and Windows
  Authenticode, and replaces neither. The ad-hoc macOS signature and the
  unsigned Windows installer are exactly as described above.
- Update checks are a network request. They are disclosed, and switchable in
  Settings.
- The macOS update package is `Scuttle.app.tar.gz`, so the macOS build asks for
  the `app` bundle as well as the `dmg` when update artifacts are wanted.
