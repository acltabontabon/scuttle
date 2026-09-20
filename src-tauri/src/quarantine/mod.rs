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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::ScuttleError;
use crate::model::{CleanupCandidate, TargetKind};
use crate::safety::{self, ActionContext};
use crate::storage::{HistoryEntry, QuarantineRecord, QuarantineStatus, Store};
use crate::Result;

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
    /// True when the original location was occupied and a new name was used.
    pub renamed: bool,
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
                hash_file(&target.path)
            } else {
                None
            };

        // The move itself. Anything that fails here leaves the original in
        // place, which is the right way round to fail.
        if let Err(err) = move_across(&target.path, &stored_path) {
            let _ = std::fs::remove_dir_all(&cell);
            return Err(err);
        }

        let size = if candidate.size > 0 {
            candidate.size
        } else {
            target.observed.size
        };
        let record = QuarantineRecord {
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
            status: QuarantineStatus::Held,
            resolved_unix: None,
        };

        write_manifest(&cell, &record);

        // If the database write fails the item is already moved, so put it
        // back rather than leaving it stranded with no record.
        if let Err(err) = self.store.insert_quarantine(&record) {
            let _ = move_across(&stored_path, &target.path);
            let _ = std::fs::remove_dir_all(&cell);
            return Err(err);
        }

        let _ = self.store.forget_candidate(&candidate.id);
        self.log(now_unix, "quarantine", &record, "held");
        Ok(record)
    }

    /// Put something back. Never overwrites.
    pub fn restore(&self, id: &str, now_unix: i64) -> Result<RestoreOutcome> {
        let record = self.store.quarantine_record(id)?;
        if record.status != QuarantineStatus::Held {
            return Err(ScuttleError::Refused(
                "That item has already left the drawer.".into(),
            ));
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

        if let Some(parent) = record.original_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let (destination, renamed) = free_destination(&record.original_path);
        move_across(&record.stored_path, &destination)?;
        let _ = std::fs::remove_dir_all(record.stored_path.parent().unwrap_or(&self.root));

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
        })
    }

    /// Remove something permanently. The only operation in Scuttle that
    /// destroys data, and it is never reached without an explicit request.
    pub fn purge(&self, id: &str, now_unix: i64) -> Result<()> {
        let record = self.store.quarantine_record(id)?;
        if record.status == QuarantineStatus::Removed {
            return Ok(());
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

/// Find a destination that does not already exist.
///
/// `report.pdf` becomes `report (restored).pdf`, then `report (restored 2).pdf`.
/// The original is never touched.
fn free_destination(original: &Path) -> (PathBuf, bool) {
    if !original.exists() {
        return (original.to_path_buf(), false);
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

    for attempt in 1..1000 {
        let suffix = if attempt == 1 {
            " (restored)".to_string()
        } else {
            format!(" (restored {attempt})")
        };
        let candidate = parent.join(format!("{stem}{suffix}{extension}"));
        if !candidate.exists() {
            return (candidate, true);
        }
    }
    // Pathological, but never overwrite.
    (
        parent.join(format!(
            "{stem} (restored {}){extension}",
            uuid::Uuid::new_v4()
        )),
        true,
    )
}

/// Move a file or directory, falling back to copy-then-remove when the source
/// and destination are on different volumes.
fn move_across(from: &Path, to: &Path) -> Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => return Ok(()),
        Err(err) => {
            // A rename can fail for reasons a copy would also fail for, but
            // the common one — crossing a volume boundary — is recoverable.
            tracing::debug!(error = %err.kind(), "rename failed, copying instead");
        }
    }

    let meta = std::fs::symlink_metadata(from)?;
    if meta.is_dir() {
        copy_tree(from, to)?;
        std::fs::remove_dir_all(from)?;
    } else {
        std::fs::copy(from, to)?;
        std::fs::remove_file(from)?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let destination = to.join(entry.file_name());
        if file_type.is_symlink() {
            // Links are not followed. Copying the target would silently
            // duplicate data that lives somewhere else entirely.
            continue;
        }
        if file_type.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else {
            std::fs::copy(entry.path(), &destination)?;
        }
    }
    Ok(())
}

fn hash_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher).ok()?;
    Some(hasher.finalize().to_hex().to_string())
}

fn write_manifest(cell: &Path, record: &QuarantineRecord) {
    let manifest = Manifest {
        id: record.id.clone(),
        original_path: record.original_path.to_string_lossy().into_owned(),
        display_name: record.display_name.clone(),
        quarantined_unix: record.quarantined_unix,
        expires_unix: record.expires_unix,
        size: record.size,
        reasons: record.evidence.iter().map(|e| e.summary.clone()).collect(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&manifest) {
        let _ = std::fs::write(cell.join("scuttle-manifest.json"), json);
    }
}

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
        assert_eq!(hash_file(&path), record.content_hash);
    }

    #[test]
    fn free_destination_keeps_trying_until_it_finds_a_gap() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("report.pdf");
        fs::write(&original, "a").unwrap();
        fs::write(tmp.path().join("report (restored).pdf"), "b").unwrap();

        let (destination, renamed) = free_destination(&original);
        assert!(renamed);
        assert!(destination
            .to_string_lossy()
            .ends_with("report (restored 2).pdf"));
    }

    #[test]
    fn copying_a_tree_does_not_follow_links_out_of_it() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "not yours").unwrap();

        let source = tmp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("real.txt"), "mine").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, source.join("link")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&outside, source.join("link")).unwrap();

        let destination = tmp.path().join("destination");
        copy_tree(&source, &destination).unwrap();

        assert!(destination.join("real.txt").exists());
        assert!(
            !destination.join("link").exists(),
            "links must not be traversed"
        );
        assert!(outside.join("secret.txt").exists());
    }
}
