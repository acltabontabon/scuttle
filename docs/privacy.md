# Privacy

Scuttle looks at everything on your computer. This document is about what it
does with that.

## The short version

Nothing about you or your files leaves the machine. No accounts, no cloud
analysis, no telemetry, no crash reporting.

Scuttle makes **one kind of network request**: it asks GitHub whether a newer
version exists. That request is a plain HTTPS `GET` for a small file at
`github.com/acltabontabon/scuttle/releases/…`, made shortly after Scuttle starts
and about once a day while it stays running. It carries no identifier and nothing
about your files; like any web request it reveals your IP address to GitHub and
the standard `User-Agent` of the update library. Nothing is downloaded until you
choose to, and downloading fetches a release file from the same place. You can
turn the checks off in Settings → About (*Automatically check for updates*);
*Check for updates* then still works when you ask.

This is the reason the dependency tree now contains an HTTP client: it is there
for updates and for nothing else, and every update it fetches is verified
against a signature before it is used.

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

Scuttle does not read the clipboard.

Everything below is **off by default**. With all of it off, Scuttle behaves
exactly as it always has: closing the window quits it, and it does nothing at
all when you are not looking at it.

### Staying in the menu bar

"Keep Scuttle in the menu bar" (on Windows, "in the system tray") makes closing
the window hide it instead of quitting. Quitting — from the icon's own menu,
with ⌘Q, or by logging out or shutting down — always quits, and Scuttle never
delays a shutdown. Quitting stops its background work with it: there is no
helper process, no service and no daemon.

### Background checks

"Check occasionally in the background" is a separate setting, and needs the
first one. When it is on, Scuttle looks around at most once a day.

**What a background check looks at is deliberately less than a rummage.** It
uses the same folders, the same ignore lists and the same safety rules, but it
reads no file contents at all — so it cannot find duplicates or near-identical
screenshots, both of which need hashing or decoding. The findings screen says
so, because an empty pile should not be mistaken for a pile that was searched.

It never moves, deletes or selects anything. Discovery and modification stay
separate; a check that found something has found something, and that is all.

### How Scuttle decides it is a reasonable moment

It defers when the window is open, when Scuttle is already busy, when the
machine is on battery, in a low-power or battery-saving mode, reporting
thermal pressure, or (on macOS) carrying a high load average. Those come from
`pmset` and `GetSystemPowerStatus`, which need no permission and no elevation.

**This is not idle detection, and Scuttle does not claim it is.** A quiet
machine on mains power is not proof that you have stepped away. Aggregate idle
time is published without a permission prompt on both platforms, and Scuttle
deliberately does not collect it, because no honest claim can be built on a
number that cannot tell a reader from an empty chair. Every signal here is a
reason to *defer*, never evidence about you.

Scuttle does not watch the filesystem, keep a queue of missed checks, retry a
skipped one, prevent the machine sleeping, wake it, or run a check straight
after it wakes.

### Notifications

Off by default, and a third separate setting. Permission is asked for at the
moment you turn it on, and refusing it leaves everything else working.

At most one summary a day, only when something genuinely new has turned up,
and only ever counts by category — "3 old installers worth a look". Filenames
and paths are never in it, because a summary can appear on a lock screen.
These are ordinary system notifications, so Focus and Do Not Disturb apply
normally; Scuttle has no pop-up of its own.

### Permissions this adds

None beyond ordinary notification permission, and only if you ask for
notifications. Specifically, Scuttle does not request Accessibility, Input
Monitoring or Screen Recording, installs no keyboard or mouse hooks, reads no
window titles, records no screen, and does not enumerate running applications
to infer what you are doing. Background mode needs no administrator rights.

## Permissions

On macOS, reading Downloads, Desktop and Documents triggers the system's own
consent prompts. Scuttle asks for nothing beyond that — no Full Disk Access, no
accessibility permissions. Denying a prompt means Scuttle sees less; it will
report the places it could not read rather than failing.

On Windows, Scuttle reads the uninstall registry hives under `HKLM` and `HKCU`
for installed-application names, and runs `tasklist` to see what is running. It
requires no elevation.
