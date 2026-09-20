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
