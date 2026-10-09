# Using Scuttle

[Download and install](installing.md) · [Organizing files](organizing.md) · [Back to Scuttle](../README.md)

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

Open a pile to review its largest findings, each one tickable, with a running
total of what you have picked. Up to 200 findings are shown at once; handling
those brings the next ones into view, and the pile's total always includes
the whole set. Scuttle marks its own suggestions, and you are free to ignore
them — a finding is something noticed, not something condemned.

**Space** explains where the room went. Its folder measurements have entry,
depth and time limits, and leave protected places and application bundles
unopened. A partial figure says **at least**; it is a lower bound, not an exact
total. Measuring stops between entries after a five-second budget, though an
individual filesystem call can take longer. Choose **Measure again** for a
fresh look. Moving something into the drawer keeps it on disk; only permanent
removal gives space back.

## Developer build output

Enable **Developer build artefacts** in Settings to find old build folders.
Scuttle discovers common project locations (Code, Projects, Workspace, Developer
and source/repos) and lets you add your own. Additional locations are checked
only for developer debris. The default inactivity window is 14 days, with 30
and 60 day choices.

Verified Cargo `target`, Next.js `.next`, and .NET `obj` output can be suggested
when both source and output are old enough, repository checks pass, and no
relevant running tool is found. Dependencies and ambiguous build folders remain
manual choices and never expire automatically. A full rummage verifies build
markers and Git metadata; background findings are preliminary. Rebuilding can
take time. Review still comes first, and space is reclaimed only after items
leave the drawer permanently.

## Dependency cache preview

Settings → **Dependency caches** adds an opt-in, read-only inventory for Maven,
Gradle, npm, pnpm and Yarn. Choose **Inspect dependency caches** to see recognized
entries, versions where their identity is known, sizes, project references and
why each entry is preserved. The same inspection is included in desktop and
command-line dry runs when the preview setting is enabled. It does not run during background checks.

The retention choices are 90, 180 and 365 days, separate from build output.
Recent changes, project references, locally installed Maven artifacts, snapshots
and Keep decisions preserve an entry. An old modification date never proves
that a dependency is unused. Last use remains unknown unless there is trustworthy
tool-provided evidence.

Default cache locations are included; add custom repository or store directories
when your tools use other paths. The pnpm reader includes regular and executable
(`-exec`) content blobs in unversioned `files/` and `vN/files/` stores; index files
and unrecognized layouts are excluded. Project discovery uses the project folders
shared with developer build artefacts, independently of that setting. Maven POMs,
Gradle lockfiles and wrapper properties, npm lockfiles and selected Yarn package
metadata provide positive references only. Unresolved profiles, transitive
Maven dependencies, pnpm/Yarn lockfile references and other branches remain
unknown. Opaque Node blobs are identified by package name only when matching
metadata or npm lockfile integrity establishes that identity.

**This release does not remove dependency cache entries.** All adapters remain
inspection-only until usage tracking, compatible tool coordination and a
recoverable move/restore are verified. Known stores and configured custom stores
are protected from generic moves, including whole-cache findings from older
scans, even when preview is off. Existing drawer items remain recoverable.

Inspection has a five-second budget between filesystem calls, with entry, depth
and manifest-read limits. Partial results say **at least**; totals describe the
logical sizes of recognized entries, not guaranteed reclaimable disk space.
Unrecognized layouts and cache-wide metadata are excluded. No build scripts run,
no repositories are contacted, and credentials and linked paths are left alone.

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
  mail or your cloud-sync folders. Those directories are never walked.
  Repository internals are excluded from traversal and cleanup; opt-in developer
  verification reads only the Git metadata needed to protect tracked files.
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
[docs/architecture.md](architecture.md) for what that actually costs.

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
[docs/updates.md](updates.md).

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
