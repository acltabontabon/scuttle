//! Cleaning the contents of a shared folder, restoring exactly what was taken,
//! and settling whatever a crash left half-done.
//!
//! A cache root such as the Windows temp directory belongs to everything that
//! runs. Moving the folder would take the files programs are using out from
//! under them, and refusing the whole finding because *one* file changed would
//! make it impossible to clean at all. So the folder is never the unit:
//!
//! * At scan time its eligible files were recorded as a reviewed set
//!   ([`crate::scanning::snapshot`]).
//! * A move works through that set in order, re-checking each file against what
//!   was recorded immediately before moving it, and moves each one by itself.
//!   A file that changed, vanished or is held open is skipped and counted; the
//!   rest go. Nothing that is not in the set is ever touched.
//! * The folder stays where it is.
//!
//! Every batch is written to the drawer's checkpoint *before* it happens, and
//! settled after. That is what makes a crash survivable: on the next start,
//! [`Quarantine::reconcile`] can look at what the checkpoint says was
//! intended and at what is actually on disk, and tell "moved", "not moved" and
//! "copied but the original is still there" apart — without deleting anything
//! it is not certain about.

use std::path::{Path, PathBuf};

use super::fsx::{EntryKind, Identity, PathSource, PinnedDir, Source};
use super::transfer::{
    self, Ctl, FailureKind, FsFailure, IssueLog, Method, Moved, Phase, Tally, PART_SUFFIX,
};
use super::{restored_name, write_manifest, Quarantine};
use crate::model::CleanupCandidate;
use crate::safety::{self, ActionContext};
use crate::storage::{
    journal, QuarantineRecord, QuarantineStatus, RecordMode, SnapshotEntry, SnapshotState,
};
use crate::Result;

/// How many files are checkpointed, moved and settled together.
const BATCH: usize = 256;

/// Told about a run as it goes. Both calls must be cheap; a caller that wants
/// throttling does it on its side.
pub trait MoveObserver {
    /// Counts so far, after each file.
    fn tally(&self, tally: &Tally);
    /// Every file has been dealt with and the drawer's records are about to be
    /// settled.
    fn settling(&self) {}
}

/// Observe a run with a closure.
pub fn observe<F: Fn(&Tally)>(f: F) -> FnObserver<F> {
    FnObserver(f)
}

pub struct FnObserver<F>(F);

impl<F: Fn(&Tally)> MoveObserver for FnObserver<F> {
    fn tally(&self, tally: &Tally) {
        (self.0)(tally)
    }
}

/// Observe nothing.
pub struct Unobserved;

impl MoveObserver for Unobserved {
    fn tally(&self, _tally: &Tally) {}
}

/// What a run over a reviewed set will do, worked out before anything moves.
#[derive(Debug, Clone)]
pub struct ContentsPlan {
    pub root: PathBuf,
    pub files: u64,
    pub bytes: u64,
}

/// What a run over a reviewed set did.
#[derive(Debug, Clone)]
pub struct ContentsOutcome {
    /// The record that now holds what moved. `None` when nothing did.
    pub record: Option<QuarantineRecord>,
    pub tally: Tally,
    pub issues: IssueLog,
    pub cancelled: bool,
    /// What the run set out to do, from the recorded set.
    pub planned_files: u64,
    pub planned_bytes: u64,
    /// Files of the set still not moved when the run ended.
    pub unmoved: u64,
}

/// What restoring a record put back.
#[derive(Debug, Clone, Default)]
pub struct ContentsRestore {
    pub restored: u64,
    pub renamed: u64,
    pub remaining: u64,
    pub issues: IssueLog,
}

/// What reconciling after an interrupted run found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciled {
    /// Interrupted moves and restores that were settled.
    pub settled: usize,
    /// Records that need a person to look, because something uncertain was
    /// left in place rather than deleted.
    pub attention: usize,
    /// Folders in the drawer that no record owns. Reported, never removed.
    pub orphan_cells: usize,
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Remove the empty folders of a cell from the bottom up, then its manifest
/// and the cell itself. Refuses to remove anything that is not an empty folder
/// or Scuttle's own manifest, so it cannot take a file with it.
pub(super) fn remove_empty_cell(cell: &Path) {
    fn empties(dir: &Path) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                empties(&path);
                let _ = std::fs::remove_dir(&path);
            }
        }
    }
    empties(cell);
    let _ = std::fs::remove_file(cell.join("scuttle-manifest.json"));
    let _ = std::fs::remove_dir(cell);
}

/// Does this cell hold any half-written copy?
fn has_partial_copies(dir: &Path) -> bool {
    let Ok(read) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if has_partial_copies(&path) {
                return true;
            }
        } else if path.to_string_lossy().ends_with(PART_SUFFIX) {
            return true;
        }
    }
    false
}

fn join_rel(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(base.to_path_buf(), |p, c| p.join(c))
}

/// The state a folder-cache lookup keeps between files: consecutive entries of
/// a sorted set almost always share a parent.
struct Walkers {
    parent_rel: Option<String>,
    parent: Option<PinnedDir>,
    made_dir: Option<PathBuf>,
}

impl Quarantine {
    /// Decide, without touching anything, whether a shared folder can be
    /// cleaned from its reviewed set, and what that would involve.
    ///
    /// This is the "checking" of a move: the safety gate, the reviewed set's
    /// state, and — when the drawer is on another volume and files will be
    /// copied — whether there is room.
    pub fn plan_contents(
        &self,
        candidate: &CleanupCandidate,
        ctx: &ActionContext<'_>,
        retry: bool,
    ) -> Result<ContentsPlan> {
        let root = safety::authorize_contents(candidate, ctx)?;

        let info = self.store.snapshot_info(&candidate.id)?;
        match (info.state, retry) {
            (SnapshotState::Complete, _) | (SnapshotState::NeedsRefresh, true) => {}
            (SnapshotState::NeedsRefresh, false) => {
                return Err(crate::ScuttleError::Stale(
                    "Some of this changed since it was reviewed. Review it again.".into(),
                ))
            }
            (SnapshotState::Truncated, _) => {
                return Err(crate::ScuttleError::Refused(
                    "There are too many files here to review one by one, so Scuttle won't \
                     move them."
                        .into(),
                ))
            }
            (SnapshotState::None, _) => {
                return Err(crate::ScuttleError::Stale(
                    "This was found before Scuttle reviewed folders file by file. \
                     Review it again."
                        .into(),
                ))
            }
        }

        let (files, bytes) = self.store.snapshot_actionable(&candidate.id)?;

        // If this will cross a drive boundary it will copy, and copying needs
        // room. Find out before writing anything.
        if files > 0 && !super::volume::same_volume_hint(&root, &self.root) {
            if let Some(free) = super::volume::free_bytes(&self.root) {
                if free < bytes {
                    return Err(FsFailure::new(
                        FailureKind::NoSpace,
                        Phase::Prepare,
                        candidate.display_name.clone(),
                    )
                    .into());
                }
            }
        }
        Ok(ContentsPlan { root, files, bytes })
    }

    /// Move the reviewed files of a shared folder into the drawer.
    ///
    /// `retry` continues a set that already had a run: it processes what was
    /// never attempted and what failed in a way trying again might fix, and
    /// nothing else. It never widens the set.
    pub fn hold_contents(
        &self,
        candidate: &CleanupCandidate,
        ctx: &ActionContext<'_>,
        now_unix: i64,
        retry: bool,
        ctl: &Ctl<'_>,
        observer: &dyn MoveObserver,
    ) -> Result<ContentsOutcome> {
        let ContentsPlan {
            root,
            files: planned_files,
            bytes: planned_bytes,
        } = self.plan_contents(candidate, ctx, retry)?;
        if planned_files == 0 {
            return Ok(ContentsOutcome {
                record: None,
                tally: Tally::default(),
                issues: IssueLog::default(),
                cancelled: false,
                planned_files: 0,
                planned_bytes: 0,
                unmoved: self.store.snapshot_unmoved(&candidate.id)?,
            });
        }

        let root_pin = PinnedDir::open(&root)
            .map_err(|e| FsFailure::from_io(&e, Phase::Check, candidate.display_name.clone()))?;

        let folder = root
            .file_name()
            .ok_or_else(|| crate::ScuttleError::Refused("That folder has no name.".into()))?
            .to_os_string();
        let id = uuid::Uuid::new_v4().to_string();
        let cell = self.root.join(&id);
        let base = cell.join(&folder);
        std::fs::create_dir_all(&base)?;

        let mut record = QuarantineRecord {
            id: id.clone(),
            finding_id: Some(candidate.id.clone()),
            original_path: root.clone(),
            stored_path: base.clone(),
            display_name: candidate.display_name.clone(),
            category: candidate.category,
            size: 0,
            content_hash: None,
            evidence: candidate.evidence.clone(),
            quarantined_unix: now_unix,
            expires_unix: now_unix + i64::from(self.retention_days) * 86_400,
            status: QuarantineStatus::Moving,
            resolved_unix: None,
            mode: RecordMode::Contents,
            item_count: 0,
            attention: false,
        };

        // Intent first: the manifest and the record exist before a single file
        // moves, so anything that moves has somewhere to be accounted for.
        write_manifest(&cell, &record, "moving");
        if let Err(err) = self.store.insert_quarantine(&record) {
            remove_empty_cell(&cell);
            return Err(err);
        }

        let mut tally = Tally::default();
        let mut issues = IssueLog::default();
        let mut cancelled = false;

        let worked: Result<()> = (|| {
            let mut walkers = Walkers {
                parent_rel: None,
                parent: None,
                made_dir: None,
            };
            let mut after: Option<String> = None;
            loop {
                let entries = self
                    .store
                    .snapshot_batch(&candidate.id, after.as_deref(), BATCH)?;
                let Some(last) = entries.last() else { break };
                after = Some(last.rel.clone());

                // Write-ahead: what is about to be attempted.
                let intent: Vec<(String, u64)> =
                    entries.iter().map(|e| (e.rel.clone(), e.size)).collect();
                self.store.journal_add(&id, &intent)?;

                let mut done: Vec<(String, &str)> = Vec::new();
                let mut dropped: Vec<String> = Vec::new();
                let mut outcomes: Vec<(String, &str)> = Vec::new();

                for entry in &entries {
                    if ctl.cancelled() {
                        cancelled = true;
                        dropped.push(entry.rel.clone());
                        continue;
                    }
                    match self.move_reviewed(entry, &root_pin, &root, &base, ctx, &mut walkers, ctl)
                    {
                        Ok(moved) => {
                            tally.processed += 1;
                            tally.moved += 1;
                            tally.moved_bytes += entry.size;
                            if moved.method == Method::Copied {
                                tally.copied_bytes += moved.bytes;
                            }
                            done.push((entry.rel.clone(), journal::DONE));
                            outcomes.push((entry.rel.clone(), "moved"));
                        }
                        Err(failure) if failure.kind == FailureKind::Cancelled => {
                            cancelled = true;
                            dropped.push(entry.rel.clone());
                        }
                        Err(failure) => {
                            tally.record_failure(&failure);
                            tracing::debug!(
                                kind = failure.kind.slug(),
                                phase = ?failure.phase,
                                os_code = ?failure.os_code,
                                "a reviewed file was not moved"
                            );
                            issues.add(&failure);
                            dropped.push(entry.rel.clone());
                            outcomes.push((entry.rel.clone(), failure.kind.slug()));
                        }
                    }
                    observer.tally(&tally);
                }

                // Settle the batch: what moved is now counted, what did not is
                // no longer pending, and the set remembers each outcome.
                self.store.journal_set(&id, &done)?;
                self.store.journal_remove(&id, &dropped)?;
                self.store.set_entry_outcomes(&candidate.id, &outcomes)?;
                if cancelled {
                    break;
                }
            }
            Ok(())
        })();

        observer.settling();
        // Whatever happened above, settle the record from the checkpoint. If
        // the database itself failed, the record stays `moving` and the next
        // start reconciles it.
        let settled = self.settle_moved(&mut record, &cell, now_unix);

        let unmoved = self.store.snapshot_unmoved(&candidate.id).unwrap_or(0);
        if unmoved == 0 {
            let _ = self.store.forget_candidate(&candidate.id);
        } else if self
            .store
            .snapshot_attempted_unmoved(&candidate.id)
            .unwrap_or(0)
            > 0
        {
            // Something was tried and did not move. The folder has moved on
            // since it was reviewed; Scuttle will not guess at what is left.
            let _ = self
                .store
                .set_snapshot_state(&candidate.id, SnapshotState::NeedsRefresh);
        } else if let Ok((_, bytes)) = self.store.snapshot_actionable(&candidate.id) {
            // Stopped early, nothing wrong: what is left is exactly the
            // unattempted part of the set, and its size is known.
            let _ = self.store.set_finding_size(&candidate.id, bytes);
        }

        worked?;
        let held = settled?;
        Ok(ContentsOutcome {
            record: held.then_some(record),
            tally,
            issues,
            cancelled,
            planned_files,
            planned_bytes,
            unmoved,
        })
    }

    /// Bring one reviewed file across. Returns how it went.
    #[allow(clippy::too_many_arguments)]
    fn move_reviewed(
        &self,
        entry: &SnapshotEntry,
        root_pin: &PinnedDir,
        root: &Path,
        base: &Path,
        ctx: &ActionContext<'_>,
        walkers: &mut Walkers,
        ctl: &Ctl<'_>,
    ) -> std::result::Result<Moved, FsFailure> {
        let parts: Vec<&str> = entry.rel.split('/').collect();
        let Some((name, parents)) = parts.split_last() else {
            return Err(FsFailure::new(FailureKind::Unsafe, Phase::Check, "?"));
        };
        let item = (*name).to_string();
        let io_fail = |e: &std::io::Error, phase: Phase| FsFailure::from_io(e, phase, &item);

        // The gate's protected table applies to each file, not just the folder.
        if ctx.protected.is_protected(&join_rel(root, &entry.rel)) {
            return Err(FsFailure::new(FailureKind::Unsafe, Phase::Check, &item));
        }

        // Reach the file through the pinned folder, one `O_NOFOLLOW` step at a
        // time, so nothing above the folder can redirect it.
        let parent_rel = parents.join("/");
        if walkers.parent_rel.as_deref() != Some(parent_rel.as_str()) {
            let mut dir = root_pin
                .try_clone()
                .map_err(|e| io_fail(&e, Phase::Check))?;
            for part in parents {
                dir = dir.child_dir(part).map_err(|e| io_fail(&e, Phase::Check))?;
            }
            walkers.parent = Some(dir);
            walkers.parent_rel = Some(parent_rel);
        }
        let dir = walkers.parent.as_ref().expect("set just above");
        let source = dir.file(name);

        // Is it still what was reviewed? Tell "it changed" apart from "it is a
        // different file": both skip, but they are different stories.
        let now = source.identity().map_err(|e| io_fail(&e, Phase::Check))?;
        match now.kind {
            EntryKind::File => {}
            EntryKind::Link => {
                return Err(FsFailure::new(FailureKind::Unsafe, Phase::Check, &item))
            }
            _ => return Err(FsFailure::new(FailureKind::Replaced, Phase::Check, &item)),
        }
        let expected = Identity {
            file_id: entry.file_id.clone(),
            size: entry.size,
            mtime_ns: entry.mtime_ns,
            created_ns: entry.created_ns,
            kind: EntryKind::File,
        };
        if !now.is_same_state(&expected) {
            let same_object =
                matches!((&now.file_id, &expected.file_id), (Some(a), Some(b)) if a == b);
            let kind = if same_object {
                FailureKind::Stale
            } else {
                FailureKind::Replaced
            };
            return Err(FsFailure::new(kind, Phase::Check, &item));
        }

        // Destination inside the cell, mirroring the folder's structure.
        let dest_parent = join_rel(base, &parents.join("/"));
        if walkers.made_dir.as_deref() != Some(dest_parent.as_path()) {
            std::fs::create_dir_all(&dest_parent).map_err(|e| io_fail(&e, Phase::Prepare))?;
            walkers.made_dir = Some(dest_parent.clone());
        }
        let dest = dest_parent.join(name);

        transfer::move_source(&source, &dest, &expected, ctl)
    }

    /// Settle a record after a run, from the checkpoint. Returns whether the
    /// drawer now holds anything.
    fn settle_moved(
        &self,
        record: &mut QuarantineRecord,
        cell: &Path,
        now_unix: i64,
    ) -> Result<bool> {
        let (files, bytes) = self.store.journal_totals(&record.id, journal::DONE)?;
        if files == 0 {
            // Nothing moved, so there is nothing to keep a record of.
            self.store.delete_quarantine(&record.id)?;
            remove_empty_cell(cell);
            return Ok(false);
        }
        record.size = bytes;
        record.item_count = files;
        record.status = QuarantineStatus::Held;
        self.store.settle_record(
            &record.id,
            QuarantineStatus::Held,
            bytes,
            files,
            false,
            None,
        )?;
        write_manifest(cell, record, "held");
        self.log(now_unix, "quarantine", record, "held");
        Ok(true)
    }

    /// Put back exactly the files a record holds.
    ///
    /// Nothing here consults the scan, the reviewed set or any cleanup rule: a
    /// restore is "the files that were recorded, back where they came from",
    /// and the only rules are that it never overwrites and never leaves the
    /// drawer's cell.
    pub(super) fn restore_contents(
        &self,
        record: &QuarantineRecord,
        now_unix: i64,
        ctl: &Ctl<'_>,
    ) -> Result<ContentsRestore> {
        self.store
            .set_record_status(&record.id, QuarantineStatus::Restoring)?;

        let mut out = ContentsRestore::default();
        let base = &record.stored_path;
        let original = &record.original_path;

        let outcome: Result<()> = (|| {
            let mut after: Option<String> = None;
            loop {
                let entries =
                    self.store
                        .journal_page(&record.id, journal::DONE, after.as_deref(), BATCH)?;
                let Some(last) = entries.last() else { break };
                after = Some(last.rel.clone());

                let intent: Vec<(String, &str)> = entries
                    .iter()
                    .map(|e| (e.rel.clone(), journal::RESTORING))
                    .collect();
                self.store.journal_set(&record.id, &intent)?;

                let mut restored: Vec<(String, &str)> = Vec::new();
                let mut back: Vec<(String, &str)> = Vec::new();
                for entry in &entries {
                    if ctl.cancelled() {
                        back.push((entry.rel.clone(), journal::DONE));
                        continue;
                    }
                    let source = join_rel(base, &entry.rel);
                    let destination = join_rel(original, &entry.rel);
                    match restore_file(&source, &destination, ctl) {
                        Ok(renamed) => {
                            out.restored += 1;
                            if renamed {
                                out.renamed += 1;
                            }
                            restored.push((entry.rel.clone(), journal::RESTORED));
                        }
                        Err(failure) => {
                            issues_add(&mut out.issues, &failure);
                            back.push((entry.rel.clone(), journal::DONE));
                        }
                    }
                }
                self.store.journal_set(&record.id, &restored)?;
                self.store.journal_set(&record.id, &back)?;
                if ctl.cancelled() {
                    break;
                }
            }
            Ok(())
        })();

        let (remaining, bytes) = self
            .store
            .journal_totals(&record.id, journal::DONE)
            .unwrap_or((0, 0));
        out.remaining = remaining;
        if remaining == 0 {
            self.store.settle_record(
                &record.id,
                QuarantineStatus::Restored,
                0,
                0,
                record.attention,
                Some(now_unix),
            )?;
            if let Some(cell) = record.stored_path.parent() {
                if crate::safety::paths::is_strictly_within(cell, &self.root) {
                    remove_empty_cell(cell);
                }
            }
            self.log(now_unix, "restore", record, "restored");
        } else {
            // Some could not come back. What is left is still held, still
            // recorded, and still restorable.
            self.store.settle_record(
                &record.id,
                QuarantineStatus::Held,
                bytes,
                remaining,
                record.attention,
                None,
            )?;
        }
        outcome?;
        Ok(out)
    }

    /// Settle whatever an interrupted move or restore left behind.
    ///
    /// Run once at startup, before anything is allowed to expire. It compares
    /// what the checkpoint says was intended with what is on disk, and:
    ///
    /// * finishes accounting for what really did move;
    /// * forgets what really did not;
    /// * keeps, and flags, anything it cannot be sure of — a copy whose
    ///   original is still in place, a half-written copy, a folder no record
    ///   owns. **It never deletes an uncertain artifact.**
    pub fn reconcile(&self, now_unix: i64) -> Result<Reconciled> {
        let mut report = Reconciled::default();

        for record in self.store.records_with_status(QuarantineStatus::Moving)? {
            let attention = match record.mode {
                RecordMode::Whole => self.reconcile_whole_move(&record, now_unix)?,
                RecordMode::Contents => self.reconcile_contents_move(&record, now_unix)?,
            };
            report.settled += 1;
            if attention {
                report.attention += 1;
            }
        }
        for record in self
            .store
            .records_with_status(QuarantineStatus::Restoring)?
        {
            let attention = match record.mode {
                RecordMode::Whole => self.reconcile_whole_restore(&record, now_unix)?,
                RecordMode::Contents => self.reconcile_contents_restore(&record, now_unix)?,
            };
            report.settled += 1;
            if attention {
                report.attention += 1;
            }
        }

        // Cells nobody owns. Reported, never touched.
        if let (Ok(read), Ok(known)) = (std::fs::read_dir(&self.root), self.store.quarantine_ids())
        {
            let known: std::collections::HashSet<String> = known.into_iter().collect();
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    && uuid::Uuid::parse_str(&name).is_ok()
                    && !known.contains(&name)
                {
                    report.orphan_cells += 1;
                }
            }
            if report.orphan_cells > 0 {
                tracing::warn!(
                    count = report.orphan_cells,
                    "the drawer holds folders that no record owns; left untouched"
                );
            }
        }
        Ok(report)
    }

    fn cell_of(record: &QuarantineRecord) -> Option<&Path> {
        record.stored_path.parent()
    }

    fn reconcile_whole_move(&self, record: &QuarantineRecord, now_unix: i64) -> Result<bool> {
        let source = exists(&record.original_path);
        let stored = exists(&record.stored_path);
        let partial = Self::cell_of(record).is_some_and(has_partial_copies);
        match (source, stored) {
            // Moved.
            (false, true) => {
                self.store.settle_record(
                    &record.id,
                    QuarantineStatus::Held,
                    record.size,
                    1,
                    partial,
                    None,
                )?;
                Ok(partial)
            }
            // Never moved. Nothing of value was made.
            (true, false) if !partial => {
                self.store.settle_record(
                    &record.id,
                    QuarantineStatus::Abandoned,
                    0,
                    0,
                    false,
                    Some(now_unix),
                )?;
                self.store.delete_quarantine(&record.id)?;
                if let Some(cell) = Self::cell_of(record) {
                    remove_empty_cell(cell);
                }
                Ok(false)
            }
            // A verified copy is in the drawer and the original is still in
            // place, or a half-written copy is lying about. Keep everything,
            // count nothing as moved, and say so.
            _ => {
                self.store.settle_record(
                    &record.id,
                    QuarantineStatus::Held,
                    if stored { record.size } else { 0 },
                    u64::from(stored),
                    true,
                    None,
                )?;
                Ok(true)
            }
        }
    }

    fn reconcile_contents_move(&self, record: &QuarantineRecord, now_unix: i64) -> Result<bool> {
        let base = &record.stored_path;
        let original = &record.original_path;
        let mut uncertain = Self::cell_of(record).is_some_and(has_partial_copies);

        // Every entry still marked pending was intended but not confirmed.
        let mut after: Option<String> = None;
        loop {
            let page =
                self.store
                    .journal_page(&record.id, journal::PENDING, after.as_deref(), BATCH)?;
            let Some(last) = page.last() else { break };
            after = Some(last.rel.clone());
            let mut done = Vec::new();
            let mut copied = Vec::new();
            let mut gone = Vec::new();
            for entry in &page {
                let source = exists(&join_rel(original, &entry.rel));
                let stored = exists(&join_rel(base, &entry.rel));
                match (source, stored) {
                    (false, true) => done.push((entry.rel.clone(), journal::DONE)),
                    (true, true) => {
                        copied.push((entry.rel.clone(), journal::COPIED));
                        uncertain = true;
                    }
                    // Never moved, or gone from both: either way nothing is
                    // held for it.
                    _ => gone.push(entry.rel.clone()),
                }
            }
            self.store.journal_set(&record.id, &done)?;
            self.store.journal_set(&record.id, &copied)?;
            self.store.journal_remove(&record.id, &gone)?;
        }

        let (files, bytes) = self.store.journal_totals(&record.id, journal::DONE)?;
        let (copies, _) = self.store.journal_totals(&record.id, journal::COPIED)?;
        if files == 0 && copies == 0 && !uncertain {
            self.store.settle_record(
                &record.id,
                QuarantineStatus::Abandoned,
                0,
                0,
                false,
                Some(now_unix),
            )?;
            self.store.delete_quarantine(&record.id)?;
            if let Some(cell) = Self::cell_of(record) {
                remove_empty_cell(cell);
            }
            return Ok(false);
        }
        self.store.settle_record(
            &record.id,
            QuarantineStatus::Held,
            bytes,
            files,
            uncertain,
            None,
        )?;
        Ok(uncertain)
    }

    fn reconcile_whole_restore(&self, record: &QuarantineRecord, now_unix: i64) -> Result<bool> {
        let stored = exists(&record.stored_path);
        if stored {
            // It never left the drawer, or a copy of it is out and it is still
            // here. Either way it is still held.
            let both = exists(&record.original_path);
            self.store.settle_record(
                &record.id,
                QuarantineStatus::Held,
                record.size,
                1,
                both || record.attention,
                None,
            )?;
            Ok(both)
        } else {
            self.store.settle_record(
                &record.id,
                QuarantineStatus::Restored,
                record.size,
                1,
                record.attention,
                Some(now_unix),
            )?;
            if let Some(cell) = Self::cell_of(record) {
                remove_empty_cell(cell);
            }
            Ok(false)
        }
    }

    fn reconcile_contents_restore(&self, record: &QuarantineRecord, now_unix: i64) -> Result<bool> {
        let base = &record.stored_path;
        let mut uncertain = false;
        let mut after: Option<String> = None;
        loop {
            let page =
                self.store
                    .journal_page(&record.id, journal::RESTORING, after.as_deref(), BATCH)?;
            let Some(last) = page.last() else { break };
            after = Some(last.rel.clone());
            let mut updates = Vec::new();
            for entry in &page {
                if exists(&join_rel(base, &entry.rel)) {
                    // Still in the drawer: not restored. If a copy went out
                    // under its own name the original cannot tell us, so a
                    // duplicate is possible and harmless.
                    updates.push((entry.rel.clone(), journal::DONE));
                } else {
                    updates.push((entry.rel.clone(), journal::RESTORED));
                }
            }
            self.store.journal_set(&record.id, &updates)?;
        }
        let (remaining, bytes) = self.store.journal_totals(&record.id, journal::DONE)?;
        if remaining == 0 {
            self.store.settle_record(
                &record.id,
                QuarantineStatus::Restored,
                0,
                0,
                record.attention,
                Some(now_unix),
            )?;
            if let Some(cell) = Self::cell_of(record) {
                remove_empty_cell(cell);
            }
        } else {
            self.store.settle_record(
                &record.id,
                QuarantineStatus::Held,
                bytes,
                remaining,
                record.attention,
                None,
            )?;
            uncertain = record.attention;
        }
        Ok(uncertain)
    }
}

fn issues_add(log: &mut IssueLog, failure: &FsFailure) {
    log.add(failure);
}

/// Put one recorded file back, beside whatever now occupies its place if need
/// be. Returns whether it had to be renamed.
fn restore_file(
    source: &Path,
    destination: &Path,
    ctl: &Ctl<'_>,
) -> std::result::Result<bool, FsFailure> {
    let item = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "this".into());
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| FsFailure::from_io(&e, Phase::Restore, &item))?;
    }
    let identity = super::fsx::identity_of(source)
        .map_err(|e| FsFailure::from_io(&e, Phase::Restore, &item))?;
    let file = PathSource(source);
    for attempt in 0..1000 {
        let candidate = restored_name(destination, attempt);
        match transfer::move_source(&file, &candidate, &identity, ctl) {
            Ok(_) => return Ok(attempt > 0),
            Err(failure) if failure.kind == FailureKind::Collision => continue,
            Err(mut failure) => {
                failure.phase = if failure.phase == Phase::Publish {
                    Phase::Restore
                } else {
                    failure.phase
                };
                return Err(failure);
            }
        }
    }
    Err(FsFailure::new(FailureKind::Collision, Phase::Restore, item))
}
