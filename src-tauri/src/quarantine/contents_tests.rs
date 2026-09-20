//! Regression tests for cleaning a shared folder file by file, restoring it,
//! and settling interrupted work. Every fixture is a private temp directory
//! registered as the scan root; nothing here touches a real cache.

use std::fs;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::contents::Reconciled;
use super::fsx;
use super::transfer::testing::{eacces, exdev, in_use, Script};
use super::transfer::{Ctl, FailureKind, Faults, Step};
use super::*;
use crate::evidence::{ev, EvidenceKind};
use crate::model::*;
use crate::safety::{Bidding, ProtectedPaths};
use crate::scanning::snapshot::{self, SnapshotPolicy};
use crate::scanning::ScanOptions;
use crate::storage::{journal, SnapshotEntry, SnapshotState};

struct World {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    root: PathBuf,
    roots: Vec<PathBuf>,
    protected: ProtectedPaths,
    store: Arc<Store>,
    q: Quarantine,
    candidate: CleanupCandidate,
}

const NOW: i64 = 1_000;

fn world(files: &[(&str, &str)]) -> World {
    world_on(None, files)
}

/// `drawer`: where the Drawer lives, when it should be somewhere other than
/// beside the home folder — for instance on a second volume.
fn world_on(drawer: Option<PathBuf>, files: &[(&str, &str)]) -> World {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home/rummager");
    let root = home.join("Library/Caches/owner");
    fs::create_dir_all(&root).unwrap();
    for (rel, body) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let protected = ProtectedPaths::for_home(&home);
    let store = Arc::new(Store::in_memory().unwrap());
    let drawer = drawer.unwrap_or_else(|| home.join(".scuttle/quarantine"));
    let q = Quarantine::new(drawer, Arc::clone(&store), 14);

    let fingerprint = safety::observe(&root).unwrap();
    let candidate = CleanupCandidate {
        id: "f1".into(),
        detector: "caches".into(),
        category: Category::Caches,
        target_kind: TargetKind::Directory,
        path: root.clone(),
        display_name: "Owner cache".into(),
        associated_app: Some("Owner".into()),
        size: 0,
        group_bytes: 0,
        confidence: Confidence::High,
        risk: Risk::Low,
        recommended_action: RecommendedAction::Quarantine,
        evidence: vec![ev(EvidenceKind::KnownCachePath {
            owner: "Owner".into(),
        })],
        remark: None,
        modified_unix: fingerprint.modified_unix,
        accessed_unix: None,
        created_unix: None,
        group: vec![],
        fingerprint,
    };
    store.begin_scan("s1", &ScanOptions::default(), 0).unwrap();
    store
        .save_candidates("s1", std::slice::from_ref(&candidate))
        .unwrap();

    let w = World {
        roots: vec![home.clone()],
        _tmp: tmp,
        home,
        root,
        protected,
        store,
        q,
        candidate,
    };
    w.review();
    w
}

impl World {
    fn ctx(&self) -> ActionContext<'_> {
        ActionContext {
            protected: &self.protected,
            allowed_roots: &self.roots,
            bidding: Bidding::Scuttle,
        }
    }

    /// Record the reviewed set, as a scan would.
    fn review(&self) -> snapshot::Recorded {
        let never = || false;
        snapshot::record(
            &self.store,
            "f1",
            &self.root,
            &SnapshotPolicy {
                protected: &self.protected,
                settle_secs: 0,
                now_unix: NOW,
                cancelled: &never,
                limit: 1_000_000,
            },
        )
        .unwrap()
    }

    fn run(&self, retry: bool) -> Result<contents::ContentsOutcome> {
        self.run_with(retry, &Ctl::none(), &|_| {})
    }

    fn run_with(
        &self,
        retry: bool,
        ctl: &Ctl<'_>,
        on_tally: &dyn Fn(&transfer::Tally),
    ) -> Result<contents::ContentsOutcome> {
        self.q.hold_contents(
            &self.candidate,
            &self.ctx(),
            NOW,
            retry,
            ctl,
            &contents::observe(on_tally),
        )
    }

    fn write(&self, rel: &str, body: &str) -> PathBuf {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
        path
    }

    fn read_drawer(&self, record: &QuarantineRecord, rel: &str) -> Option<String> {
        fs::read_to_string(join(&record.stored_path, rel)).ok()
    }
}

fn join(base: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(base.to_path_buf(), |p, c| p.join(c))
}

fn ten() -> Vec<(String, String)> {
    (0..10)
        .map(|n| (format!("f{n:02}.bin"), format!("body {n}")))
        .collect()
}

fn world_of(files: &[(String, String)]) -> World {
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    world(&borrowed)
}

// ---------------------------------------------------------------------------
// The reported bug: a live folder can never pass a whole-folder freshness gate
// ---------------------------------------------------------------------------

#[test]
fn a_folder_that_changed_after_the_scan_is_cleaned_file_by_file_not_refused() {
    let w = world(&[
        ("a.bin", "aaaa"),
        ("b.bin", "bbbb"),
        ("c.bin", "cccc"),
        ("d.bin", "dddd"),
    ]);

    // The world moves: one file is rewritten, one is deleted, a new one
    // appears. The folder's own timestamp and child count change with it.
    fs::write(w.root.join("b.bin"), "bbbb, rewritten and longer").unwrap();
    fs::remove_file(w.root.join("d.bin")).unwrap();
    w.write("e.bin", "brand new");
    w.write("f.bin", "also new");

    // The old whole-folder gate would have refused all of it, for ever.
    assert_eq!(
        safety::authorize(&w.candidate, &w.ctx())
            .unwrap_err()
            .code(),
        "stale",
        "this is the refusal from the bug report"
    );

    let outcome = w.run(false).unwrap();

    assert_eq!(outcome.tally.moved, 2, "the unchanged files go");
    assert_eq!(
        outcome.tally.skipped, 2,
        "the changed and the vanished are skipped"
    );
    assert_eq!(outcome.tally.failed, 0, "nothing here is a system failure");
    let kinds: Vec<FailureKind> = outcome.issues.groups().iter().map(|g| g.kind).collect();
    assert!(kinds.contains(&FailureKind::Stale) && kinds.contains(&FailureKind::Missing));

    let record = outcome.record.expect("something was moved");
    assert_eq!(record.status, QuarantineStatus::Held);
    assert_eq!(record.mode, crate::storage::RecordMode::Contents);
    assert_eq!(record.item_count, 2);
    assert_eq!(record.size, 8, "the drawer holds exactly what moved");
    assert_eq!(w.read_drawer(&record, "a.bin").as_deref(), Some("aaaa"));
    assert_eq!(w.read_drawer(&record, "c.bin").as_deref(), Some("cccc"));

    assert!(w.root.is_dir(), "the shared folder itself never moves");
    assert!(!w.root.join("a.bin").exists() && !w.root.join("c.bin").exists());
    assert_eq!(
        fs::read_to_string(w.root.join("b.bin")).unwrap(),
        "bbbb, rewritten and longer",
        "a changed file is left exactly as it is"
    );
    assert_eq!(
        fs::read_to_string(w.root.join("e.bin")).unwrap(),
        "brand new",
        "a file that was not in the reviewed set is never touched"
    );
}

#[test]
fn what_remains_asks_to_be_reviewed_again_and_never_guesses_its_size() {
    let w = world(&[("a.bin", "aaaa"), ("b.bin", "bbbb")]);
    fs::write(w.root.join("b.bin"), "changed!").unwrap();
    w.run(false).unwrap();

    let info = w.store.snapshot_info("f1").unwrap();
    assert_eq!(info.state, SnapshotState::NeedsRefresh);
    let err = w.run(false).unwrap_err();
    assert_eq!(err.code(), "stale");
    assert!(err.to_string().contains("Review it again"), "{err}");
}

#[test]
fn a_file_that_appears_with_old_timestamps_is_not_in_the_reviewed_set() {
    let w = world(&[("a.bin", "aaaa")]);

    // Created after the review, but made to look ancient.
    let late = w.write("late.bin", "sneaked in");
    let ancient = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    fs::File::options()
        .write(true)
        .open(&late)
        .unwrap()
        .set_modified(ancient)
        .unwrap();

    let outcome = w.run(false).unwrap();
    assert_eq!(outcome.tally.moved, 1);
    assert_eq!(
        fs::read_to_string(&late).unwrap(),
        "sneaked in",
        "timestamps alone do not admit a file to the set"
    );
}

#[test]
fn a_file_swapped_for_a_lookalike_is_skipped_as_replaced() {
    let w = world(&[("a.bin", "original"), ("b.bin", "other")]);
    let target = w.root.join("a.bin");
    let mtime = fs::metadata(&target).unwrap().modified().unwrap();

    // Same name, same size, same modification time — a different object,
    // put in place while the original still existed.
    let staging = w.root.join("..").join("staging.bin");
    fs::write(&staging, "imposter").unwrap();
    fs::File::options()
        .write(true)
        .open(&staging)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    fs::rename(&staging, &target).unwrap();

    let outcome = w.run(false).unwrap();
    assert_eq!(outcome.tally.moved, 1, "only the untouched file goes");
    assert!(outcome
        .issues
        .groups()
        .iter()
        .any(|g| g.kind == FailureKind::Replaced));
    assert_eq!(fs::read_to_string(&target).unwrap(), "imposter");
}

#[test]
fn a_reviewed_set_without_creation_times_still_moves_its_files() {
    // Plenty of filesystems have no birth time. Its absence must not read as a
    // change.
    let w = world(&[("a.bin", "aaaa"), ("b.bin", "bbbb")]);
    w.store.begin_snapshot("f1").unwrap();
    let entries: Vec<SnapshotEntry> = ["a.bin", "b.bin"]
        .iter()
        .map(|rel| {
            let id = fsx::identity_of(&w.root.join(rel)).unwrap();
            SnapshotEntry {
                rel: rel.to_string(),
                size: id.size,
                mtime_ns: id.mtime_ns,
                created_ns: None,
                file_id: id.file_id,
            }
        })
        .collect();
    w.store.append_snapshot_entries("f1", &entries).unwrap();
    w.store
        .finish_snapshot("f1", SnapshotState::Complete, NOW, 2, 8, true)
        .unwrap();

    let outcome = w.run(false).unwrap();
    assert_eq!(outcome.tally.moved, 2);
    assert!(outcome.issues.is_empty(), "{:?}", outcome.issues);
}

#[test]
fn a_finding_from_before_reviewed_sets_asks_for_a_review_instead_of_guessing() {
    let w = world(&[("a.bin", "aaaa")]);
    w.store.begin_snapshot("f1").unwrap(); // as an older database would look
    let err = w.run(false).unwrap_err();
    assert_eq!(err.code(), "stale");
    assert!(w.root.join("a.bin").exists());
    assert!(w.store.held_quarantine().unwrap().is_empty());
}

#[test]
fn a_folder_too_big_to_review_is_refused_not_moved_by_a_rule_nobody_read() {
    let w = world(&[("a.bin", "a"), ("b.bin", "b"), ("c.bin", "c")]);
    let never = || false;
    let recorded = snapshot::record(
        &w.store,
        "f1",
        &w.root,
        &SnapshotPolicy {
            protected: &w.protected,
            settle_secs: 0,
            now_unix: NOW,
            cancelled: &never,
            limit: 2,
        },
    )
    .unwrap();
    assert_eq!(recorded.state, SnapshotState::Truncated);

    let err = w.run(false).unwrap_err();
    assert_eq!(err.code(), "refused");
    assert!(w.root.join("a.bin").exists());
}

// ---------------------------------------------------------------------------
// System refusals are reported as themselves
// ---------------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn a_file_the_system_will_not_let_go_is_reported_with_its_code_and_the_rest_move() {
    use std::os::unix::fs::PermissionsExt;
    let w = world(&[
        ("free-1.bin", "one"),
        ("held/locked.bin", "stuck"),
        ("free-2.bin", "two"),
    ]);
    let held = w.root.join("held");
    fs::set_permissions(&held, fs::Permissions::from_mode(0o555)).unwrap();

    let outcome = w.run(false).unwrap();
    fs::set_permissions(&held, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(outcome.tally.moved, 2);
    assert_eq!(outcome.tally.failed, 1, "the system refused one");
    assert_eq!(outcome.tally.skipped, 0);
    let group = &outcome.issues.groups()[0];
    assert_eq!(group.kind, FailureKind::AccessDenied);
    assert_eq!(group.phase, transfer::Phase::Publish);
    assert!(group.os_code.is_some(), "the OS code is kept");
    assert_eq!(group.samples, vec!["locked.bin".to_string()]);
    assert_ne!(
        group.next_step,
        transfer::NextStep::CloseApp,
        "a permission refusal is not evidence a program has it open"
    );
    assert_eq!(
        fs::read_to_string(held.join("locked.bin")).unwrap(),
        "stuck"
    );
}

#[test]
#[cfg(windows)]
fn a_file_another_program_holds_open_is_skipped_as_in_use_and_the_rest_move() {
    use std::os::windows::fs::OpenOptionsExt;
    let w = world(&[("free.bin", "free"), ("busy.bin", "busy")]);
    // Share mode 0: nothing else may open, rename or delete it.
    let _held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(w.root.join("busy.bin"))
        .unwrap();

    let outcome = w.run(false).unwrap();

    assert_eq!(outcome.tally.moved, 1);
    assert_eq!(outcome.tally.failed, 1);
    let group = &outcome.issues.groups()[0];
    assert_eq!(group.kind, FailureKind::InUse);
    assert_eq!(group.os_code, Some(32), "ERROR_SHARING_VIOLATION");
    assert_eq!(group.next_step, transfer::NextStep::CloseApp);
    assert!(w.root.join("busy.bin").exists());
    assert!(w.root.is_dir());
}

#[test]
#[cfg(unix)]
fn a_retry_only_touches_what_failed_and_never_widens_the_set() {
    use std::os::unix::fs::PermissionsExt;
    let w = world(&[("ok.bin", "ok"), ("held/locked.bin", "stuck")]);
    let held = w.root.join("held");
    fs::set_permissions(&held, fs::Permissions::from_mode(0o555)).unwrap();
    let first = w.run(false).unwrap();
    assert_eq!(first.tally.moved, 1);

    // The obstacle goes away — and a new file arrives in the meantime.
    fs::set_permissions(&held, fs::Permissions::from_mode(0o755)).unwrap();
    w.write("newcomer.bin", "not reviewed");

    assert_eq!(
        w.run(false).unwrap_err().code(),
        "stale",
        "a plain run will not proceed over a set that needs review"
    );
    let retry = w.run(true).unwrap();

    assert_eq!(
        retry.tally.moved, 1,
        "only the file that failed is tried again"
    );
    assert!(!held.join("locked.bin").exists());
    assert_eq!(
        fs::read_to_string(w.root.join("newcomer.bin")).unwrap(),
        "not reviewed"
    );
    assert!(
        w.store.candidate("f1").is_err(),
        "once nothing reviewed is left, the finding is done"
    );
}

// ---------------------------------------------------------------------------
// Volume boundaries and running out of room
// ---------------------------------------------------------------------------

#[test]
fn across_volumes_every_file_is_copied_verified_and_then_removed() {
    let files = ten();
    let w = world_of(&files);
    let script = Script::new().cross_device();
    let ctl = Ctl {
        faults: Some(&script),
        ..Ctl::default()
    };

    let outcome = w.run_with(false, &ctl, &|_| {}).unwrap();

    assert_eq!(outcome.tally.moved, 10);
    assert!(outcome.tally.copied_bytes > 0, "a copy moves real bytes");
    assert_eq!(outcome.tally.copied_bytes, outcome.tally.moved_bytes);
    let record = outcome.record.unwrap();
    for (rel, body) in &files {
        assert_eq!(w.read_drawer(&record, rel).as_deref(), Some(body.as_str()));
        assert!(!w.root.join(rel).exists());
    }
}

#[test]
fn a_rename_is_reported_as_files_in_the_drawer_not_as_bytes_copied() {
    let w = world_of(&ten());
    let outcome = w.run(false).unwrap();
    assert_eq!(outcome.tally.copied_bytes, 0, "a rename copies nothing");
    assert!(
        outcome.tally.moved_bytes > 0,
        "but the files are in the drawer"
    );
}

#[test]
fn a_copy_that_will_not_fit_is_refused_before_anything_is_written() {
    let w = world_of(&ten());
    fsx_forced_volume(true, Some(3));

    let err = w.run(false).unwrap_err();
    fsx_forced_volume(false, None);

    assert_eq!(err.code(), "transfer");
    match err {
        crate::ScuttleError::Transfer(f) => assert_eq!(f.kind, FailureKind::NoSpace),
        other => panic!("{other:?}"),
    }
    assert!(w.root.join("f00.bin").exists(), "nothing moved");
    assert!(
        fs::read_dir(w.q.root()).map(|r| r.count()).unwrap_or(0) == 0,
        "no cell was created for a move that could not happen"
    );
}

fn fsx_forced_volume(different: bool, free: Option<u64>) {
    super::volume::testing::DIFFERENT.with(|d| d.set(different));
    super::volume::testing::FREE.with(|f| f.set(free));
}

#[test]
fn the_disk_filling_up_mid_run_stops_copying_and_loses_nothing() {
    let files = ten();
    let w = world_of(&files);
    // Copies fail with "disk full" from the fourth write on.
    struct FillsUp(std::cell::Cell<u32>);
    impl Faults for FillsUp {
        fn at(&self, step: Step) -> Option<io::Error> {
            match step {
                Step::Rename => Some(io::Error::from_raw_os_error(exdev())),
                Step::CopyWrite => {
                    self.0.set(self.0.get() + 1);
                    (self.0.get() > 3).then(|| {
                        io::Error::from_raw_os_error(super::transfer::testing::disk_full())
                    })
                }
                _ => None,
            }
        }
    }
    let faults = FillsUp(std::cell::Cell::new(0));
    let ctl = Ctl {
        faults: Some(&faults),
        ..Ctl::default()
    };

    let outcome = w.run_with(false, &ctl, &|_| {}).unwrap();

    assert_eq!(outcome.tally.moved, 3);
    assert_eq!(outcome.tally.failed, 7);
    assert!(outcome
        .issues
        .groups()
        .iter()
        .any(|g| g.kind == FailureKind::NoSpace));
    let record = outcome.record.unwrap();
    for (rel, body) in &files {
        let in_source = fs::read_to_string(w.root.join(rel)).ok().as_deref() == Some(body.as_str());
        let in_drawer = w.read_drawer(&record, rel).as_deref() == Some(body.as_str());
        assert!(in_source || in_drawer, "{rel}: content is nowhere");
    }
}

// ---------------------------------------------------------------------------
// Many files, one large file, cancellation
// ---------------------------------------------------------------------------

#[test]
fn thousands_of_small_files_and_a_large_one_are_moved_and_counted_exactly() {
    let w = world(&[]);
    let mut expected = 0u64;
    for n in 0..1_200 {
        let rel = format!("d{}/f{n}.bin", n % 7);
        let body = format!("small {n}");
        expected += body.len() as u64;
        w.write(&rel, &body);
    }
    let big = w.root.join("big.bin");
    fs::File::create(&big)
        .unwrap()
        .set_len(64 * 1024 * 1024)
        .unwrap();
    expected += 64 * 1024 * 1024;
    w.review();

    let calls = std::cell::Cell::new(0u32);
    let outcome = w
        .run_with(false, &Ctl::none(), &|_| calls.set(calls.get() + 1))
        .unwrap();

    assert_eq!(outcome.planned_files, 1_201);
    assert_eq!(outcome.tally.moved, 1_201);
    assert_eq!(outcome.tally.moved_bytes, expected);
    assert_eq!(calls.get(), 1_201, "progress is reported per file");
    let record = outcome.record.unwrap();
    assert_eq!(record.size, expected);
    assert_eq!(record.item_count, 1_201);
    assert!(
        w.store.candidate("f1").is_err(),
        "everything reviewed moved: finding done"
    );
    assert_eq!(
        fs::metadata(join(&record.stored_path, "big.bin"))
            .unwrap()
            .len(),
        64 * 1024 * 1024
    );
}

#[test]
fn cancelling_stops_at_a_file_boundary_and_what_moved_is_recoverable() {
    let files = ten();
    let w = world_of(&files);
    let flag = AtomicBool::new(false);
    let ctl = Ctl {
        cancel: Some(&flag),
        ..Ctl::default()
    };

    let outcome = w
        .run_with(false, &ctl, &|t| {
            if t.processed == 4 {
                flag.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();

    assert!(outcome.cancelled);
    assert_eq!(outcome.tally.moved, 4);
    assert_eq!(outcome.unmoved, 6);
    let record = outcome.record.expect("what moved is held");
    assert_eq!(record.item_count, 4);

    // Stopping early is not something having gone wrong: the rest of the set is
    // untouched and still exactly what was reviewed.
    let info = w.store.snapshot_info("f1").unwrap();
    assert_eq!(info.state, SnapshotState::Complete);
    let (left, _) = w.store.snapshot_actionable("f1").unwrap();
    assert_eq!(left, 6);

    let resumed = w.run(false).unwrap();
    assert_eq!(resumed.tally.moved, 6);
    for (rel, body) in &files {
        let a = w.read_drawer(&record, rel);
        let b = resumed.record.as_ref().and_then(|r| w.read_drawer(r, rel));
        assert_eq!(a.or(b).as_deref(), Some(body.as_str()), "{rel}");
    }
}

#[test]
fn cancelling_in_the_middle_of_a_copy_leaves_no_partial_file() {
    let w = world(&[("big.bin", ""), ("small.bin", "s")]);
    fs::write(w.root.join("big.bin"), vec![9u8; 3 * 1024 * 1024]).unwrap();
    w.review();
    let flag = AtomicBool::new(false);
    let report = |_: u64| flag.store(true, Ordering::Relaxed);
    let script = Script::new().cross_device();
    let ctl = Ctl {
        cancel: Some(&flag),
        faults: Some(&script),
        on_bytes: Some(&report),
    };

    let outcome = w.run_with(false, &ctl, &|_| {}).unwrap();

    assert!(outcome.cancelled);
    assert!(
        w.root.join("big.bin").exists(),
        "the source of a stopped copy is untouched"
    );
    let leftovers = walk(w.q.root())
        .into_iter()
        .filter(|p| p.to_string_lossy().ends_with(transfer::PART_SUFFIX))
        .count();
    assert_eq!(leftovers, 0);
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(read) = fs::read_dir(dir) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Restore takes back exactly what was recorded
// ---------------------------------------------------------------------------

#[test]
fn restoring_returns_exactly_the_recorded_files_and_touches_nothing_else() {
    let w = world(&[("a.bin", "aaaa"), ("sub/b.bin", "bbbb"), ("c.bin", "cccc")]);
    fs::write(w.root.join("c.bin"), "cccc, but changed").unwrap();
    let record = w.run(false).unwrap().record.unwrap();
    w.write("newcomer.bin", "arrived later");

    let outcome = w.q.restore(&record.id, 2_000).unwrap();

    assert_eq!(outcome.restored, 2);
    assert_eq!(outcome.remaining, 0);
    assert_eq!(fs::read_to_string(w.root.join("a.bin")).unwrap(), "aaaa");
    assert_eq!(
        fs::read_to_string(w.root.join("sub/b.bin")).unwrap(),
        "bbbb"
    );
    assert_eq!(
        fs::read_to_string(w.root.join("c.bin")).unwrap(),
        "cccc, but changed"
    );
    assert_eq!(
        fs::read_to_string(w.root.join("newcomer.bin")).unwrap(),
        "arrived later"
    );
    assert!(w.store.held_quarantine().unwrap().is_empty());
    assert!(
        !record.stored_path.parent().unwrap().exists(),
        "the emptied cell is dropped"
    );
}

#[test]
fn restoring_never_overwrites_a_file_that_arrived_in_the_meantime() {
    let w = world(&[("a.bin", "the original")]);
    let record = w.run(false).unwrap().record.unwrap();
    w.write("a.bin", "something new and important");

    let outcome = w.q.restore(&record.id, 2_000).unwrap();

    assert!(outcome.renamed);
    assert_eq!(
        fs::read_to_string(w.root.join("a.bin")).unwrap(),
        "something new and important"
    );
    assert_eq!(
        fs::read_to_string(w.root.join("a (restored).bin")).unwrap(),
        "the original"
    );
}

#[test]
fn restoring_recreates_a_folder_that_has_been_deleted() {
    let w = world(&[("deep/er/a.bin", "aaaa")]);
    let record = w.run(false).unwrap().record.unwrap();
    fs::remove_dir_all(&w.root).unwrap();

    w.q.restore(&record.id, 2_000).unwrap();
    assert_eq!(
        fs::read_to_string(w.root.join("deep/er/a.bin")).unwrap(),
        "aaaa"
    );
}

#[test]
fn a_file_touched_after_it_went_into_the_drawer_still_restores() {
    // Restore is not cleanup: no recency rule, no scan check, no eligibility
    // filter applies to it.
    let w = world(&[("a.bin", "aaaa")]);
    let record = w.run(false).unwrap().record.unwrap();
    let held = join(&record.stored_path, "a.bin");
    fs::write(&held, "edited in the drawer, just now").unwrap();

    w.q.restore(&record.id, 2_000).unwrap();
    assert_eq!(
        fs::read_to_string(w.root.join("a.bin")).unwrap(),
        "edited in the drawer, just now"
    );
}

#[test]
#[cfg(unix)]
fn a_restore_that_cannot_finish_keeps_what_is_left_held_and_restorable() {
    use std::os::unix::fs::PermissionsExt;
    let w = world(&[
        ("ok.bin", "ok"),
        ("sub/stuck.bin", "stuck"),
        ("zed.bin", "zed"),
    ]);
    let record = w.run(false).unwrap().record.unwrap();
    // `sub` survives the move (folders are left in place). Make it unwritable.
    let sub = w.root.join("sub");
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o555)).unwrap();

    let first = w.q.restore(&record.id, 2_000).unwrap();
    assert_eq!(first.restored, 2);
    assert_eq!(first.remaining, 1);
    assert_eq!(first.failed[0].kind, FailureKind::AccessDenied);
    let still = w.store.quarantine_record(&record.id).unwrap();
    assert_eq!(still.status, QuarantineStatus::Held);
    assert_eq!(still.item_count, 1);
    assert_eq!(
        w.read_drawer(&still, "sub/stuck.bin").as_deref(),
        Some("stuck")
    );

    fs::set_permissions(&sub, fs::Permissions::from_mode(0o755)).unwrap();
    let second = w.q.restore(&record.id, 3_000).unwrap();
    assert_eq!((second.restored, second.remaining), (1, 0));
    assert_eq!(fs::read_to_string(sub.join("stuck.bin")).unwrap(), "stuck");
}

// ---------------------------------------------------------------------------
// Interrupted work is told apart, never guessed at, never deleted
// ---------------------------------------------------------------------------

/// Fails at one step by *panicking*, which leaves everything exactly as a
/// process that died there would: no cleanup runs, nothing is settled.
struct CrashAt(Step);

impl Faults for CrashAt {
    fn at(&self, step: Step) -> Option<io::Error> {
        if step == self.0 {
            panic!("simulated crash at {step:?}");
        }
        (step == Step::Rename).then(|| io::Error::from_raw_os_error(exdev()))
    }
}

fn silently<T>(f: impl FnOnce() -> T) -> std::thread::Result<T> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(hook);
    out
}

#[test]
fn a_crash_mid_batch_is_settled_from_the_checkpoint_on_the_next_start() {
    let files = ten();
    let w = world_of(&files);

    let crashed = silently(|| {
        w.run_with(false, &Ctl::none(), &|t| {
            if t.processed == 4 {
                panic!("power cut");
            }
        })
    });
    assert!(crashed.is_err());

    // In flight: invisible, not counted, not sweepable.
    assert!(w.store.held_quarantine().unwrap().is_empty());
    assert_eq!(w.q.sweep_expired(i64::MAX / 2).unwrap(), 0);

    let report = w.q.reconcile(2_000).unwrap();
    assert_eq!(
        report,
        Reconciled {
            settled: 1,
            attention: 0,
            orphan_cells: 0
        }
    );

    let held = w.store.held_quarantine().unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].item_count, 4, "exactly the files that really moved");
    assert!(!held[0].attention);
    let moved = files
        .iter()
        .filter(|(rel, _)| !w.root.join(rel).exists())
        .count();
    assert_eq!(moved, 4);

    // And it restores.
    w.q.restore(&held[0].id, 3_000).unwrap();
    for (rel, body) in &files {
        assert_eq!(fs::read_to_string(w.root.join(rel)).unwrap(), *body);
    }
}

#[test]
fn a_crash_after_the_last_file_but_before_settling_still_yields_a_complete_record() {
    let files = ten();
    let w = world_of(&files);
    let crashed = silently(|| {
        w.run_with(false, &Ctl::none(), &|t| {
            if t.processed == 10 {
                panic!("power cut before the record settled");
            }
        })
    });
    assert!(crashed.is_err());

    w.q.reconcile(2_000).unwrap();
    let held = w.store.held_quarantine().unwrap();
    assert_eq!(held[0].item_count, 10);
    assert_eq!(
        held[0].size,
        files.iter().map(|(_, b)| b.len() as u64).sum::<u64>()
    );
}

#[test]
fn a_copy_published_but_not_yet_removed_from_its_source_is_never_counted_as_moved() {
    let w = world(&[("a.bin", "aaaa")]);
    let crash = CrashAt(Step::RemoveSource);
    let ctl = Ctl {
        faults: Some(&crash),
        ..Ctl::default()
    };
    assert!(silently(|| w.run_with(false, &ctl, &|_| {})).is_err());

    let report = w.q.reconcile(2_000).unwrap();
    assert_eq!(report.attention, 1);

    let held = w.store.held_quarantine().unwrap();
    assert_eq!(held.len(), 1);
    assert!(held[0].attention, "flagged for a person to look at");
    assert_eq!(held[0].item_count, 0, "not counted as moved");
    assert_eq!(held[0].size, 0);
    assert_eq!(
        fs::read_to_string(w.root.join("a.bin")).unwrap(),
        "aaaa",
        "the original is untouched"
    );
    assert_eq!(
        w.read_drawer(&held[0], "a.bin").as_deref(),
        Some("aaaa"),
        "and the verified copy is kept, not deleted"
    );
    // Not eligible for the expiry sweep while it needs a look.
    assert_eq!(w.q.sweep_expired(i64::MAX / 2).unwrap(), 0);
}

#[test]
fn a_half_written_copy_is_kept_flagged_and_never_shown_as_held_content() {
    for step in [Step::CopyWrite, Step::PublishPart] {
        let w = world(&[("a.bin", "aaaa")]);
        let crash = CrashAt(step);
        let ctl = Ctl {
            faults: Some(&crash),
            ..Ctl::default()
        };
        assert!(silently(|| w.run_with(false, &ctl, &|_| {})).is_err());

        w.q.reconcile(2_000).unwrap();

        assert_eq!(
            fs::read_to_string(w.root.join("a.bin")).unwrap(),
            "aaaa",
            "{step:?}: original intact"
        );
        let parts: Vec<PathBuf> = walk(w.q.root())
            .into_iter()
            .filter(|p| p.to_string_lossy().ends_with(transfer::PART_SUFFIX))
            .collect();
        assert_eq!(
            parts.len(),
            1,
            "{step:?}: an uncertain artifact is not deleted"
        );

        let held = w.store.held_quarantine().unwrap();
        assert_eq!(held.len(), 1, "{step:?}: it must be visible, not hidden");
        assert!(held[0].attention);
        assert_eq!(
            held[0].item_count, 0,
            "{step:?}: a partial copy is not content"
        );
        assert_eq!(w.q.held_bytes().unwrap(), 0);
    }
}

#[test]
fn a_move_that_never_got_going_leaves_no_record_behind() {
    let w = world(&[("a.bin", "aaaa")]);
    let crash = CrashAt(Step::Rename);
    let ctl = Ctl {
        faults: Some(&crash),
        ..Ctl::default()
    };
    assert!(silently(|| w.run_with(false, &ctl, &|_| {})).is_err());

    let report = w.q.reconcile(2_000).unwrap();
    assert_eq!(report.attention, 0);
    assert!(w.store.held_quarantine().unwrap().is_empty());
    assert!(w
        .store
        .records_with_status(QuarantineStatus::Moving)
        .unwrap()
        .is_empty());
    assert_eq!(fs::read_to_string(w.root.join("a.bin")).unwrap(), "aaaa");
    assert!(
        fs::read_dir(w.q.root()).map(|r| r.count()).unwrap_or(0) == 0,
        "an empty cell is tidied; nothing else is"
    );
}

#[test]
fn a_crash_in_the_middle_of_a_restore_is_settled_the_same_careful_way() {
    let files = ten();
    let w = world_of(&files);
    let record = w.run(false).unwrap().record.unwrap();

    // Die on the fourth file put back.
    struct DiesOnFourth(std::cell::Cell<u32>);
    impl Faults for DiesOnFourth {
        fn at(&self, step: Step) -> Option<io::Error> {
            if step == Step::Rename {
                self.0.set(self.0.get() + 1);
                if self.0.get() == 4 {
                    panic!("simulated crash during restore");
                }
            }
            None
        }
    }
    let faults = DiesOnFourth(std::cell::Cell::new(0));
    let ctl = Ctl {
        faults: Some(&faults),
        ..Ctl::default()
    };
    let stored = w.store.quarantine_record(&record.id).unwrap();
    assert!(silently(|| w.q.restore_contents(&stored, 2_000, &ctl)).is_err());
    assert_eq!(
        w.store.quarantine_record(&record.id).unwrap().status,
        QuarantineStatus::Restoring
    );
    assert!(
        w.store.held_quarantine().unwrap().is_empty(),
        "not presented as held while in flight"
    );

    let report = w.q.reconcile(3_000).unwrap();
    assert_eq!(report.attention, 0);

    let after = w.store.quarantine_record(&record.id).unwrap();
    assert_eq!(after.status, QuarantineStatus::Held);
    assert_eq!(after.item_count, 7, "three came back; seven are still held");

    // Nothing was lost between the two places, and the rest restores.
    w.q.restore(&record.id, 4_000).unwrap();
    for (rel, body) in &files {
        assert_eq!(
            fs::read_to_string(w.root.join(rel)).unwrap(),
            *body,
            "{rel}"
        );
    }
}

#[test]
fn a_whole_item_interrupted_at_each_point_is_settled_by_looking_at_the_disk() {
    let f = |name: &str, original_exists: bool, stored_exists: bool| {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let store = Arc::new(Store::in_memory().unwrap());
        let q = Quarantine::new(home.join("drawer"), Arc::clone(&store), 14);
        let original = home.join("thing.bin");
        let cell = q.root().join("11111111-1111-1111-1111-111111111111");
        let stored = cell.join("thing.bin");
        fs::create_dir_all(&cell).unwrap();
        if original_exists {
            fs::write(&original, "original").unwrap();
        }
        if stored_exists {
            fs::write(&stored, "stored").unwrap();
        }
        store
            .insert_quarantine(&QuarantineRecord {
                id: "11111111-1111-1111-1111-111111111111".into(),
                finding_id: None,
                original_path: original.clone(),
                stored_path: stored.clone(),
                display_name: name.into(),
                category: Category::Installers,
                size: 8,
                content_hash: None,
                evidence: vec![],
                quarantined_unix: 0,
                expires_unix: 0,
                status: QuarantineStatus::Moving,
                resolved_unix: None,
                mode: crate::storage::RecordMode::Whole,
                item_count: 1,
                attention: false,
            })
            .unwrap();
        q.reconcile(10).unwrap();
        (tmp, store, original, stored)
    };

    // Moved: the drawer has it and the original is gone.
    let (_t, store, original, stored) = f("moved", false, true);
    let held = store.held_quarantine().unwrap();
    assert_eq!(held.len(), 1);
    assert!(!held[0].attention);
    assert!(stored.exists() && !original.exists());

    // Not moved: nothing was made, and the record goes.
    let (_t, store, original, _) = f("never moved", true, false);
    assert!(store.held_quarantine().unwrap().is_empty());
    assert!(store
        .records_with_status(QuarantineStatus::Moving)
        .unwrap()
        .is_empty());
    assert!(original.exists());

    // Both present: keep both, flag it, delete nothing.
    let (_t, store, original, stored) = f("both", true, true);
    let held = store.held_quarantine().unwrap();
    assert_eq!(held.len(), 1);
    assert!(held[0].attention);
    assert!(original.exists() && stored.exists());
}

#[test]
fn a_folder_in_the_drawer_that_no_record_owns_is_reported_and_left_alone() {
    let w = world(&[("a.bin", "a")]);
    let orphan = w.q.root().join("22222222-2222-2222-2222-222222222222");
    fs::create_dir_all(&orphan).unwrap();
    fs::write(orphan.join("mystery.bin"), "who knows").unwrap();

    let report = w.q.reconcile(1).unwrap();
    assert_eq!(report.orphan_cells, 1);
    assert_eq!(
        fs::read_to_string(orphan.join("mystery.bin")).unwrap(),
        "who knows"
    );
}

#[test]
fn a_record_still_being_moved_cannot_be_restored_or_purged() {
    let files = ten();
    let w = world_of(&files);
    let crashed = silently(|| {
        w.run_with(false, &Ctl::none(), &|t| {
            if t.processed == 2 {
                panic!("power cut");
            }
        })
    });
    assert!(crashed.is_err());
    let moving = w
        .store
        .records_with_status(QuarantineStatus::Moving)
        .unwrap();
    assert_eq!(moving.len(), 1);
    assert_eq!(w.q.restore(&moving[0].id, 1).unwrap_err().code(), "refused");
    assert_eq!(w.q.purge(&moving[0].id, 1).unwrap_err().code(), "refused");
}

// ---------------------------------------------------------------------------
// What may never be in a reviewed set
// ---------------------------------------------------------------------------

#[test]
fn scuttles_own_files_and_the_drawer_are_never_part_of_a_reviewed_set() {
    let w = world(&[("ordinary.bin", "fine")]);
    // A drawer and a runtime folder that happen to sit inside a scanned cache.
    let own = w.root.join("app.scuttle.desktop");
    fs::create_dir_all(&own).unwrap();
    fs::write(own.join("runtime.db"), "in use right now").unwrap();
    let mut protected = ProtectedPaths::for_home(&w.home);
    protected.also_protect("Scuttle's own files", &own);
    let never = || false;
    let recorded = snapshot::record(
        &w.store,
        "f1",
        &w.root,
        &SnapshotPolicy {
            protected: &protected,
            settle_secs: 0,
            now_unix: NOW,
            cancelled: &never,
            limit: 100,
        },
    )
    .unwrap();

    assert_eq!(recorded.files, 1, "only the ordinary file is reviewed");
    let (files, _) = w.store.snapshot_actionable("f1").unwrap();
    assert_eq!(files, 1);
}

#[test]
fn recently_touched_files_in_a_settling_folder_are_left_out_of_the_set() {
    let w = world(&[("old.bin", "old"), ("fresh.bin", "fresh")]);
    let old = w.root.join("old.bin");
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86_400))
        .unwrap();
    let never = || false;
    let recorded = snapshot::record(
        &w.store,
        "f1",
        &w.root,
        &SnapshotPolicy {
            protected: &w.protected,
            settle_secs: 86_400,
            now_unix: chrono::Utc::now().timestamp(),
            cancelled: &never,
            limit: 100,
        },
    )
    .unwrap();
    assert_eq!(
        recorded.files, 1,
        "only the file untouched for a day qualifies"
    );
}

#[test]
fn a_link_inside_a_shared_folder_is_never_recorded_or_followed() {
    let w = world(&[("real.bin", "real")]);
    let outside = w.home.join("outside.txt");
    fs::write(&outside, "not the cache's").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, w.root.join("link.bin")).unwrap();
    #[cfg(windows)]
    if std::os::windows::fs::symlink_file(&outside, w.root.join("link.bin")).is_err() {
        return;
    }
    let recorded = w.review();
    assert_eq!(recorded.files, 1);

    w.run(false).unwrap();
    assert_eq!(fs::read_to_string(&outside).unwrap(), "not the cache's");
    assert!(
        w.root.join("link.bin").symlink_metadata().is_ok(),
        "the link is left where it is"
    );
}

#[test]
fn the_shared_folder_itself_is_never_moved_by_the_whole_item_path() {
    let w = world(&[("a.bin", "a")]);
    let err = w.q.hold(&w.candidate, &w.ctx(), NOW).unwrap_err();
    assert_eq!(err.code(), "refused");
    assert!(w.root.join("a.bin").exists());
}

#[test]
fn a_root_swapped_for_a_link_after_review_is_refused() {
    let w = world(&[("a.bin", "aaaa")]);
    let elsewhere = w.home.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join("a.bin"), "aaaa").unwrap();
    fs::remove_dir_all(&w.root).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&elsewhere, &w.root).unwrap();
    #[cfg(windows)]
    if std::os::windows::fs::symlink_dir(&elsewhere, &w.root).is_err() {
        return;
    }
    let err = w.run(false).unwrap_err();
    assert_eq!(err.code(), "refused", "{err}");
    assert_eq!(fs::read_to_string(elsewhere.join("a.bin")).unwrap(), "aaaa");
}

#[test]
fn an_unused_failure_in_the_helper_stays_quiet() {
    // Keeps the shared test imports honest on platforms where a helper is
    // otherwise unused.
    let _ = (eacces, in_use, journal::DONE);
}

// ---------------------------------------------------------------------------
// A real second volume
//
// Everything above forces the "different volume" answer with an injected
// fault. These run against an actual second volume, and are skipped unless
// `SCUTTLE_XVOL` names a directory on one:
//
//   hdiutil create -size 24m -fs APFS -volname ScuttleXvol x.dmg
//   hdiutil attach x.dmg -mountpoint /Volumes/ScuttleXvol -nobrowse
//   SCUTTLE_XVOL=/Volumes/ScuttleXvol cargo test real_second_volume
// ---------------------------------------------------------------------------

fn second_volume(test: &str) -> Option<PathBuf> {
    match std::env::var_os("SCUTTLE_XVOL") {
        Some(dir) => {
            let path = PathBuf::from(dir).join(format!("{test}-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Some(path)
        }
        None => {
            eprintln!("skipped: SCUTTLE_XVOL is not set");
            None
        }
    }
}

#[test]
fn real_second_volume_a_whole_folder_is_refused_and_nothing_is_lost() {
    let Some(volume) = second_volume("whole-folder") else {
        return;
    };
    // The shape that lost files in the original code: a folder that has to
    // cross drives, with one part that will not go.
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home/u");
    let dir = home.join("Library/Application Support/com.dead.app");
    fs::create_dir_all(dir.join("zzz-locked")).unwrap();
    for n in 0..300 {
        fs::write(dir.join(format!("file-{n}.bin")), format!("precious {n}")).unwrap();
    }
    fs::write(dir.join("zzz-locked/held.bin"), "locked one").unwrap();

    let store = Arc::new(Store::in_memory().unwrap());
    let q = Quarantine::new(volume.join("drawer"), Arc::clone(&store), 14);
    let protected = ProtectedPaths::for_home(&home);
    let roots = vec![home.clone()];
    let ctx = ActionContext {
        protected: &protected,
        allowed_roots: &roots,
        bidding: Bidding::Scuttle,
    };
    let fingerprint = safety::observe(&dir).unwrap();
    let candidate = CleanupCandidate {
        id: "f1".into(),
        detector: "ghosts".into(),
        category: Category::Ghosts,
        target_kind: TargetKind::Directory,
        path: dir.clone(),
        display_name: "com.dead.app".into(),
        associated_app: None,
        size: 1000,
        group_bytes: 1000,
        confidence: Confidence::High,
        risk: Risk::Low,
        recommended_action: RecommendedAction::Quarantine,
        evidence: vec![],
        remark: None,
        modified_unix: fingerprint.modified_unix,
        accessed_unix: None,
        created_unix: None,
        group: vec![],
        fingerprint,
    };

    let err = q.hold(&candidate, &ctx, 1).unwrap_err();

    match err {
        crate::ScuttleError::Transfer(f) => assert_eq!(f.kind, FailureKind::CrossVolumeRefused),
        other => panic!("expected a refusal to cross drives, got {other:?}"),
    }
    for n in 0..300 {
        assert_eq!(
            fs::read_to_string(dir.join(format!("file-{n}.bin"))).unwrap(),
            format!("precious {n}"),
            "file-{n} must be exactly where it was"
        );
    }
    assert!(store.held_quarantine().unwrap().is_empty());
    let _ = fs::remove_dir_all(&volume);
}

#[test]
fn real_second_volume_files_are_copied_verified_moved_and_restored() {
    let Some(volume) = second_volume("contents") else {
        return;
    };
    let w = world_on(Some(volume.join("drawer")), &[]);
    for n in 0..40 {
        w.write(
            &format!("d{}/f{n}.bin", n % 4),
            &format!("body of file {n}"),
        );
    }
    let big = w.root.join("big.bin");
    fs::write(&big, vec![7u8; 3 * 1024 * 1024 + 17]).unwrap();
    w.review();

    let outcome = w.run(false).unwrap();

    assert_eq!(outcome.tally.moved, 41, "{:?}", outcome.issues);
    assert!(
        outcome.tally.copied_bytes > 3 * 1024 * 1024,
        "real bytes crossed a real boundary"
    );
    let record = outcome.record.unwrap();
    assert!(record.stored_path.starts_with(&volume));
    assert_eq!(
        fs::read(join(&record.stored_path, "big.bin"))
            .unwrap()
            .len(),
        3 * 1024 * 1024 + 17
    );
    assert!(!big.exists());
    assert_eq!(
        walk(&volume)
            .iter()
            .filter(|p| p.to_string_lossy().ends_with(transfer::PART_SUFFIX))
            .count(),
        0
    );

    let back = w.q.restore(&record.id, 2).unwrap();
    assert_eq!(back.restored, 41);
    assert_eq!(fs::read(&big).unwrap().len(), 3 * 1024 * 1024 + 17);
    assert_eq!(
        fs::read_to_string(w.root.join("d3/f39.bin")).unwrap(),
        "body of file 39"
    );
    let _ = fs::remove_dir_all(&volume);
}

#[test]
fn real_second_volume_a_drive_that_fills_up_mid_run_loses_nothing_and_leaves_no_partial() {
    let Some(volume) = second_volume("full") else {
        return;
    };
    let w = world_on(Some(volume.join("drawer")), &[]);
    // Far more than the volume can hold.
    let bodies: Vec<Vec<u8>> = (0..8u8).map(|n| vec![n; 4 * 1024 * 1024]).collect();
    for (n, body) in bodies.iter().enumerate() {
        fs::write(w.root.join(format!("chunk-{n}.bin")), body).unwrap();
    }
    w.review();
    // Skip the up-front room check so the disk really runs out part-way.
    super::volume::testing::FREE.with(|f| f.set(Some(u64::MAX)));

    let outcome = w.run(false).unwrap();
    super::volume::testing::FREE.with(|f| f.set(None));

    assert!(
        outcome.tally.moved > 0 && outcome.tally.moved < 8,
        "moved {}",
        outcome.tally.moved
    );
    assert!(
        outcome
            .issues
            .groups()
            .iter()
            .any(|g| g.kind == FailureKind::NoSpace),
        "{:?}",
        outcome.issues
    );
    let record = outcome.record.unwrap();
    for (n, body) in bodies.iter().enumerate() {
        let name = format!("chunk-{n}.bin");
        let in_source = fs::read(w.root.join(&name)).ok().as_deref() == Some(&body[..]);
        let in_drawer =
            fs::read(join(&record.stored_path, &name)).ok().as_deref() == Some(&body[..]);
        assert!(in_source || in_drawer, "{name}: content is nowhere");
        assert!(
            !(in_source && in_drawer),
            "{name}: a failed move left a duplicate"
        );
    }
    assert_eq!(
        walk(&volume)
            .iter()
            .filter(|p| p.to_string_lossy().ends_with(transfer::PART_SUFFIX))
            .count(),
        0
    );
    let _ = fs::remove_dir_all(&volume);
}
