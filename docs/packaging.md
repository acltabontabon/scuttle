# Packaging

```sh
npm run app:build
```

Artifacts land in `src-tauri/target/release/bundle/`.

Users need no developer runtime. Tauri bundles what it needs and uses the
system webview — WebKit on macOS, WebView2 on Windows.

## macOS

Produces `Scuttle.app` and a `.dmg`. Minimum supported version is 10.15.

### Signing and notarisation

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

Produces an NSIS installer and an MSI. `installMode` is `currentUser`, so no
elevation is required: Scuttle only ever touches the current user's files, and
asking for admin rights would be asking for more than it needs.

### Signing

Unsigned installers trigger SmartScreen warnings. You need a code-signing
certificate — an EV certificate builds SmartScreen reputation immediately; a
standard OV certificate accumulates it over time.

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY="path\to\certificate.pfx"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD="…"
npm run app:build
```

### Uninstall behaviour

The installer registers a proper uninstall entry. Uninstalling removes the
application but **leaves Scuttle's data directory in place**
(`%LOCALAPPDATA%\Scuttle`), because it may still contain quarantined files the
user has not decided about. That is deliberate, and the uninstaller says so.

There is an irony here that is worth stating plainly: a utility that finds
leftovers from uninstalled software should not leave a pile of its own. It
leaves exactly one directory, it tells you where, and the Settings screen shows
the path.

## Reproducibility

`Cargo.lock` and `package-lock.json` are committed. The release profile uses
`lto = true`, `codegen-units = 1`, `opt-level = "s"` and `strip = true` — the
binary is small, and a filesystem utility is I/O-bound rather than CPU-bound.

## Updates

There is no auto-updater in this release, deliberately: shipping an update
mechanism badly is worse than not shipping one.

The architecture leaves room for `tauri-plugin-updater`, which verifies a
minisign signature before applying anything. When it is added:

- Signature verification is mandatory, not a configurable option.
- Nothing downloaded is executed before its signature is verified.
- Update checks are a network request, so they will be disclosed and
  switchable — today Scuttle makes none at all.
