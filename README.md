<div align="center">

<img src="src-tauri/icons/128x128.png" width="88" alt="">

# Scuttle

**Your computer leaves stuff everywhere. Scuttle finds it.**

</div>

Scuttle is an open-source desktop utility that rummages through the forgotten
corners of your computer and shows you what turned up:

- apps that left files behind
- forgotten screenshots
- installers you probably don't need any more
- duplicates
- abandoned caches
- giant mystery files
- assorted digital oddments

Scuttle tells you **why** something looks disposable before asking you to do
anything about it.

No accounts. No cloud scanning. No "boost your PC" nonsense.

Just rummage around, see what turned up, and clean what you want.

---

## What it actually does

You press **Rummage**. Scuttle walks the places software leaves things — your
Downloads folder, application support directories, game libraries — and comes
back with what it noticed, grouped into piles.

Every finding carries its evidence:

```
Cyberpunk 2077                                            6.40 GB

Steam has no installation of Cyberpunk 2077.
6.40 GB stayed behind, untouched for 142 days.

Why Scuttle noticed
  ✓ Steam has no installation of Cyberpunk 2077
  ✓ Untouched for 142 days
  ✓ Contents look generated — 91% of it is cache and log files
  ✓ Nothing appears to be using it

Confidence: High          Risk: Low

  Reveal        Keep        Put in the drawer
```

And when Scuttle isn't sure, it says so:

```
Scuttle has no idea what this is.

You decide.
```

## What it won't do

- It won't invent problems. A scan of a tidy machine comes back quiet.
- It won't call a file junk because it's old. Age alone never reaches high
  confidence.
- It won't delete anything as a side effect. Things move into a drawer first,
  and stay recoverable.
- It won't go near your keys, your password vault, your browser profiles, your
  repositories or your cloud-sync folders. Those directories aren't just
  excluded from results — they're never walked at all.
- It won't show you a health score, a fake urgency counter, or a percentage
  with three decimal places.

## Two ideas that do most of the work

**Confidence and risk are different things.** Scuttle can be *completely
certain* that a 40 GB archive hasn't been touched in two years and still have
no business suggesting you delete it. Confidence is about classification; risk
is about what it costs if Scuttle is wrong. A finding is only ever proposed for
cleanup when confidence is high *and* being wrong would be cheap.

**A finding is a request, not an authorisation.** The interface can ask to
quarantine something, but the Rust core re-derives every decision against the
live filesystem first: does the path still exist, is it still the same size and
shape, does it pass through a symlink, is it protected, is it inside the area
you asked Scuttle to look at? If anything moved between the scan and the
action, Scuttle refuses and asks you to rummage again.

## Install

Builds aren't published yet. To run it from source you'll need
[Rust](https://rustup.rs) and [Node](https://nodejs.org) 20+:

```sh
git clone https://github.com/scuttle-app/scuttle
cd scuttle
npm install
npm run app:dev
```

To produce an installable artifact — a `.dmg` on macOS, an installer on
Windows:

```sh
npm run app:build
```

See [docs/development.md](docs/development.md) for platform prerequisites, and
[docs/packaging.md](docs/packaging.md) for signing and notarisation.

## Platform support

| Platform | Status | Notes |
| --- | --- | --- |
| macOS 10.15+ | Supported | App bundles, Steam and Epic libraries, `~/Library` layout |
| Windows 10+ | Supported | Uninstall registry (both views), Steam and Epic libraries, `AppData` layout |
| Linux | Not yet | The platform layer has a stub; nothing else is Linux-aware |

## Documentation

- [Architecture](docs/architecture.md) — how the pieces fit together
- [Safety model](docs/safety.md) — what stops Scuttle deleting your thesis
- [Writing a detector](docs/writing-a-detector.md)
- [Privacy](docs/privacy.md) — what's stored, what's logged, what never leaves
- [Development](docs/development.md)
- [Packaging](docs/packaging.md)

## Contributing

Yes please — especially detectors, and especially protected-path rules for
software we haven't thought of. Read
[CONTRIBUTING.md](CONTRIBUTING.md) first; the short version is that anything
touching the safety layer needs tests that demonstrate the failure it prevents.

## Licence

Apache 2.0. See [LICENSE](LICENSE) and
[the reasoning](docs/architecture.md#why-apache-20).
