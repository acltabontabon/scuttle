# Privacy

Scuttle looks at everything on your computer. This document is about what it
does with that.

## The short version

Nothing leaves the machine. Scuttle makes no network requests of any kind:
no accounts, no cloud analysis, no telemetry, no crash reporting, no update
check. The dependency tree contains no HTTP client.

## What Scuttle looks at

**Metadata, nearly always.** Paths, file names, sizes, timestamps, extensions,
directory structure, and information the platform already publishes about
installed applications and running processes.

**Contents, in three specific cases:**

1. **Duplicate detection** hashes file contents (BLAKE3) to confirm that two
   same-size files really are identical. The hash is compared and, for
   quarantined files under 128 MB, stored. The contents are never stored.
2. **Screenshot grouping** decodes images to compute a 64-bit perceptual hash,
   to find near-identical captures. The image is decoded, hashed and dropped.
3. **Application metadata** — `Info.plist` on macOS, the uninstall registry on
   Windows — is read to find out what is installed.

Scuttle does not read documents to classify them. A folder is judged by the
shape of what is in it — extensions, names, counts — not by opening the files.
Knowing a folder holds seventeen `.sav` files is enough to be careful, and
opening them would tell us nothing more useful.

## What Scuttle never reads

The protected-path table is pruned at traversal, not just filtered from
results. These are never opened, never measured, never counted:

- SSH and GPG keys, cloud credentials, password stores
- Password vaults (`.kdbx`, `.opvault`, `.keychain`, …)
- Browser profiles
- Version-control internals
- Cloud-sync folders (Dropbox, OneDrive, iCloud Drive, Google Drive, …)
- Device backups
- Sandboxed application containers
- The operating system's own directories

See [safety.md](safety.md) for the full table.

## What is stored, and where

A single SQLite database:

- macOS — `~/Library/Application Support/Scuttle/scuttle.db`
- Windows — `%LOCALAPPDATA%\Scuttle\scuttle.db`

It holds scan runs, findings, evidence, ignore lists, quarantine records,
cleanup history and settings. Findings include full paths — they have to, since
that's what the actions operate on.

It does **not** hold an index of your filesystem. Scuttle is not Spotlight.
Only files that became findings are recorded, and old scans are pruned on
launch.

Quarantined items live beside it under `Quarantine/`, one directory per item,
with a JSON manifest recording where each came from and why.

To remove everything Scuttle knows, quit it and delete that directory. (Restore
anything you still want from the drawer first — deleting the folder deletes the
quarantined files with it.)

## Logging

File paths expose usernames, project names, client names and personal content.
So:

- **Paths are not logged at normal levels.** Scan problems are recorded as
  counts by category — "12 permission denied" — not as lists of paths.
- Non-fatal traversal errors keep only the final path component, never the
  full path.
- File contents are never logged.
- Detailed paths appear only in the dry-run report, which you have to ask for
  explicitly, and in `SCUTTLE_LOG=debug` output.

Logs go to stderr. Scuttle does not write a log file.

## Clipboard, notifications, background activity

None of these. Scuttle does not read the clipboard, does not send
notifications, and does nothing when you are not looking at it. There is no
background scanning in this version. If ambient rummaging is added later it
will be opt-in and it will not nag.

## Permissions

On macOS, reading Downloads, Desktop and Documents triggers the system's own
consent prompts. Scuttle asks for nothing beyond that — no Full Disk Access, no
accessibility permissions. Denying a prompt means Scuttle sees less; it will
report the places it could not read rather than failing.

On Windows, Scuttle reads the uninstall registry hives under `HKLM` and `HKCU`
for installed-application names, and runs `tasklist` to see what is running. It
requires no elevation.
