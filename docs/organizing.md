# Give things a home

New in Scuttle 0.2.0. It gathers loose screenshots and installers
into ordinary folders you choose, independently of cleanup eligibility. It does
not free storage or put files into the Drawer.

## Find, review, organize

1. Run **Rummage**. Scuttle checks files directly in Desktop, Downloads, and
   Documents; custom scan-root choices limit those locations. Existing folders
   are not recursively searched for organization.
2. Choose **Organize…** from Findings or a screenshot/installer pile. Recent
   screenshots are included, even when cleanup has nothing to suggest.
3. Select the files. Screenshots have local, bounded thumbnail previews. Choose
   **Change folder** to select another destination with the native folder picker.
4. Choose **Review organization**. Check the exact source and destination paths,
   including any numbered suffixes needed to avoid existing filenames.
5. Confirm **Organize**. Stop cancels remaining work at safe transfer checkpoints.
   Completed moves stay in history and can be undone.

Screenshots default to the platform Pictures folder under `Screenshots/YYYY-MM`,
using modification month in the computer’s local timezone. Files without a
usable date go in `Unknown date`. Choose **Together in one folder** for a flat
collection. Installers default to `Downloads/Installers` and keep their names.
The last destination and grouping are remembered after a successful move.
Configured destination folders are treated as already organized.

Inventory is capped at 1,000 opportunities and 10,000 directory entries, with a
five-second budget between calls. Partial results say so. Review handles up to
200 files per batch; handle those to see more. Unreadable locations are reported
without asking for additional permissions.

## A way back

Open **Organization history** from Findings, even when no cleanup piles remain.
Each batch has per-file outcomes, **Open folder**, and **Undo**. History and
recovery records survive restarts and never expire with the Drawer.

Undo checks both identity and a content hash. Edited, replaced, missing, or
protected files stay where they are. An occupied original path is never
overwritten; clear the conflict yourself and retry Undo. Missing original folders
are recreated when their paths are safe. Files left behind during organization
can be reviewed again; changed files need another rummage.

## What a move guarantees

The Rust core validates registered destinations and stored opportunity IDs;
the frontend cannot submit arbitrary source or destination paths to a move.
Sources, ignores, protected locations, installation areas, and destination
ancestors are checked again before execution. Links are refused. If a collision
appears after review, that item stays in place instead of silently receiving a
new name.

Moves use the same no-replace, identity-checked transfer primitives as the
Drawer. Across drives, Scuttle copies to a temporary file, verifies the entire
content, publishes without replacement, and removes the source last. This needs
space for both copies. Extended attributes, alternate data streams, and custom
access rules are not preserved across drives; the review explains this.

A durable journal is written before each move. Startup reconciles interrupted
operations without deleting either copy when the outcome is ambiguous. Such
rows say they need a look. The existing filesystem primitives narrow path races;
Scuttle does not claim a transaction spanning the filesystem and database.

Organization does not run during background checks. There is no OCR, content
classification, automatic folder rule, duplicate removal, or project relocation.

[Back to Scuttle](../README.md)
