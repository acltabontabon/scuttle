//! Quarantine: the drawer things go into before they are gone.
//!
//! Permanent deletion is not the primary workflow and never happens as a side
//! effect of anything. An item moves into a holding area, keeps a record of
//! where it came from and why it was moved, and stays recoverable until the
//! user says otherwise or its retention window closes.
//!
//! Two invariants this module is built around:
//!
//! * **Nothing is moved that has not passed [`crate::safety::authorize`].**
//!   The gate re-checks the live filesystem; a stored finding is never trusted.
//! * **Restore never overwrites.** If something now occupies the original
//!   path, the restored item is placed beside it under a new name and the
//!   caller is told where it went.

pub mod contents;
pub mod fsx;
pub mod transfer;
pub mod volume;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::ScuttleError;
use crate::model::{CleanupCandidate, TargetKind};
use crate::safety::{self, ActionContext};
use crate::storage::{HistoryEntry, QuarantineRecord, QuarantineStatus, Store};
use crate::Result;
use transfer::{hash_file, Ctl, FailureKind};

/// Files larger than this are not hashed on the way in: the check would cost
/// minutes and the path plus size plus timestamp already identify the item.
const MAX_HASH_BYTES: u64 = 128 * 1024 * 1024;

/// A small sidecar written next to each held item, so the drawer is still
/// legible if the database is ever lost.
#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    id: String,
    original_path: String,
    display_name: String,
    quarantined_unix: i64,
    expires_unix: i64,
    size: u64,
    reasons: Vec<String>,
    /// `whole`, or `contents` for the reviewed files of a shared folder.
    #[serde(default)]
    mode: crate::storage::RecordMode,
    /// `moving` while a move is under way or was interrupted; `held` once it
    /// settled. A person reading a cell with no database can tell which.
    #[serde(default)]
    state: String,
}

pub struct Quarantine {
    root: PathBuf,
    store: Arc<Store>,
    retention_days: u32,
}

/// Where a restored item actually ended up.
#[derive(Debug, Clone, Serialize)]
pub struct RestoreOutcome {
    pub path: PathBuf,
    /// True when the original location was occupied and a new name was used
    /// (for a folder's files: for at least one of them).
    pub renamed: bool,
    /// Files put back. One for a whole item.
    pub restored: u64,
    /// Files that could not be put back and are still in the drawer.
    pub remaining: u64,
    /// Why, grouped, for the ones that could not.
    pub failed: Vec<transfer::IssueGroup>,
}

/// What emptying the drawer actually managed to do.
///
/// `bytes` is deliberately counted from items that were really removed, not
/// from what was held when the sweep started: this number is shown to the user
/// as space they got back, so it has to be a claim about the disk rather than
/// about the database.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PurgeOutcome {
    pub removed: usize,
    pub bytes: u64,
    /// Anything that would not go, and why. One stubborn item is not a reason
    /// to abandon the rest of the drawer.
    pub failed: Vec<PurgeFailure>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PurgeFailure {
    pub display_name: String,
    pub reason: String,
}

impl Quarantine {
    pub fn new(root: PathBuf, store: Arc<Store>, retention_days: u32) -> Quarantine {
        Quarantine {
            root,
            store,
            retention_days,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Move a finding into the drawer.
    pub fn hold(
        &self,
        candidate: &CleanupCandidate,
        ctx: &ActionContext<'_>,
        now_unix: i64,
    ) -> Result<QuarantineRecord> {
        self.hold_controlled(candidate, ctx, now_unix, &Ctl::none())
    }

    /// [`Quarantine::hold`], stoppable: a copy across a volume boundary checks
    /// `ctl` between chunks.
    pub fn hold_controlled(
        &self,
        candidate: &CleanupCandidate,
        ctx: &ActionContext<'_>,
        now_unix: i64,
        ctl: &Ctl<'_>,
    ) -> Result<QuarantineRecord> {
        // A shared folder is never moved whole; its reviewed files are moved
        // one by one. Reaching this with one means the caller skipped that
        // path, so refuse rather than move the folder.
        if candidate.is_shared_contents() {
            return Err(ScuttleError::Refused(
                "That is a shared folder. Scuttle moves its files one by one, never the folder."
                    .into(),
            ));
        }

        // Everything is re-derived here. The stored finding is a request, not
        // an authorisation.
        let target = safety::authorize(candidate, ctx)?;

        let id = uuid::Uuid::new_v4().to_string();
        let cell = self.root.join(&id);
        std::fs::create_dir_all(&cell)?;

        let file_name = target
            .path
            .file_name()
            .ok_or_else(|| ScuttleError::Refused("That path has no name to preserve.".into()))?;
        let stored_path = cell.join(file_name);

        let content_hash =
            if target.kind == TargetKind::File && target.observed.size <= MAX_HASH_BYTES {
                hash_file(&target.path).ok()
            } else {
                None
            };

        // What the drawer holds is what was actually moved into it, which is
        // this one path — not what the finding it came from was worth.
        //
        // A group finding's `size` covers every redundant member, but holding
        // it moves only the anchor. Recording the finding's size made the
        // drawer claim bytes it did not have: a screenshot burst worth 400 MB
        // moved one 12 MB file and reported 400 MB held, so "empty the drawer
        // to free 400 MB" was a promise nothing could keep.
        //
        // Directories are the reason for the fallback: `observe` does not walk
        // them, so their observed size is zero and the finding's measured size
        // is the only real figure available.
        let size = match target.kind {
            TargetKind::File if target.observed.size > 0 => target.observed.size,
            _ => candidate.size,
        };
        let mut record = QuarantineRecord {
            id: id.clone(),
            finding_id: Some(candidate.id.clone()),
            original_path: target.path.clone(),
            stored_path: stored_path.clone(),
            display_name: candidate.display_name.clone(),
            category: candidate.category,
            size,
            content_hash,
            evidence: candidate.evidence.clone(),
            quarantined_unix: now_unix,
            expires_unix: now_unix + i64::from(self.retention_days) * 86_400,
            // Recorded as *moving* before anything moves. A crash between the
            // move and the record settling leaves a record that says what was
            // intended, which is what startup reconciliation reads.
            status: QuarantineStatus::Moving,
            resolved_unix: None,
            mode: crate::storage::RecordMode::Whole,
            item_count: 1,
            attention: false,
        };
        write_manifest(&cell, &record, "moving");
        if let Err(err) = self.store.insert_quarantine(&record) {
            contents::remove_empty_cell(&cell);
            return Err(err);
        }

        // The move itself. Anything that fails here leaves the original in
        // place, which is the right way round to fail. Only an empty cell is
        // removed: nothing published means nothing to lose.
        if let Err(failure) = transfer::move_entry(&target.path, &stored_path, ctl) {
            let _ = self.store.delete_quarantine(&id);
            contents::remove_empty_cell(&cell);
            return Err(failure.into());
        }

        // Settle. If that fails the item is already moved, so put it back
        // rather than leaving it stranded. The cell is dropped only once the
        // item is safely home: if putting it back fails too, the cell — and its
        // manifest and its `moving` record — is the trace left, and it stays.
        record.status = QuarantineStatus::Held;
        if let Err(err) =
            self.store
                .settle_record(&id, QuarantineStatus::Held, size, 1, false, None)
        {
            if transfer::move_entry(&stored_path, &target.path, &Ctl::none()).is_ok() {
                let _ = self.store.delete_quarantine(&id);
                contents::remove_empty_cell(&cell);
            } else {
                tracing::warn!("could not undo a move after a database failure; the cell was kept");
            }
            return Err(err);
        }
        write_manifest(&cell, &record, "held");

        let _ = self.store.forget_candidate(&candidate.id);
        self.log(now_unix, "quarantine", &record, "held");
        Ok(record)
    }

    /// Put something back. Never overwrites.
    pub fn restore(&self, id: &str, now_unix: i64) -> Result<RestoreOutcome> {
        let record = self.store.quarantine_record(id)?;
        match record.status {
            QuarantineStatus::Held => {}
            QuarantineStatus::Moving | QuarantineStatus::Restoring => {
                return Err(ScuttleError::Refused(
                    "That item is still being moved. Try again in a moment.".into(),
                ))
            }
            _ => {
                return Err(ScuttleError::Refused(
                    "That item has already left the drawer.".into(),
                ))
            }
        }
        // The stored path must be inside the drawer. A record pointing
        // anywhere else is not something to act on.
        if !safety::paths::is_strictly_within(&record.stored_path, &self.root) {
            return Err(ScuttleError::Refused(
                "That quarantine record does not point into the quarantine folder.".into(),
            ));
        }
        if !record.stored_path.exists() {
            return Err(ScuttleError::Stale(
                "The held copy is no longer there. Nothing to restore.".into(),
            ));
        }

        if record.mode == crate::storage::RecordMode::Contents {
            let done = self.restore_contents(&record, now_unix, &Ctl::none())?;
            return Ok(RestoreOutcome {
                path: record.original_path.clone(),
                renamed: done.renamed > 0,
                restored: done.restored,
                remaining: done.remaining,
                failed: done.issues.into_groups(),
            });
        }

        if let Some(parent) = record.original_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Marked before anything moves, so an interruption is recognisable.
        self.store.set_quarantine_restoring(id)?;
        let (destination, renamed) = match restore_into(&record.stored_path, &record.original_path)
        {
            Ok(placed) => placed,
            Err(err) => {
                // Nothing moved: it is still held.
                let _ = self.store.set_record_status(id, QuarantineStatus::Held);
                return Err(err);
            }
        };
        // Drop the emptied cell, and only ever a cell. A malformed record
        // whose stored path sat directly in the drawer root would otherwise
        // take the whole drawer with it.
        if let Some(cell) = record.stored_path.parent() {
            if safety::paths::is_strictly_within(cell, &self.root) {
                let _ = std::fs::remove_dir_all(cell);
            }
        }

        self.store
            .set_quarantine_status(id, QuarantineStatus::Restored, now_unix)?;
        self.log(
            now_unix,
            "restore",
            &record,
            if renamed {
                "restored beside an existing file"
            } else {
                "restored"
            },
        );

        Ok(RestoreOutcome {
            path: destination,
            renamed,
            restored: 1,
            remaining: 0,
            failed: Vec::new(),
        })
    }

    /// Remove something permanently. The only operation in Scuttle that
    /// destroys data, and it is never reached without an explicit request.
    pub fn purge(&self, id: &str, now_unix: i64) -> Result<()> {
        let record = self.store.quarantine_record(id)?;
        if record.status == QuarantineStatus::Removed {
            return Ok(());
        }
        if matches!(
            record.status,
            QuarantineStatus::Moving | QuarantineStatus::Restoring
        ) {
            return Err(ScuttleError::Refused(
                "That item is still being moved. Try again in a moment.".into(),
            ));
        }
        if !safety::paths::is_strictly_within(&record.stored_path, &self.root) {
            return Err(ScuttleError::Refused(
                "That quarantine record does not point into the quarantine folder.".into(),
            ));
        }

        let cell = record
            .stored_path
            .parent()
            .filter(|p| safety::paths::is_strictly_within(p, &self.root))
            .ok_or_else(|| ScuttleError::Refused("Malformed quarantine record.".into()))?;
        if cell.exists() {
            std::fs::remove_dir_all(cell)?;
        }

        self.store
            .set_quarantine_status(id, QuarantineStatus::Removed, now_unix)?;
        self.log(now_unix, "remove", &record, "removed permanently");
        Ok(())
    }

    /// Purge anything past its retention window. Returns how many went.
    pub fn sweep_expired(&self, now_unix: i64) -> Result<usize> {
        let expired = self.store.expired_quarantine(now_unix)?;
        let mut removed = 0;
        for record in expired {
            // One failure must not stop the sweep.
            if self.purge(&record.id, now_unix).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Empty the drawer: permanently remove everything currently held.
    ///
    /// This is the only operation that frees disk space in bulk, and it is the
    /// end of the one path a user takes to get space back. It is still built
    /// out of the same single-item `purge`, so every item passes the same
    /// containment check — being part of a batch grants nothing.
    pub fn purge_all(&self, now_unix: i64) -> Result<PurgeOutcome> {
        let mut outcome = PurgeOutcome::default();
        for record in self.store.held_quarantine()? {
            match self.purge(&record.id, now_unix) {
                Ok(()) => {
                    outcome.removed += 1;
                    outcome.bytes += record.size;
                }
                Err(error) => outcome.failed.push(PurgeFailure {
                    display_name: record.display_name.clone(),
                    reason: error.to_string(),
                }),
            }
        }
        Ok(outcome)
    }

    /// Total bytes currently held.
    pub fn held_bytes(&self) -> Result<u64> {
        Ok(self.store.held_quarantine()?.iter().map(|r| r.size).sum())
    }

    fn log(&self, now_unix: i64, action: &str, record: &QuarantineRecord, outcome: &str) {
        let _ = self.store.record_history(&HistoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            action: action.to_string(),
            display_name: record.display_name.clone(),
            category: record.category,
            size: record.size,
            at_unix: now_unix,
            outcome: outcome.to_string(),
        });
    }
}

/// The name to try for a restore. Attempt 0 is the original path itself;
/// `report.pdf` then becomes `report (restored).pdf`, `report (restored 2).pdf`
/// and so on. This only proposes names; that nothing is overwritten is
/// guaranteed by the no-replace publish that uses them.
pub(crate) fn restored_name(original: &Path, attempt: u32) -> PathBuf {
    if attempt == 0 {
        return original.to_path_buf();
    }
    let parent = original.parent().unwrap_or(Path::new("."));
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "restored".into());
    let extension = original
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let suffix = if attempt == 1 {
        " (restored)".to_string()
    } else {
        format!(" (restored {attempt})")
    };
    parent.join(format!("{stem}{suffix}{extension}"))
}

/// Put `stored` back at `original`, or beside it if something has arrived
/// there in the meantime. Never overwrites: each candidate name is published
/// with a rename that fails if the name is taken, and a collision — including
/// one that appears between choosing the name and using it — moves on to the
/// next name.
fn restore_into(stored: &Path, original: &Path) -> Result<(PathBuf, bool)> {
    for attempt in 0..1000 {
        let candidate = restored_name(original, attempt);
        match transfer::move_entry(stored, &candidate, &Ctl::none()) {
            Ok(_) => return Ok((candidate, attempt > 0)),
            Err(failure) if failure.kind == FailureKind::Collision => continue,
            Err(failure) => return Err(failure.into()),
        }
    }
    // Pathological, but never overwrite.
    let candidate = original.with_file_name(format!(
        "{} (restored {})",
        original
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        uuid::Uuid::new_v4()
    ));
    transfer::move_entry(stored, &candidate, &Ctl::none())
        .map(|_| (candidate, true))
        .map_err(Into::into)
}

pub(crate) fn write_manifest(cell: &Path, record: &QuarantineRecord, state: &str) {
    let manifest = Manifest {
        id: record.id.clone(),
        original_path: record.original_path.to_string_lossy().into_owned(),
        display_name: record.display_name.clone(),
        quarantined_unix: record.quarantined_unix,
        expires_unix: record.expires_unix,
        size: record.size,
        reasons: record.evidence.iter().map(|e| e.summary.clone()).collect(),
        mode: record.mode,
        state: state.to_string(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&manifest) {
        let _ = std::fs::write(cell.join("scuttle-manifest.json"), json);
    }
}

#[cfg(test)]
mod contents_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{ev, EvidenceKind};
    use crate::model::*;
    use crate::safety::ProtectedPaths;
    use std::fs;

    struct Fixture {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        roots: Vec<PathBuf>,
        protected: ProtectedPaths,
        store: Arc<Store>,
        quarantine: Quarantine,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home/rummager");
        fs::create_dir_all(home.join("Downloads")).unwrap();
        let protected = ProtectedPaths::for_home(&home);
        let store = Arc::new(Store::in_memory().unwrap());
        let quarantine = Quarantine::new(home.join(".scuttle/quarantine"), Arc::clone(&store), 14);
        Fixture {
            _tmp: tmp,
            roots: vec![home.clone()],
            home,
            protected,
            store,
            quarantine,
        }
    }

    impl Fixture {
        fn ctx(&self) -> ActionContext<'_> {
            ActionContext {
                protected: &self.protected,
                allowed_roots: &self.roots,
                // Test harnesses take the strict bidding, so every existing
                // assertion keeps meaning what it meant.
                bidding: crate::safety::Bidding::Scuttle,
            }
        }

        fn file(&self, rel: &str, contents: &str) -> PathBuf {
            let path = self.home.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }

        fn candidate(&self, path: &Path) -> CleanupCandidate {
            let fingerprint = safety::observe(path).unwrap();
            CleanupCandidate {
                id: "f1".into(),
                detector: "installers".into(),
                category: Category::Installers,
                target_kind: if fingerprint.is_dir {
                    TargetKind::Directory
                } else {
                    TargetKind::File
                },
                path: path.to_path_buf(),
                display_name: path.file_name().unwrap().to_string_lossy().into_owned(),
                associated_app: None,
                size: fingerprint.size,
                group_bytes: fingerprint.size,
                confidence: Confidence::High,
                risk: Risk::Low,
                recommended_action: RecommendedAction::Quarantine,
                evidence: vec![ev(EvidenceKind::InstallerFormat { ext: "dmg".into() })],
                remark: None,
                modified_unix: fingerprint.modified_unix,
                accessed_unix: None,
                created_unix: None,
                group: vec![],
                fingerprint,
            }
        }
    }

    #[test]
    fn holding_moves_the_file_and_leaves_a_record() {
        let f = fixture();
        let path = f.file("Downloads/Chrome.dmg", "installer bytes");
        let candidate = f.candidate(&path);

        let record = f.quarantine.hold(&candidate, &f.ctx(), 1000).unwrap();

        assert!(!path.exists(), "the original should have moved");
        assert!(
            record.stored_path.exists(),
            "the copy should be in the drawer"
        );
        assert_eq!(
            fs::read_to_string(&record.stored_path).unwrap(),
            "installer bytes"
        );
        assert_eq!(record.original_path, path);
        assert_eq!(record.expires_unix, 1000 + 14 * 86_400);
        assert_eq!(f.store.held_quarantine().unwrap().len(), 1);
    }

    #[test]
    fn holding_writes_a_readable_manifest_beside_the_item() {
        let f = fixture();
        let path = f.file("Downloads/Chrome.dmg", "bytes");
        let record = f
            .quarantine
            .hold(&f.candidate(&path), &f.ctx(), 1000)
            .unwrap();

        let manifest = record
            .stored_path
            .parent()
            .unwrap()
            .join("scuttle-manifest.json");
        let text = fs::read_to_string(manifest).unwrap();
        assert!(text.contains("Chrome.dmg"));
        assert!(
            text.contains("installer package"),
            "the reasons should be legible: {text}"
        );
    }

    #[test]
    fn a_protected_path_is_refused_and_stays_exactly_where_it_is() {
        let f = fixture();
        let key = f.file(".ssh/id_ed25519", "PRIVATE KEY");
        let mut candidate = f.candidate(&key);
        candidate.category = Category::Oddments;

        let err = f.quarantine.hold(&candidate, &f.ctx(), 1000).unwrap_err();
        assert_eq!(err.code(), "refused");
        assert!(key.exists(), "a refused hold must not move anything");
        assert!(f.store.held_quarantine().unwrap().is_empty());
    }

    #[test]
    fn a_stale_finding_is_refused_and_nothing_moves() {
        let f = fixture();
        let path = f.file("Downloads/Chrome.dmg", "bytes");
        let candidate = f.candidate(&path);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(&path, "different bytes entirely").unwrap();

        let err = f.quarantine.hold(&candidate, &f.ctx(), 1000).unwrap_err();
        assert_eq!(err.code(), "stale");
        assert!(path.exists());
    }

    #[test]
    fn restoring_puts_it_back_exactly_where_it_was() {
        let f = fixture();
        let path = f.file("Downloads/Chrome.dmg", "installer bytes");
        let record = f
            .quarantine
            .hold(&f.candidate(&path), &f.ctx(), 1000)
            .unwrap();

        let outcome = f.quarantine.restore(&record.id, 2000).unwrap();
        assert_eq!(outcome.path, path);
        assert!(!outcome.renamed);
        assert_eq!(fs::read_to_string(&path).unwrap(), "installer bytes");
        assert!(f.store.held_quarantine().unwrap().is_empty());
    }

    #[test]
    fn restoring_never_overwrites_something_that_arrived_in_the_meantime() {
        // The most dangerous moment in the whole flow.
        let f = fixture();
        let path = f.file("Downloads/report.pdf", "the original");
        let record = f
            .quarantine
            .hold(&f.candidate(&path), &f.ctx(), 1000)
            .unwrap();

        fs::write(&path, "something new and important").unwrap();

        let outcome = f.quarantine.restore(&record.id, 2000).unwrap();
        assert!(outcome.renamed);
        assert_ne!(outcome.path, path);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "something new and important",
            "the newer file must survive untouched"
        );
        assert_eq!(fs::read_to_string(&outcome.path).unwrap(), "the original");
        assert!(outcome.path.to_string_lossy().contains("(restored)"));
    }

    #[test]
    fn restoring_recreates_a_parent_folder_that_has_since_been_deleted() {
        let f = fixture();
        let path = f.file("Downloads/nested/Chrome.dmg", "bytes");
        let record = f
            .quarantine
            .hold(&f.candidate(&path), &f.ctx(), 1000)
            .unwrap();
        fs::remove_dir_all(f.home.join("Downloads/nested")).unwrap();

        let outcome = f.quarantine.restore(&record.id, 2000).unwrap();
        assert!(outcome.path.exists());
    }

    #[test]
    fn restoring_twice_is_refused_rather_than_duplicating() {
        let f = fixture();
        let path = f.file("Downloads/Chrome.dmg", "bytes");
        let record = f
            .quarantine
            .hold(&f.candidate(&path), &f.ctx(), 1000)
            .unwrap();
        f.quarantine.restore(&record.id, 2000).unwrap();

        let err = f.quarantine.restore(&record.id, 3000).unwrap_err();
        assert_eq!(err.code(), "refused");
    }

    #[test]
    fn purging_removes_the_item_and_its_cell() {
        let f = fixture();
        let path = f.file("Downloads/Chrome.dmg", "bytes");
        let record = f
            .quarantine
            .hold(&f.candidate(&path), &f.ctx(), 1000)
            .unwrap();
        let cell = record.stored_path.parent().unwrap().to_path_buf();

        f.quarantine.purge(&record.id, 2000).unwrap();
        assert!(!cell.exists());
        assert_eq!(
            f.store.quarantine_record(&record.id).unwrap().status,
            QuarantineStatus::Removed
        );
    }

    #[test]
    fn a_record_pointing_outside_the_drawer_is_refused() {
        // Defence against a corrupted or tampered-with database.
        let f = fixture();
        let elsewhere = f.file("Downloads/precious.txt", "do not delete me");
        f.store
            .insert_quarantine(&QuarantineRecord {
                id: "forged".into(),
                finding_id: None,
                original_path: "/tmp/whatever".into(),
                stored_path: elsewhere.clone(),
                display_name: "precious.txt".into(),
                category: Category::Oddments,
                size: 1,
                content_hash: None,
                evidence: vec![],
                quarantined_unix: 0,
                expires_unix: 0,
                status: QuarantineStatus::Held,
                resolved_unix: None,
                mode: crate::storage::RecordMode::Whole,
                item_count: 1,
                attention: false,
            })
            .unwrap();

        assert_eq!(
            f.quarantine.purge("forged", 1).unwrap_err().code(),
            "refused"
        );
        assert_eq!(
            f.quarantine.restore("forged", 1).unwrap_err().code(),
            "refused"
        );
        assert!(
            elsewhere.exists(),
            "a forged record must not reach real files"
        );
    }

    #[test]
    fn a_whole_directory_can_be_held_and_restored() {
        let f = fixture();
        let dir = f.home.join("Library/Application Support/com.dead.app");
        fs::create_dir_all(dir.join("Cache")).unwrap();
        fs::write(dir.join("Cache/a.bin"), "aaa").unwrap();
        fs::write(dir.join("Cache/b.bin"), "bbb").unwrap();

        let mut candidate = f.candidate(&dir);
        candidate.category = Category::Ghosts;
        let record = f.quarantine.hold(&candidate, &f.ctx(), 1000).unwrap();
        assert!(!dir.exists());

        f.quarantine.restore(&record.id, 2000).unwrap();
        assert_eq!(fs::read_to_string(dir.join("Cache/a.bin")).unwrap(), "aaa");
        assert_eq!(fs::read_to_string(dir.join("Cache/b.bin")).unwrap(), "bbb");
    }

    #[test]
    fn the_sweep_removes_only_what_has_actually_expired() {
        let f = fixture();
        let old = f.file("Downloads/old.dmg", "old");
        let new = f.file("Downloads/new.dmg", "new");

        let mut first = f.candidate(&old);
        first.id = "f1".into();
        let held_old = f.quarantine.hold(&first, &f.ctx(), 0).unwrap();

        let mut second = f.candidate(&new);
        second.id = "f2".into();
        let held_new = f.quarantine.hold(&second, &f.ctx(), 1_000_000).unwrap();

        // Just past the first item's window, nowhere near the second's.
        let removed = f.quarantine.sweep_expired(15 * 86_400).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(
            f.store.quarantine_record(&held_old.id).unwrap().status,
            QuarantineStatus::Removed
        );
        assert_eq!(
            f.store.quarantine_record(&held_new.id).unwrap().status,
            QuarantineStatus::Held
        );
    }

    #[test]
    fn the_drawer_records_what_it_actually_holds_not_what_the_finding_was_worth() {
        // A group finding's `size` covers every redundant member of the group,
        // but holding one moves a single file. Recording the finding's size
        // made the drawer overstate itself, and "empty the drawer to free X"
        // is a promise that has to be keepable.
        let f = fixture();
        let path = f.file("Pictures/shot-1.png", "twelve bytes");
        let mut candidate = f.candidate(&path);
        candidate.category = Category::Screenshots;
        candidate.group = vec![
            GroupMember {
                path: path.clone(),
                size: 12,
                modified_unix: None,
                suggested_keep: false,
            },
            GroupMember {
                path: f.home.join("Pictures/shot-2.png"),
                size: 400,
                modified_unix: None,
                suggested_keep: true,
            },
        ];
        // What the whole burst is worth, far more than the one file moving.
        candidate.size = 412;

        let record = f.quarantine.hold(&candidate, &f.ctx(), 1000).unwrap();

        assert_eq!(
            record.size,
            "twelve bytes".len() as u64,
            "the drawer holds one file and must say so"
        );
        assert_eq!(
            f.quarantine.held_bytes().unwrap(),
            "twelve bytes".len() as u64
        );
    }

    #[test]
    fn a_directory_still_reports_its_measured_size() {
        // The other side of the same rule: `observe` does not walk a
        // directory, so its observed size is zero and the measured size from
        // the scan is the only real figure there is.
        let f = fixture();
        f.file("Library/Caches/thing/a.bin", "aaaa");
        let dir = f.home.join("Library/Caches/thing");
        let mut candidate = f.candidate(&dir);
        candidate.size = 4096;

        let record = f.quarantine.hold(&candidate, &f.ctx(), 1000).unwrap();
        assert_eq!(record.size, 4096, "a directory keeps its measured size");
    }

    #[test]
    fn held_bytes_adds_up_what_is_in_the_drawer() {
        let f = fixture();
        let a = f.file("Downloads/a.dmg", "12345");
        let mut candidate = f.candidate(&a);
        candidate.size = 5;
        f.quarantine.hold(&candidate, &f.ctx(), 0).unwrap();
        assert_eq!(f.quarantine.held_bytes().unwrap(), 5);
    }

    #[test]
    fn a_file_keeps_its_hash_so_a_restore_can_be_verified() {
        let f = fixture();
        let path = f.file("Downloads/Chrome.dmg", "installer bytes");
        let record = f
            .quarantine
            .hold(&f.candidate(&path), &f.ctx(), 1000)
            .unwrap();
        assert!(record.content_hash.is_some());

        f.quarantine.restore(&record.id, 2000).unwrap();
        assert_eq!(hash_file(&path).ok(), record.content_hash);
    }

    #[test]
    fn restore_names_are_proposed_in_order_and_the_first_is_the_original() {
        let original = Path::new("/somewhere/report.pdf");
        assert_eq!(restored_name(original, 0), original);
        assert!(restored_name(original, 1)
            .to_string_lossy()
            .ends_with("report (restored).pdf"));
        assert!(restored_name(original, 2)
            .to_string_lossy()
            .ends_with("report (restored 2).pdf"));
    }

    #[test]
    fn restoring_skips_every_taken_name_and_touches_none_of_them() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("report.pdf");
        fs::write(&original, "a").unwrap();
        fs::write(tmp.path().join("report (restored).pdf"), "b").unwrap();
        let stored = tmp.path().join("held.pdf");
        fs::write(&stored, "held").unwrap();

        let (destination, renamed) = restore_into(&stored, &original).unwrap();
        assert!(renamed);
        assert!(destination
            .to_string_lossy()
            .ends_with("report (restored 2).pdf"));
        assert_eq!(fs::read_to_string(&original).unwrap(), "a");
        assert_eq!(
            fs::read_to_string(tmp.path().join("report (restored).pdf")).unwrap(),
            "b"
        );
        assert_eq!(fs::read_to_string(&destination).unwrap(), "held");
    }

    #[test]
    #[cfg(unix)]
    fn a_move_that_cannot_finish_leaves_nothing_behind() {
        // The copy-then-remove fallback is the half of `move_across` that can
        // stop in the middle: on Windows a file another process holds open
        // copies perfectly well and then refuses to be deleted. A read-only
        // parent directory reproduces that shape here — the source can be
        // read but neither renamed nor removed.
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let locked = tmp.path().join("locked");
        fs::create_dir_all(&locked).unwrap();
        let source = locked.join("held.bin");
        fs::write(&source, "the only copy").unwrap();
        let destination = tmp.path().join("moved.bin");

        let original = fs::metadata(&locked).unwrap().permissions();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

        let outcome = transfer::move_entry(&source, &destination, &Ctl::none());

        fs::set_permissions(&locked, original).unwrap();

        assert!(outcome.is_err(), "a move that cannot complete must say so");
        assert!(
            !destination.exists(),
            "the half-written copy must be cleaned up, not left as a duplicate"
        );
        assert_eq!(
            fs::read_to_string(&source).unwrap(),
            "the only copy",
            "the source must survive a failed move intact"
        );
    }

    #[test]
    fn a_record_pointing_at_the_drawer_root_does_not_take_the_drawer_with_it() {
        // Every real record stores its item one cell deep. A record that does
        // not — a corrupted row, a hand-edited database — must not turn a
        // single restore into an emptied drawer.
        let f = fixture();
        let root = f.quarantine.root().to_path_buf();
        fs::create_dir_all(&root).unwrap();
        let stray = root.join("stray.txt");
        fs::write(&stray, "one restore").unwrap();
        let neighbour_cell = root.join("11111111-1111-1111-1111-111111111111");
        fs::create_dir_all(&neighbour_cell).unwrap();
        fs::write(neighbour_cell.join("someone-elses.bin"), "still held").unwrap();

        f.store
            .insert_quarantine(&QuarantineRecord {
                id: "shallow".into(),
                finding_id: None,
                original_path: f.home.join("Downloads/stray.txt"),
                stored_path: stray,
                display_name: "stray.txt".into(),
                category: Category::Oddments,
                size: 11,
                content_hash: None,
                evidence: vec![],
                quarantined_unix: 0,
                expires_unix: 0,
                status: QuarantineStatus::Held,
                resolved_unix: None,
                mode: crate::storage::RecordMode::Whole,
                item_count: 1,
                attention: false,
            })
            .unwrap();

        f.quarantine.restore("shallow", 2000).unwrap();

        assert!(root.is_dir(), "the drawer itself must still be there");
        assert_eq!(
            fs::read_to_string(neighbour_cell.join("someone-elses.bin")).unwrap(),
            "still held",
            "restoring one item must not disturb another"
        );
    }
}
