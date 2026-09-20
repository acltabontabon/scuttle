# Security policy

## Reporting a vulnerability

Please report security issues privately through
[GitHub's private vulnerability reporting](https://github.com/scuttle-app/scuttle/security/advisories/new)
rather than in a public issue.

Please include what you were able to make Scuttle do, the smallest filesystem
layout that reproduces it, and the platform and version. You'll get an
acknowledgement within a few days.

## What counts as a vulnerability here

Scuttle's threat model is unusual for a desktop utility: the thing being
protected is the user's own filesystem, and the most likely attacker is a bug.

Treat these as security issues:

- **Escaping the safety layer.** Any path that reaches a destructive operation
  despite being protected, too shallow, outside the scanned roots, or reached
  through a symlink, junction or other reparse point.
- **Time-of-check to time-of-use.** Any sequence where a file can be swapped
  between validation and the operation.
- **Restore overwriting data.** Restore must never replace a file that exists
  at the destination.
- **Traversal escaping its roots**, including through links, mount points, or
  `..` handling.
- **Data leaving the machine.** Scuttle has no network code; any path by which
  file contents, paths or metadata could leave the machine is a serious bug.
- **Privacy in logs.** Full filesystem paths appearing in logs at normal
  levels. Paths identify people, projects and clients.

Not security issues, though still worth reporting as bugs:

- A detector classifying something wrongly but *conservatively* — surfacing a
  file it shouldn't have, or refusing one it could have acted on.
- Permission-denied errors on directories Scuttle can't read.

## Design commitments

These hold across releases, and breaking one is a vulnerability:

1. The webview cannot name a path to act on. Every destructive command takes an
   id, and the core re-derives the path and re-validates it.
2. Every destructive operation re-checks the live filesystem after the scan:
   existence, type, size, modification time, directory child count, protected
   rules, containment and links.
3. Deletion is never a side effect. Findings move to quarantine; permanent
   removal requires a separate, explicit request, or the retention window
   closing.
4. Restore never overwrites.
5. Scuttle makes no network requests.

## Updates

Scuttle has no auto-updater yet. When one is added it will verify signatures
before executing anything downloaded; see `docs/packaging.md`.
