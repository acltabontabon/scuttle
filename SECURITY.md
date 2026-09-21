# Security policy

## Reporting a vulnerability

Please report security issues privately through
[GitHub's private vulnerability reporting](https://github.com/acltabontabon/scuttle/security/advisories/new)
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
5. Scuttle's only network request is the update check (and the download an
   update is chosen for), to GitHub Releases over HTTPS. It sends nothing about
   the machine's files, and it can be turned off.
6. Nothing downloaded is run unless its signature verifies against the key
   compiled into the application, and installing never starts while a file
   operation is running.

## Updates

Scuttle checks for updates on its own (switchable), downloads only when asked,
and restarts only when asked. Every update is verified with a minisign
signature — the public key is in the application, the private key only in CI
secrets — and the signature must be bound to the version the manifest
announces, so a tampered manifest cannot pair a new version number with an
older signed file. See [`docs/updates.md`](docs/updates.md) for the design and
for what is and is not protected.

Updates are not code-signed in the operating-system sense: macOS builds are
ad-hoc signed and not notarized, the Windows installer is unsigned, and the
update signature is a separate thing that replaces neither. A report that the
update key could be misused, or that an update could be installed without its
signature verifying, is a vulnerability.
