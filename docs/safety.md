# Safety model

The design question behind every decision here is:

> What happens if the detector is wrong?

The answer has to be "nothing irreversible", and it has to come from the
architecture rather than from the detector being careful.

## Asymmetry

A missed cleanup candidate costs a user some disk space. A destructive false
positive costs them their SSH keys, their password vault, or their thesis.
These are not comparable, so Scuttle is not tuned to balance them.

Concretely, this is why:

- Age alone never reaches high confidence. A file is not junk because it is
  old.
- Being *certain* about something risky is not permission to act on it.
- Any single piece of evidence can raise risk; none can lower it.
- A category can be highly confident and still be inspect-only by default
  (heavy strays, screenshots).

## Four independent layers

Each can refuse on its own. A bug in one is caught by the next.

### 1. Path arithmetic (`safety/paths.rs`)

Every containment question compares **components**, never characters. The
classic bug is `/home/alice2` matching a prefix check against `/home/alice`;
there's a test named after it.

`..` is resolved textually and clamped at the root, so
`~/Downloads/../.ssh/id_ed25519` is correctly identified as being inside
`~/.ssh` and *not* inside `~/Downloads`. Comparisons are case-folded on macOS
and Windows, because otherwise a rule protecting `~/.ssh` is bypassed by
typing `~/.SSH`.

### 2. Protected paths (`safety/protected.rs`)

A table of places Scuttle never goes, consulted in three places: traversal
(don't even look), classification (refuse to surface), and action (refuse to
perform).

It covers credentials and keys, password vaults, version-control internals,
cloud-sync folders, browser profiles, device backups, sandboxed app data, and
the operating system's own territory on each platform.

Being pruned at traversal matters. A password vault isn't merely excluded from
results — it is never read, never measured, never counted.

Separately, `is_too_shallow` refuses anything that is a volume root, the home
directory, or a structural top-level folder like `~/Library` or `~/Documents`.
No evidence can override it.

A third table marks **sensitive areas** — Documents, Desktop, Pictures,
projects folders. These are not forbidden, because screenshots live on the
Desktop and Scuttle would be useless if it couldn't look there. Instead,
anything found inside one gains `USER_CONTENT_DETECTED` as evidence, which
raises risk through the normal arithmetic rather than through a special case.

### 3. The safety net (`scanning::GuardedSink`)

Between every detector and the outside world. Nothing protected, nothing too
shallow, and nothing the user has told Scuttle to forget can become a finding,
whatever the detector believes. There is a test in which a detector
deliberately tries to surface an SSH private key, and it proves the net drops
it.

### 4. The action gate (`safety/validate.rs`)

The filesystem changes. The gate assumes it has:

```
what is it, live? (stored evidence + is it, or does it hold, an application?)
        ↓
does this request's bidding allow that eligibility?
        ↓
has every caution on it been acknowledged for this request?
        ↓
is the path absolute, and not structurally off-limits?
        ↓
does a protected rule match it?
        ↓
is it still inside a scanned root?
        ↓
does any component below that root pass through a link?
        ↓
is the target itself a link?
        ↓
is it still the same kind of thing?
        ↓
for a folder: does anything inside it match a protected rule?
        ↓
does its size, modification time and child count still match the scan?
        ↓
                    quarantine
```

Any material change is a refusal with a specific reason — "this has been
modified since Scuttle found it, rummage again" — not a warning the user can
click through.

## Links, junctions and reparse points

Two rules:

**Traversal never follows them.** This makes loops structurally impossible and
means a link cannot carry a scan out of its roots. Both are tested, including
a symlink pointing at its own ancestor.

**Actions refuse to pass through them.** If any component *below the scanned
root* is a symlink, junction or reparse point, Scuttle declines rather than
resolving and hoping — the thing validated and the thing moved might not be the
same object.

Link detection is scoped to below the root deliberately. On macOS `/var` and
`/tmp` are themselves symlinks; on Windows `C:\Users\Public\Documents` is a
junction. Links above the area Scuttle was asked to look at are the operating
system's business. Links below it are the dangerous kind.

## Confidence, impact and eligibility

Three questions, answered separately (`safety/assess.rs`), because one "risk"
rating cannot say three things. It once both let an application through and
stopped people moving their own files.

**Confidence** is how sure Scuttle is about what an item *is*, from the
evidence (`evidence::weigh`). The levels the user sees are coarse — High,
Medium, Low. "Confidence: 83.742%" is a tell of software that is guessing.

**Impact** is what moving it could disrupt:

| Impact | Examples |
| --- | --- |
| Regenerable | A cache with a rule, build output |
| Redownloadable | An installer in Downloads |
| Personal file | Screenshots, archives, documents, anything large |
| Application data | Leftover app folders, anything inside app-data areas |
| Application install | Anything that is, or holds, an installed or portable application |

**Eligibility** is which requests may move it:

| Eligibility | Sweep ("Scuttle's picks") | A batch you ticked | One item, chosen on its own |
| --- | --- | --- | --- |
| Suggested | yes | yes | yes |
| By choice | no | yes, after its cautions | yes, after its cautions |
| Explicit only | no | no | yes, after its cautions |
| Blocked | no | no | no |

*Suggested* needs all of: high confidence, regenerable or redownloadable, and
no caution at all. Application installs are *explicit only*; application data
and personal files are *by choice*; only protected paths and things that cannot
be moved safely are *blocked*.

**Cautions** are specific, and are what a person accepts instead of being
refused: *changed recently* (in the last 7 days), *may be your work*, *in use*,
*application data*, *will likely break an application*, *Scuttle is unsure*.
They are grouped by kind and accepted once for a whole reviewed move
(`MoveRequest::acknowledged`), never file by file. The gate refuses an item
with a caution that was not accepted (`needs_acknowledgement`), whatever the
interface did.

Low confidence is a caution, not a lock. A person's own recently edited file
can always be moved after the caution; nothing old is called unused and no
download is called junk.

### Applications are recognised by their shape

The Discord incident on Windows (0.1.0-alpha.2) came from the installers
detector accepting any `.exe` over 512 KB anywhere under the scan roots —
including `%LOCALAPPDATA%`. Discord's own `app-1.0.x\Discord.exe` matched the
installed "Discord" and was rated a confident, low-risk installer, so *Move
everything confident* could take it (and `Update.exe`, which every Discord
shortcut launches). Moving either breaks the application.

Name, extension, age, size and location are not evidence of disposability.
Neither is missing uninstall metadata, nor a process that is not running right
now. `platform/installations.rs` recognises installations by structure:

* an updater (`Update.exe`) beside versioned `app-x.y.z` folders or `packages`;
* an Electron `resources/app.asar`;
* a program beside the libraries it loads (installed or portable);
* its own uninstaller;
* a macOS `.app` bundle;
* anything inside a platform install location (Program Files,
  `%LOCALAPPDATA%\Programs`, /Applications) or a registry `InstallLocation`.

Structure is checked at three points: the detectors skip it, the scan's safety
net (`GuardedSink`) tags anything at, inside or above an installation with
`PartOfInstalledApplication` evidence, and the gate checks the live filesystem
again before every move — so a finding recorded before Scuttle knew better is
still caught. A cache rule may name a folder beside an application, but a
cache finding that *is* or *holds* an application is dropped: finding a cache
never promotes its application folder.

## Acting on several things at once

Doing everything one finding at a time is its own kind of failure: a tool that
turns a tidy-up into fifty separate decisions is a chore, not a help. But bulk
handling is also how cleanup software does its worst damage.

* **A sweep names a pile, not a list of findings.** `MoveRequest::Confident`
  takes a `Category`, or nothing for the whole floor, and moves only what is
  *suggested*.
* **A batch never includes an application folder.** The interface does not
  let one be ticked, and the gate refuses one under `Bidding::User`. Only
  `MoveRequest::Single` — one finding opened and chosen — can move one.
* **The same finding twice is one move; a child inside a selected parent goes
  with the parent** and is reported as included, not moved twice.
* **Each item still passes the gate individually.** Being part of a batch is
  not an authorisation. If one file changed since the scan it is refused and
  reported, and the rest still go.

Every move is reviewed first (`preview_move`, the same gate run without moving
anything): exact paths, file or whole folder and how much is inside, evidence,
refusals, where it goes and how restore works.

## Quarantine first

Permanent deletion is not the primary workflow and never happens as a side
effect.

```
found → review → drawer → (rebuildable only) 7 / 14 / 30 days → permanent removal
                        → (everything else) kept until a person removes it
```

Only items whose impact is *regenerable* or *redownloadable*, moved with no
caution to accept, carry `keep = 0` and may expire. A person's own files,
application data, applications and anything moved past a caution are kept
until someone removes them, so a risky move a person chose never silently
becomes permanent. On upgrade from an alpha every held record is kept except
caches and build output (migration 4).

Each held item keeps its original path, its size, a content hash where cheap,
and a frozen snapshot of the evidence it was shown with. If a detector changes
its mind in a later version, the record of what the user was told does not.

Restore never overwrites. If something now occupies the original path, the
restored item goes beside it as `name (restored).ext` and the caller is told.
Before anything moves back, restore re-checks the destination: no link or
junction on the way below the home folder (or the drive's root), nothing
protected, not inside the drawer. It verifies the held copy against the hash
recorded when it went in and refuses — flagging the record — on a mismatch.
Journal paths that could climb out of their folder are refused.

To see what the drawer holds or held on a machine, without changing anything:

```
scuttle --drawer-report
```

or, directly: `sqlite3 -readonly <data dir>/scuttle.db "SELECT category,
status, original_path, stored_path FROM quarantine_items"`. The data directory
is `~/Library/Application Support/Scuttle` on macOS and `%LOCALAPPDATA%\Scuttle`
on Windows.

## How files actually move

A finding is a request; a move is a promise about what happens to someone's
files, and it is held to a stricter standard than the scan that suggested it.

**Copy only across volumes, and never to recover from a refusal.** A rename that
fails because the destination is on another drive can be answered with a copy.
A rename that fails because the system said no — access denied, in use, no
space — would fail a copy just as surely, and the old code copied first and
asked questions after. That fallback was also unsafe for folders: it copied the
tree, deleted the source tree file by file, and on a locked file removed the
copy too, leaving files in neither place. There is no folder copy now. A whole
folder that would have to cross drives is refused; files are copied.

**A copy proves what it can.** It is written under a temporary name and published
with a rename that cannot replace anything; the original is removed last, and
only if it is still the object that was copied. Verification establishes that
the destination holds as many bytes as were read from the source handle, that
the source still has the same identity, size and modification time as when the
copy began, and that the copy, re-read in full from the destination, hashes
to exactly what was read from the source — at every size. A stop requested
during verification leaves the original untouched and publishes nothing.
Folders are never copied across drives; that is refused with a reason.
Extended attributes, alternate data streams and ACLs are not carried across.

**Nothing is overwritten, enforced by the kernel.** Every move and every restore
is published with `renameat2(RENAME_NOREPLACE)`, `renamex_np(RENAME_EXCL)` or
`MoveFileEx` without `REPLACE_EXISTING`. Choosing a free name first is not a
guarantee — something can appear between choosing and using it — so a restore
that collides simply tries the next name.

**Acting on what was checked.** On Windows a file is opened, its identity
compared with what was recorded, and then renamed or deleted *through that
handle*, so the check and the action are about one object. Unix has no
rename-by-descriptor: the cache root is held open, children are reached through
it with `O_NOFOLLOW` (so swapping something above the root for a link cannot
redirect anything), and the moved object is checked again afterwards and put
back if it turns out to be the wrong one. A same-name swap in the instant
between check and rename is *narrowed, not closed*. Where a platform cannot
make a guarantee the file is skipped, not moved on trust.

### Shared folders are cleaned file by file

A cache root — the Windows temp directory, a shader cache — belongs to every
program on the machine. The folder is never what moves. At scan time its
eligible files are written to disk as a **reviewed set**: relative path, size,
modification time, creation time where the filesystem has one, and file
identity. A move works through that set, re-checking each file immediately
before it goes, and skips (and counts) any that changed, vanished, are in use
or are not what was recorded. A file that appeared after the scan is not in
the set, whatever timestamps it carries. Scuttle's own database, drawer and
webview folders are excluded, and in the temp directory files touched in the
last day are left out, because a shared folder is full of files in use right
now.

The reviewed set is what makes the old "changed since the scan — rummage again"
refusal go away for live folders: the folder's own timestamp changes whenever
anything inside it does, so comparing it could never pass. What remains of a
folder after a partial run is marked as needing review, not given an estimated
size, and a retry only ever continues the same set.

Restoring is a different operation and shares none of that policy: it puts back
exactly the files recorded for the item, whatever their timestamps, and only
ever refuses to overwrite.

### Interrupted moves

Every batch is written to the drawer's checkpoint *before* it happens and settled
after. On the next start, before anything is allowed to expire, Scuttle compares
what the checkpoint intended with what is on disk. A file that is in the drawer
and gone from its place moved; one still in place did not; one present in both —
a verified copy whose original was not yet removed — is **not** counted as moved,
is kept, and flags the record for a person to look at. A half-written copy is
never presented as held content. Nothing uncertain is deleted, and a record that
needs a look is skipped by the expiry sweep.

### Quitting while files move

Quitting while a move, restore or empty is running asks the move to stop at its
next safe point (between files, or between chunks of a copy), shows the window
with a sentence saying so, and exits once the work has settled — within two
minutes at most. A second quit exits at once, and the journal above settles
anything that was in flight at the next start.

### One thing at a time

Moving, restoring, emptying the drawer, removing an item, scanning, refreshing
findings and keeping or ignoring a finding are mutually exclusive, enforced in
the core. Reads (the findings, the drawer listing, the space overview) are never
blocked, so the interface stays usable while work runs. This is deliberately
coarse: proving that two operations touch unrelated files means
enforcing ownership of every record and path, which is more machinery than the
guarantee is worth yet.

## What we accept

Scuttle will miss things. A cache with no rule written for it, a leftover
folder with an unrecognisable name, a duplicate below the size threshold. Those
are the right failures. The alternative — a heuristic broad enough to catch
them — is the heuristic that eventually deletes something that mattered.
