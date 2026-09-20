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
is this the kind of finding that may be acted on?
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

## Confidence is not risk

The single most important distinction in the model.

**Confidence** is how sure Scuttle is about its *classification*. **Risk** is
what it costs if Scuttle is wrong.

Scuttle can be completely certain that `archive.zip` is two years old and
completely unable to justify recommending its deletion. The action ladder
reflects this:

| Risk | High confidence | Medium | Low |
| --- | --- | --- | --- |
| Low | Quarantine | Review | Inspect only |
| Moderate | Review | Inspect only | Inspect only |
| High | Inspect only | Inspect only | Inspect only |
| Protected | Inspect only | Inspect only | Inspect only |

High risk is never actionable, at any confidence. Note also that the levels the
user sees are coarse — High, Medium, Low. Internal weighted scoring is fine;
"Confidence: 83.742%" is a tell of software that is guessing.

## Quarantine first

Permanent deletion is not the primary workflow and never happens as a side
effect.

```
found → quarantine → 7 / 14 / 30 days → permanent removal
```

Each held item keeps its original path, its size, a content hash where cheap,
and a frozen snapshot of the evidence it was shown with. If a detector changes
its mind in a later version, the record of what the user was told does not.

Restore never overwrites. If something now occupies the original path, the
restored item goes beside it as `name (restored).ext` and the caller is told.

## What we accept

Scuttle will miss things. A cache with no rule written for it, a leftover
folder with an unrecognisable name, a duplicate below the size threshold. Those
are the right failures. The alternative — a heuristic broad enough to catch
them — is the heuristic that eventually deletes something that mattered.
