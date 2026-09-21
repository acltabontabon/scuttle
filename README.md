<div align="center">

<img src="src-tauri/icons/128x128.png" width="88" alt="">

# Scuttle

**Your computer leaves stuff everywhere. Scuttle finds it.**

[![CI](https://github.com/acltabontabon/scuttle/actions/workflows/ci.yml/badge.svg)](https://github.com/acltabontabon/scuttle/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Latest release](https://img.shields.io/github/v/release/acltabontabon/scuttle?include_prereleases&label=release)](https://github.com/acltabontabon/scuttle/releases)
[![Platforms](https://img.shields.io/badge/platforms-macOS%20%C2%B7%20Windows-lightgrey)](#platform-support)

**[scuttle on the web →](https://acltabontabon.com/scuttle/)**

</div>

<img width="100%" alt="A run through Scuttle: pressing Rummage and watching it work through 301,264 files, the findings settling into four illustrated piles totalling 1.21 GB, opening the pile of leftovers to see two folders each marked “The app is gone.”, putting one in the drawer — which says nothing has been deleted and the space is not back yet — then opening the drawer and putting the file back where it came from." src="docs/media/demo.gif">

Scuttle is a desktop utility that rummages through the forgotten corners of your
computer and shows you what turned up:

- apps that left files behind
- forgotten screenshots
- installers you probably don't need any more
- duplicates
- abandoned caches
- giant mystery files
- assorted digital oddments

Scuttle tells you **why** something looks disposable before asking you to do
anything about it. Most of what a rummage finds is yours to keep — the point is
to see it, not to sweep it.

No accounts. No cloud scanning. No "boost your PC" nonsense.

---

## Download

| Your computer | Download |
| --- | --- |
| **Mac**, Apple silicon (M1 and later) | [`.dmg`](https://github.com/acltabontabon/scuttle/releases) |
| **Mac**, Intel | [`.dmg`](https://github.com/acltabontabon/scuttle/releases) |
| **Windows** 10 or 11, 64-bit | [`-setup.exe`](https://github.com/acltabontabon/scuttle/releases) |

All three are on the [releases page](https://github.com/acltabontabon/scuttle/releases),
each with a `.sha256` file beside it. Not sure which Mac you have? Apple menu →
About This Mac; "Apple M1" or later means Apple silicon.

0.1.0 is the first stable release. It does what this page describes and its
tests pass on macOS and Windows. It is still young software that has not met
many machines, so the drawer is there for a reason: look in it before you empty
it.

### Installing

Scuttle is built without paid code-signing certificates: it is **not signed with
an Apple Developer ID and not notarized** on macOS, and the Windows installer is
**unsigned**. Neither operating system can tell you who made it, so both will say
so the first time. Here is what you will see, and what to do.

**macOS.** Open the `.dmg` and drag Scuttle to Applications, then open it from
Applications. You will be told the developer cannot be verified. Go to **System
Settings → Privacy & Security**, scroll down to **Security**, and click **Open
Anyway** next to Scuttle, then open the app again. That button only appears
after you have tried to open the app, and only for about an hour afterwards, so
if you have wandered off and come back, try opening Scuttle once more first. You
do this once.

The macOS builds are *ad-hoc* signed, which is what lets an Apple silicon build
launch at all. Ad-hoc signing is not a verified publisher identity and it is not
notarization — it only means the bundle is internally consistent. If macOS tells
you Scuttle is **damaged** rather than unverified, that is not the same prompt
and it is not something to click past: the download is broken or was tampered
with. Check it against its `.sha256`, download it again, and
[open an issue](https://github.com/acltabontabon/scuttle/issues) if it persists.

**Windows.** Run the `-setup.exe`. Microsoft Defender SmartScreen will say it
prevented an unrecognised app from starting: click **More info**, then **Run
anyway**. Scuttle installs for your user account only, so Windows will not ask
for an administrator password. You will see this warning again on future
versions — SmartScreen's reputation is per file, and an unsigned project never
accumulates any.

Please don't switch off Gatekeeper, SmartScreen or your antivirus to install
this, or anything else. Nothing above asks you to change a system setting.

---

## What it actually does

You press **Rummage**. Scuttle walks the places software leaves things — your
Downloads folder, the Desktop, application support and cache directories,
screenshot folders, Steam and Epic libraries — and comes back with what it
noticed, grouped into piles you can dig through.

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

Open a pile and you get every file in it, each one tickable, with a running
total of what you have picked. Scuttle marks its own suggestions, and you are
free to ignore them — a finding is something noticed, not something condemned.

## The drawer

Nothing is ever deleted as a side effect of anything else.

**Putting something in the drawer** moves it into a holding folder inside
Scuttle's own data directory and writes down where it came from, both in its
database and in a plain manifest beside the item. The file is out of your way
but still on the disk.

**Before anything moves, you see what will move.** Every move — one item, a
batch you ticked, a group of copies, Scuttle's own suggestions — opens a review
first: the exact path of each thing, whether it is one file or a whole folder
and how much is in it, why Scuttle noticed it, anything it is unsure of, and
what it will refuse. Anything worth knowing ("changed 2 days ago", "Scuttle is
unsure what this is") is said once for the whole move and accepted with one
tick, not file by file.

**Putting it back** returns it to exactly where it was. If the folder it lived
in has since been deleted, Scuttle recreates it. If something else has since
taken its name, Scuttle restores it alongside, as "name (restored)", rather
than over the top, and tells you it did. It checks the way back first: if a
folder on the path has become a link to somewhere else, or somewhere protected,
the item stays in the drawer. It also checks the held copy is still exactly what
went in.

**Emptying the drawer** is the only thing that frees space, and the only thing
that destroys data. Items do not go to the Trash or the Recycle Bin, which is
what the confirmation says before you confirm it. Only things that can be
rebuilt or downloaded again — caches, build output, installers — expire on
their own, after a window you choose (7, 14 or 30 days). Your own files, an
application's data, and anything you moved past a caution stay until you remove
them.

## What it won't do

- It won't invent problems. A scan of a tidy machine comes back quiet.
- It won't call a file junk because it's old. Age alone never reaches high
  confidence.
- It won't delete anything as a side effect. Things move into a drawer first,
  and stay recoverable.
- It won't go near your keys, your password vault, your browser profiles, your
  mail, your repositories or your cloud-sync folders. Those directories aren't
  just excluded from results — they're never walked at all.
- It won't treat an installed application as a leftover. Anything shaped like
  an application — an updater beside versioned folders, a program beside its
  libraries, a `.app` bundle — is never suggested and never swept, whatever a
  registry says or whether it is running. You can still move an application's
  folder yourself, one at a time, with a plain warning that it will probably
  stop working. Scuttle does not uninstall or relocate applications.
- It won't show you a health score, a fake urgency counter, or a percentage
  with three decimal places.
- It won't sit in your menu bar unless you ask it to, and it won't scan behind
  your back. Both are separate settings, both off by default.

## Two ideas that do most of the work

**Confidence, impact and permission are different things.** Scuttle can be
*completely certain* that a 40 GB archive hasn't been touched in two years and
still have no business suggesting you move it. Confidence is how sure it is
about what something is; impact is what moving it could disrupt; permission is
which ways of asking may move it. Scuttle only *suggests* things that are
rebuildable or re-downloadable, that it is confident about, and that need no
caution. Your own files are always yours to move, with the caution said once.
Applications never go in a sweep or a batch.

**A finding is a request, not an authorisation.** The interface can ask to
quarantine something, but the Rust core re-derives every decision against the
live filesystem first: does the path still exist, is it still the same size and
shape, does it pass through a symlink, is it protected, is it inside the area
you asked Scuttle to look at? If anything moved between the scan and the
action, Scuttle refuses and asks you to rummage again.

## Staying in the menu bar

**Off by default.** Left alone, Scuttle quits when you close the window and
does nothing at all when you are not looking at it.

If you turn it on, closing the window hides it behind a menu bar icon (system
tray on Windows) instead of quitting. Clicking the icon opens Scuttle's
burrow: where things stand, one short line from Scuttle, and buttons to open
Scuttle, view the drawer, rummage, open Settings or quit. A right click opens a
plain menu with the same choices. Opening either starts nothing. *Quiet
Scuttle* in Settings keeps the wording plain and the creature still, and it is
still whenever your system asks for reduced motion.

⌘Q, logging out and shutting down all still quit properly, and quitting stops
Scuttle's background work. The one thing that makes quitting wait is files in
the middle of moving: Scuttle stops at the next safe point, says so, and quits
once the drawer is settled. Quit again and it goes at once; the next start
settles anything left in flight. No helper, no service, no daemon.

A second setting, also off by default, lets it check occasionally on its own —
at most once a day, and only when the machine looks able to spare it: on mains
power, not saving battery, not running hot, not already busy, and not while
you have the window open.

**A background check reads names, sizes and dates, and never opens a file.**
That makes it cheap, and it also makes it partial: it cannot find duplicates
or near-identical screenshots, because those need reading the files. Scuttle
says so on the findings screen rather than letting an empty pile read as good
news. It moves nothing, deletes nothing and selects nothing.

This is not idle detection and Scuttle does not pretend otherwise. A quiet
machine on mains power is a reasonable moment to try, not evidence that you
have stepped away. Scuttle asks for no Accessibility, Input Monitoring or
Screen Recording permission, watches no input, and reads no window titles.

Notifications are a third setting, off by default. At most one a day, only
when something new turned up, and never containing a filename — a summary can
end up on a lock screen. "Launch at login" is separate again, and is an
ordinary per-user login item with no installer and no administrator rights.

Hiding the window is not free: the webview keeps its memory. See
[docs/architecture.md](docs/architecture.md) for what that actually costs.

## It stays on your machine

There is no account, no sign-in and no server. Scuttle's only network request
is asking GitHub whether a newer version exists, and, when you say so,
downloading it — described below. (The one other thing that ever goes over the
network is the WebView2 installer on the rare Windows machine that doesn't
already have it, and that is Microsoft's, not Scuttle's.) Findings, settings and drawer records live in one
SQLite database in Scuttle's own directory on the machine that made them, and
full paths are never written to logs.

### Updates

Scuttle checks for a newer version shortly after it starts and about once a
day, and says so quietly in the header. **It only looks.** Nothing is
downloaded until you choose to, and it only restarts when you choose to; a move
or restore in progress always finishes first, and *Update and restart* waits for
it. Every update is verified with a signature before it is used. The check is
one HTTPS request to GitHub, and *Automatically check for updates* in Settings
turns it off; *Check for updates* still works when you ask. Stable installs are
never offered a prerelease. How it works, and how it is tested:
[docs/updates.md](docs/updates.md).

**If you installed a build from before updates existed** (`v0.1.0-alpha.1` or
`-alpha.2`) it has no updater and cannot fetch one. Install 0.1.0 by hand from
[the releases page](https://github.com/acltabontabon/scuttle/releases) — once.
From then on it can update itself.

`scuttle --drawer-report` prints everything the drawer holds or has held —
where each item came from, whether its held copy is still on disk — from a
read-only view of the database.

`scuttle --dry-run` in a terminal prints everything Scuttle would classify and
what it would recommend, and changes nothing. It's the one place full paths are
shown, and it's the quickest way to see what a rummage would say before you run
one.

## Platform support

| Platform | Status | Notes |
| --- | --- | --- |
| macOS 11+ (Apple silicon) | Supported | App bundles, Steam and Epic libraries, `~/Library` layout |
| macOS 10.15+ (Intel) | Supported | Same, on an Intel build |
| Windows 10+ | Supported | Uninstall registry (both views), Steam and Epic libraries, `AppData` layout |
| Linux | Not yet | The platform layer is a stub that knows nothing about a Linux system, so there is no Linux download |

Scuttle asks for no special permissions. On macOS that means folders needing
Full Disk Access are skipped rather than requested, and counted as places that
could not be read. On Windows, a drawer on a different drive from the thing
being held turns the move into a copy, verified by reading it back before the
original is removed, which needs room for both copies while it runs. Whole
folders on another drive are not moved at all; move the files inside instead.
The Windows tray icon picks its light or dark version when Scuttle starts.

## Development

Rust via [rustup](https://rustup.rs) and [Node](https://nodejs.org) 22+:

```sh
git clone https://github.com/acltabontabon/scuttle
cd scuttle
npm install
npm run app:dev       # the app, with the frontend hot-reloading
npm run check         # everything CI runs, in one command
npm run app:build     # a .dmg on macOS, a -setup.exe on Windows
```

[`docs/development.md`](docs/development.md) has the platform prerequisites and
the design workbench at `/preview.html`. [`docs/releasing.md`](docs/releasing.md)
is the release process; [`docs/packaging.md`](docs/packaging.md) is what actually
goes into a build.

The site at [acltabontabon.com/scuttle](https://acltabontabon.com/scuttle/) is
in [`www/`](www), with its own dependencies so none of it reaches the
application — `npm --prefix www run dev` to work on it, and
[`docs/website.md`](docs/website.md) for the rest.

## Documentation

- [Architecture](docs/architecture.md) — how the pieces fit together
- [Safety model](docs/safety.md) — what stops Scuttle deleting your thesis
- [Writing a detector](docs/writing-a-detector.md)
- [Privacy](docs/privacy.md) — what's stored, what's logged, what never leaves
- [Development](docs/development.md) · [Packaging](docs/packaging.md) ·
  [Releasing](docs/releasing.md) · [Website](docs/website.md)
- [Docker](docs/docker.md) — why there isn't an image
- [Changelog](CHANGELOG.md)

## Contributing

Yes please — especially detectors, and especially protected-path rules for
software we haven't thought of. Read
[CONTRIBUTING.md](CONTRIBUTING.md) first; the short version is that anything
touching the safety layer needs tests that demonstrate the failure it prevents.

Security concerns go through [SECURITY.md](SECURITY.md).

## Support

Scuttle is free and always will be. If it found something you were glad to get
back, you can [buy me a coffee](https://ko-fi.com/aclt_attic) — entirely
optional, and it buys you nothing but goodwill.

## Licence

Apache 2.0. See [LICENSE](LICENSE) and
[the reasoning](docs/architecture.md#why-apache-20).
