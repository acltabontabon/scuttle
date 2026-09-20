# Architecture

## The shape of it

```
┌─ React / TypeScript ─────────────────────────────────┐
│  presentation, interaction, motion, accessibility    │
│  addresses everything by id, never by path           │
└──────────────────────┬───────────────────────────────┘
                       │  Tauri IPC
                       │  coarse commands + event streams
┌──────────────────────▼───────────────────────────────┐
│  Rust core                                           │
│                                                      │
│  commands/   the IPC surface and app state           │
│  scanning/   traversal, orchestration, the safety net│
│  detectors/  one question each, evidence only        │
│  evidence/   observations → confidence, risk, action │
│  safety/     path arithmetic, protected paths, gate  │
│  quarantine/ move, restore, remove                   │
│  storage/    SQLite behind a repository boundary     │
│  platform/   the only place cfg(target_os) appears   │
│  space/      the storage explanation                 │
└──────────────────────────────────────────────────────┘
```

## Why this stack

**Tauri 2** rather than Electron: Scuttle is a filesystem utility that should
be small and start instantly. Bundling a browser to walk directories is the
wrong trade, and the security-sensitive half of this app wants to be in Rust
regardless.

**Rust owns everything trust-sensitive.** Traversal, detection, evidence,
confidence, risk, path safety, quarantine, restore, deletion, hashing,
installed-app discovery. The webview cannot perform a filesystem operation, and
there is no command that accepts a path and acts on it.

**React owns presentation only.** No cleanup logic lives in TypeScript. The
frontend does not decide what a file is, how confident to be, or whether
something may be removed — it renders decisions the core already made and
sends requests back by id.

## Three properties everything else follows from

### 1. Detectors produce evidence, not verdicts

A detector emits a `Finding`. Look at what a `Finding` deliberately *cannot*
contain: a confidence level, a risk level, or a recommended action. Those are
computed centrally in `evidence::weigh` from the same evidence list the user
can read. A detector author cannot mark their own work as high confidence, and
a buggy detector cannot promote itself.

### 2. Everything passes through the safety net

`GuardedSink` sits between every detector and the outside world. Before a
finding exists it is checked against the protected-path table, the
too-shallow rule, the user's ignore list, and the sensitive-area rules. There
is a test that takes a detector deliberately trying to surface an SSH private
key and proves the net drops it.

### 3. A finding is a request, not an authorisation

Between a scan and an action the world moves. `safety::authorize` re-derives
every decision against the live filesystem: the path is re-normalised,
re-checked against the protected table, re-checked for containment within the
scanned roots, re-checked for symlinks below those roots, and compared against
the state fingerprint captured at scan time. Any material change is a refusal,
not a warning.

## Scanning

```
prepare   read installed apps, game libraries and running processes once
   │
walk      one streaming pass per root; every entry offered to every
   │      stream detector — detectors never walk the tree themselves
   │
probe     detectors that need targeted lookups (known caches, game
   │      libraries) do them here
   │
finish    deferred expensive work: hashing, perceptual grouping
   │
guard     every candidate re-checked before it is allowed to exist
```

The single shared traversal is the main performance decision. Seven detectors
each walking the disk would be seven times the I/O for the same answers.
Detectors that genuinely need to look somewhere specific use `probe` instead.

The walker never follows links, which makes traversal loops structurally
impossible and means a link cannot carry the scan outside the roots the user
chose. macOS bundles and a small set of leaf directories (`node_modules`,
`.git`) are reported as single objects rather than opened — except when the
walker is pointed directly at one, so their size can still be measured.

Permission errors, files that vanish mid-walk and unreadable directories are
**hiccups**: counted, reported as totals, and stepped over. None of them abort
a scan. They're also counted rather than logged, because a log of every
unreadable path is a map of someone's home directory.

## Communication

One `rummage` command starts a scan and returns immediately. Results arrive as
events (`scuttle://phase`, `progress`, `found`, `done`), each tagged with the
scan id so a superseded scan's events are discarded. React never calls into
Rust per file.

Operations are `start_move(request)`, `restore(id)`, `keep(id)`,
`ignore(id, scope)` — ids throughout.

`start_move` does almost nothing itself: it claims the operation gate, records a
job, starts a worker thread and returns. Progress arrives on `scuttle://move`.
Every snapshot carries a **job id** and a **revision** that only rise, so a late,
repeated or reordered event cannot overwrite newer state, and `move_status`
lets a listener that missed events catch up. Updates are counts, not lists, and
are sent at most every 100 ms. A synchronous Tauri command runs on the main
thread, which is the thread that draws the window, so anything that touches the
disk for long is a worker or a blocking-pool task, never a plain command.

## Storage

SQLite via `rusqlite`, behind the `Store` repository. Every SQL statement in
Scuttle is in `storage/mod.rs`.

Scuttle stores scans, findings, evidence, ignores, quarantine records, cleanup
history and settings. It does **not** build an index of the filesystem. Old
scans are pruned on launch; this is not an archive.

Migrations exist from the first release, applied by `PRAGMA user_version`
inside a transaction. Shipped migrations are never edited.

Two details worth knowing:

- **Evidence gets its own table.** It is the part a person reads when deciding
  whether to trust a finding, so it should be queryable rather than buried in
  a JSON blob.
- **Unreadable enum values fail closed.** A risk level the code doesn't
  recognise parses as `Protected`, and an unrecognised action parses as
  `InspectOnly`. Corruption or a downgrade must never read as "safe to
  delete".

## Quarantine

Findings move into a holding area, one directory per item, with a JSON manifest
beside them so the drawer is legible even without the database. The move is a
no-replace rename; across volumes a file (never a whole folder) is copied,
verified and then removed. A shared cache folder is not moved at all: its
reviewed files are, one at a time, and the record is a `contents` record listing
exactly what it holds. See [safety.md](safety.md#how-files-actually-move).

A record is written as *moving* before anything moves and settled afterwards,
with a per-file checkpoint in between, so an interrupted move can be understood
on the next start instead of guessed at.

Restore never overwrites: if something now occupies the original path, the item
is placed beside it under a new name and the caller is told where it went.

The drawer lives inside a directory Scuttle scans, so it's added to the
protected table at runtime — otherwise Scuttle would find its own quarantined
items and offer to quarantine them again. There's a test for that.

## Platform layer

`PlatformService` is the whole interface: installed applications, application
data roots, screenshot locations, cache rules, game libraries, running
processes, reveal, quarantine root, installer extensions. Detectors ask it
questions; they never ask which OS they're on.

There are two related but distinct questions the platform answers about a
directory, and conflating them causes bad findings:

* `application_data_roots` — places where each *immediate child* belongs to one
  application. These are ghost candidates.
* `application_managed_roots` — places an application maintains top to bottom.
  A superset: `~/Library/Developer` is Xcode's entirely, so its children are
  not ghost candidates, but nothing inside it is a duplicate the user can act
  on either.

`platform::testing::FixedPlatform` is public API, not a test helper hidden
behind `cfg(test)`. Writing a detector means writing tests for it, and those
must not depend on what happens to be installed on the machine running them.

## Performance

Scanning is I/O-bound and the interface has to stay responsive while it
happens. Two things make that work: the scan runs on a worker thread and
streams results as events, and it is cancellable at every checkpoint.

The rest came from measuring rather than guessing. Every scan reports
`walk_ms`, `probe_ms` and `finish_ms`, and the dry run prints them — because
the first two things that *looked* slow were not the slow ones.

On a real machine, 351,773 files across Downloads, Desktop, Application
Support, Caches, Logs and Developer:

| | Before | After |
| --- | --- | --- |
| Walk | 36.3 s | 4.8 s |
| Probe | 0.04 s | 0.01 s |
| Finish (hashing, image decoding) | 9.8 s | 2.6 s |
| **Total** | **46.0 s** | **10.0 s** |

Five changes, in the order they mattered:

1. **The protected-path table is compiled once.** It is consulted for every
   entry in a scan, and it was re-folding both the candidate path *and* every
   rule's own path on each check — roughly 120 path normalisations per file.
   Rules are now pre-folded at construction and grouped by how they match, and
   the candidate path is folded once per query. This was almost all of the
   walk's cost.
2. **Directory profiles are built during the shared walk.** Detectors used to
   walk each candidate's subtree to find out how big it was, and the
   directories they care about are exactly the ones holding most of the files.
   `ScanContext::profile_of` now returns the profile gathered during the main
   traversal, falling back to a walk only for paths the scan did not cover.
3. **Each file is classified once.** Working out whether a file looks
   generated, authored or like save data was being done inside the
   per-directory loop — which meant re-lowercasing every path component nine
   times over, four levels deep, for every file. It is now computed once per
   file and folded into each containing directory as a plain struct.
4. **Derived strings are computed once.** A file's lowercased name and
   extension are produced by the walker and carried on the entry, rather than
   re-allocated by each of the six detectors that wanted them.
5. **Hashing runs in parallel.** Duplicate detection is staged — group by size,
   then a head-and-tail probe, then a full streaming hash — and the stages are
   embarrassingly parallel. The sink is not `Send`, so the work happens across
   threads and emission happens after, in order.

The last of those got faster for a second reason: the duplicate detector no
longer hashes files an application maintains for itself, which on a developer's
machine is most of them.

Bounded by construction: large files are streamed rather than read into memory,
image decoding is capped, directory measurement has an entry budget and admits
when it was truncated, and the directory index has a ceiling.

## Background mode

Off by default. With `background_mode` off, none of this runs and closing the
window quits, exactly as before.

### Three settings, nested

`background_mode` → `background_checks` → `background_notify`. Each needs the
one before it, and `save_settings` enforces that server-side rather than
trusting the interface: a check cannot happen if closing the window quits, and
there is nothing to notify about if no checks happen. `launch_at_login` sits
apart — staying in the menu bar is not permission to start with the machine.

### Window lifecycle

The window is **hidden, never closed**. That is the load-bearing decision:
because the window still exists, `RunEvent::ExitRequested` never fires from a
close, so Scuttle never has to call `prevent_exit` — and therefore ⌘Q, the
tray's own Quit, logging out and shutting down all simply work. Nothing in the
codebase prevents an exit.

Hiding is conditional on `background_mode && tray_alive`. If the tray icon
failed to build, closing quits as usual: a hidden window with no icon to
return from would be a trap, and Settings says so rather than leaving the
toggle looking like a promise.

On macOS the activation policy follows the window — `Accessory` while hidden,
`Regular` set *before* `show()` so the window comes to the front rather than
appearing behind everything.

`tauri-plugin-single-instance` keeps it to one process per user. Two would
share one database and one drawer while each believed its own operation gate
was the only one.

### The scheduler

One thread, asleep on a `Condvar` with a five-minute timeout. Pausing,
switching the setting off, and quitting all wake it immediately rather than
waiting out the interval. A tick that decides to do nothing costs a handful of
integer comparisons.

All the judgement is in `background::schedule::decide`, a pure function of the
clock, the persisted state and what the machine reports — which is what makes
the policy testable against a clock moved by hand. The gates run cheapest
first, so the expensive signals (a subprocess on macOS) are consulted only
once everything free has agreed: in practice a handful of times a day rather
than twelve times an hour.

Scheduling state lives in the `settings` table under `background`, so
restarting does not earn a fresh check. A gap between ticks larger than
`SLEEP_GAP` means the machine slept; the answer is to settle for fifteen
minutes, not to catch up. There is no queue of missed checks.

### What a check is allowed to do

It goes through the ordinary `start_scan` path, so it takes the same operation
gate, obeys the same ignore lists, and every candidate passes the same safety
guard. Two differences:

- **It reads no file contents.** `detectors::glance_set` is `default_set`
  minus `duplicates` (BLAKE3) and `screenshots` (image decode) — the only two
  detectors that open files. That is what makes it cheap enough to run
  unattended, and it is also what makes it incomplete.
- **It is bounded in time.** A watchdog sets the same flag the Stop button
  does after ten minutes, so it unwinds through the ordinary cancellation path
  and its partial results are saved *and labelled*.

Because the results are partial, `scan_runs.kind` records `full` or `glance`,
and the findings screen says which it is looking at. Without that, an empty
Copies pile after a check reads as "you have no duplicates" when nothing
opened a file to find out.

### Yielding

A background check yields to a person, and never the reverse. The rule lives
in one place — `begin_operation_as(op, Priority)` and `start_scan_as` — rather
than at each of the eight call sites, so no command can forget it. A user
action finding the gate held by a check cancels it and waits up to 1.5 s;
since a metadata-only scan tests its cancel flag on every 120 ms progress
tick, that is tens of milliseconds in practice. If the wait runs out, the
ordinary "Scuttle is busy" refusal is what appears, unchanged.

Lock ordering is `running` → `gate`, consistently. The yielding half of
`begin_operation_as` inspects the running scan, so `start_scan_as` — which
already holds that lock — goes to `claim_gate` directly.

### Drawer retention

Expiry used to run once per launch, and `Drawer.tsx` said so. An application
that stays in the menu bar starts far less often, so the same promise would
quietly have become "after N days, eventually". The scheduler therefore sweeps
on its tick as well, under the previously unused `Operation::Sweep`.

This preserves the existing contract rather than changing it: an item still
expires after exactly the days it was given, and only items the user placed in
the drawer under a stated date are ever removed. With background mode off,
behaviour is byte-for-byte what it was.

Startup work is claimed once per process (`AppState::claim_startup`), so
showing a hidden window is never mistaken for a launch that re-runs cleanup.

### Cost when hidden

Measured on an Apple Silicon Mac, macOS 27.0 (Darwin 27.0.0), 10 cores, on
battery, **debug build** — a release build will differ. One window, a drawer
of seven items, background mode and background checks both on. Sampled with
`ps` every ten seconds.

| | Hidden (60 samples, 10 min) | Visible (42 samples, 7 min) |
|---|---|---|
| Resident set | 114.8 MB, flat to ±0.1 MB | 116.2 MB at launch, settling to 86.3 MB |
| CPU, idle | 0.00% on every sample | 32.7% peak during startup, 0.0% once settled |

Two honest caveats. The two runs are separate processes with different
histories, so the resident-set figures are not a controlled comparison and
the difference between them should not be read as hiding costing *more*. And
the display slept during part of the visible run, so its idle CPU figure
understates a window somebody is actually using.

What the numbers do support: **an idle Scuttle costs no measurable CPU in
either state, and hiding the window does not give the memory back.** The
webview stays resident; the saving is in frames not composited, not in pages
returned. Scuttle does not market this as "zero impact".

The scheduler itself is one thread asleep on a condition variable, waking
twelve times an hour to compare a few integers. Over twenty minutes of ticks
on this machine it ran zero checks — correctly, since the machine was on
battery — which is the behaviour the conservative gates exist to produce.

What was actually changed for this: scheduling and scan coordination are in
Rust, outside the render loop entirely; the frontend adds no polling (the
background status arrives as an event, with a catch-up on `visibilitychange`,
the same arrangement moves already used); and `:root[data-window="hidden"]`
pauses the looping decorative animations with `animation-play-state` so they
resume where they stopped rather than snapping back.

No architecture rewrite was undertaken to eliminate the webview, and none is
proposed on this evidence.

## Frontend state

A single React context over `useState`. The Rust core is the source of truth
for filesystem state, scans, candidates, quarantine and safety; what the
frontend holds is which view is open, what the running scan has said so far,
and a cache of the last answer from each command. Reaching for Redux to hold
six values would be its own kind of mess.

## Why Apache 2.0

Both Apache 2.0 and MIT would be reasonable. Apache 2.0 because:

- It grants patent rights explicitly. MIT is silent on patents, and this is a
  filesystem tool with heuristics that organisations may want to contribute to
  and adopt.
- Its contribution terms (§5) mean contributions arrive under the same licence
  without a separate CLA.
- It requires modifications to be marked, which for software that deletes
  files is worth having: a fork that loosens the safety rules should be
  identifiable as a fork.

The dependencies are MIT/Apache-2.0 dual-licensed, so there is no conflict.
