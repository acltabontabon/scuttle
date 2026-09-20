//! Moving findings into the Drawer as the application does it: started from
//! a command, run on a worker, reported as events.
//!
//! These tests prove the *contract* of the job — what is reported and in what
//! order, what refuses to run alongside it, what happens when it is stopped.
//! They do not prove the window stays responsive: a worker thread finishing
//! while a test thread waits says nothing about a webview's event loop. That is
//! measured separately (see the main-thread probe in the desktop build).

mod fixtures;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fixtures::*;
use scuttle_core::commands::{
    AppState, FindingStatus, MovePhase, MoveRequest, MoveSink, MoveSnapshot, Operation,
};
use scuttle_core::model::Category;
use scuttle_core::platform::caches::{CacheRule, CacheSafety};
use scuttle_core::platform::PlatformService;
use scuttle_core::scanning::{ScanOptions, SilentObserver};

/// Collects every snapshot the interface would have been sent.
#[derive(Default)]
struct Collector {
    seen: Mutex<Vec<MoveSnapshot>>,
}

impl MoveSink for Collector {
    fn emit(&self, snapshot: &MoveSnapshot) {
        self.seen.lock().unwrap().push(snapshot.clone());
    }
}

impl Collector {
    fn all(&self) -> Vec<MoveSnapshot> {
        self.seen.lock().unwrap().clone()
    }
}

fn state_for(world: &World) -> (AppState, ScanOptions) {
    let platform: Arc<dyn PlatformService> = Arc::new(world.platform.clone());
    (
        AppState::new(platform).expect("app state"),
        world.options.clone(),
    )
}

fn scan(state: &AppState, options: ScanOptions) -> String {
    let id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&id, options, &SilentObserver)
        .expect("scan");
    id
}

/// A cache with `files` small files plus one big one, being written to.
fn cache_world(files: usize) -> World {
    let mut world = World::new();
    world.file("Library/Caches/owner/big.bin", 40, 60 * MB);
    for n in 0..files {
        world.file(
            &format!("Library/Caches/owner/d{}/s{n}.bin", n % 5),
            40,
            512,
        );
    }
    world.platform = world.platform.clone().with_cache_rule(CacheRule {
        owner: "Owner",
        label: "Owner cache",
        path: world.path("Library/Caches/owner"),
        safety: CacheSafety::Regenerates,
        owner_process: None,
        developer_only: false,
        settle_secs: 0,
    });
    world
}

fn cache_id(state: &AppState, scan_id: &str) -> String {
    state
        .store()
        .candidates_for_scan(scan_id)
        .unwrap()
        .into_iter()
        .find(|c| c.category == Category::Caches)
        .expect("the cache is found")
        .id
}

fn select(ids: Vec<String>) -> MoveRequest {
    MoveRequest::Selection { ids, retry: false }
}

#[test]
fn a_move_reports_its_phases_in_order_with_rising_revisions_and_one_final_snapshot() {
    let world = cache_world(300);
    let (state, options) = state_for(&world);
    let scan_id = scan(&state, options);
    let id = cache_id(&state, &scan_id);

    let sink = Arc::new(Collector::default());
    let (job_id, worker) = state.spawn_move(select(vec![id]), sink.clone()).unwrap();
    worker.join().unwrap();

    let seen = sink.all();
    assert!(seen.iter().all(|s| s.job_id == job_id), "one job, one id");
    assert!(
        seen.windows(2).all(|w| w[0].rev < w[1].rev),
        "revisions must strictly rise: {:?}",
        seen.iter().map(|s| s.rev).collect::<Vec<_>>()
    );

    let phases: Vec<MovePhase> = seen.iter().map(|s| s.phase).collect();
    let position = |wanted: MovePhase| phases.iter().position(|p| *p == wanted);
    assert_eq!(phases.first(), Some(&MovePhase::Checking));
    assert!(position(MovePhase::Moving).is_some());
    assert!(position(MovePhase::Finalizing).is_some());
    assert!(position(MovePhase::Moving) < position(MovePhase::Finalizing));
    assert_eq!(phases.last(), Some(&MovePhase::Completed));
    assert_eq!(
        phases.iter().filter(|p| p.is_terminal()).count(),
        1,
        "exactly one terminal snapshot, and it is last"
    );

    // Nothing reads as done before it is: the total is unknown while
    // checking, and the report exists only on the last snapshot.
    for s in seen.iter().filter(|s| s.phase == MovePhase::Checking) {
        assert_eq!(s.total, None);
    }
    for s in &seen[..seen.len() - 1] {
        assert!(s.report.is_none());
    }
    let last = seen.last().unwrap();
    let report = last
        .report
        .as_ref()
        .expect("the last snapshot carries the report");
    assert_eq!(report.moved_files, 301);
    assert_eq!(last.moved, 301);
    assert_eq!(last.total, Some(301));
    assert_eq!(report.outcome, MovePhase::Completed);
    assert_eq!(report.findings[0].status, FindingStatus::Moved);
    assert!(
        world.path("Library/Caches/owner").is_dir(),
        "the shared folder is still there"
    );
}

#[test]
fn starting_a_move_returns_at_once_even_when_the_listener_is_slow() {
    // The start command must not do the work. A listener that takes a long
    // time to accept an event slows the worker, never the caller.
    struct Sluggish(AtomicBool);
    impl MoveSink for Sluggish {
        fn emit(&self, s: &MoveSnapshot) {
            if s.phase == MovePhase::Moving && !self.0.swap(true, Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(600));
            }
        }
    }
    let world = cache_world(50);
    let (state, options) = state_for(&world);
    let scan_id = scan(&state, options);
    let id = cache_id(&state, &scan_id);

    let started = Instant::now();
    let (_, worker) = state
        .spawn_move(select(vec![id]), Arc::new(Sluggish(AtomicBool::new(false))))
        .unwrap();
    let took = started.elapsed();
    worker.join().unwrap();

    assert!(
        took < Duration::from_millis(250),
        "starting took {took:?}; it must not wait for the work"
    );
}

#[test]
fn progress_is_throttled_but_the_totals_are_exact() {
    let world = cache_world(3_000);
    let (state, options) = state_for(&world);
    let scan_id = scan(&state, options);
    let id = cache_id(&state, &scan_id);

    let sink = Arc::new(Collector::default());
    let (_, worker) = state.spawn_move(select(vec![id]), sink.clone()).unwrap();
    worker.join().unwrap();

    let seen = sink.all();
    assert!(
        seen.len() < 200,
        "{} events for 3,001 files: reporting must not become the workload",
        seen.len()
    );
    let last = seen.last().unwrap();
    assert_eq!(last.moved, 3_001);
    let moved_counts: Vec<u64> = seen.iter().map(|s| s.moved).collect();
    assert!(
        moved_counts.windows(2).all(|w| w[0] <= w[1]),
        "counts never go back"
    );
    assert!(seen
        .iter()
        .all(|s| s.processed <= s.total.unwrap_or(u64::MAX)));
}

#[test]
fn nothing_else_that_changes_files_can_run_alongside_a_move() {
    // Park the worker in its first `moving` event so the move is definitely
    // in flight while the conflicts are tried.
    struct Parks {
        parked: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
        done: AtomicBool,
    }
    impl MoveSink for Parks {
        fn emit(&self, s: &MoveSnapshot) {
            if s.phase == MovePhase::Moving && !self.done.swap(true, Ordering::SeqCst) {
                self.parked.send(()).unwrap();
                let _ = self.release.lock().unwrap().recv();
            }
        }
    }
    let world = cache_world(20);
    let (state, options) = state_for(&world);
    let scan_id = scan(&state, options.clone());
    let id = cache_id(&state, &scan_id);

    let (parked_tx, parked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let sink = Arc::new(Parks {
        parked: parked_tx,
        release: Mutex::new(release_rx),
        done: AtomicBool::new(false),
    });
    let (_, worker) = state.spawn_move(select(vec![id.clone()]), sink).unwrap();
    parked_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the move is under way");

    // Every other mutation is refused, with a reason a person can read.
    let refused = |what: &str, err: scuttle_core::ScuttleError| {
        assert_eq!(err.code(), "busy", "{what}: {err}");
        assert!(err.to_string().contains("moving files"), "{what}: {err}");
    };
    refused(
        "second move",
        state
            .spawn_move(select(vec![id.clone()]), Arc::new(Collector::default()))
            .err()
            .unwrap(),
    );
    refused("scan", state.start_scan(&options).unwrap_err());
    for operation in [
        Operation::Restore,
        Operation::EmptyDrawer,
        Operation::RemoveItem,
        Operation::Refresh,
        Operation::Decide,
        Operation::Sweep,
    ] {
        refused(
            &format!("{operation:?}"),
            state.begin_operation(operation).err().unwrap(),
        );
    }
    assert_eq!(state.current_operation(), Some(Operation::Move));

    // Reads are never blocked: the interface can look around meanwhile.
    assert!(state.store().candidate(&id).is_ok());
    assert!(state.move_status().is_some());

    release_tx.send(()).unwrap();
    worker.join().unwrap();

    // And once it ends the gate is free again.
    assert_eq!(state.current_operation(), None);
    state
        .begin_operation(Operation::Restore)
        .expect("free again");
}

#[test]
fn a_move_cannot_start_while_a_scan_is_running() {
    let world = cache_world(5);
    let (state, options) = state_for(&world);
    let _scan = state.start_scan(&options).unwrap();
    let err = state
        .spawn_move(
            select(vec!["anything".into()]),
            Arc::new(Collector::default()),
        )
        .err()
        .unwrap();
    assert_eq!(err.code(), "busy");
}

#[test]
fn stopping_a_move_reports_cancelled_only_after_the_worker_has_finished() {
    // Ask for the stop from inside the first `moving` event, then check that
    // the last snapshot says so and that nothing moved.
    struct StopsEarly {
        state: Mutex<Option<AppState>>,
        stopped: AtomicBool,
    }
    impl MoveSink for StopsEarly {
        fn emit(&self, s: &MoveSnapshot) {
            if s.phase == MovePhase::Moving
                && !s.cancelling
                && !self.stopped.swap(true, Ordering::SeqCst)
            {
                let state = self.state.lock().unwrap().clone().unwrap();
                assert!(state.cancel_move());
            }
        }
    }
    let world = cache_world(200);
    let (state, options) = state_for(&world);
    let scan_id = scan(&state, options);
    let id = cache_id(&state, &scan_id);
    let sink = Arc::new(StopsEarly {
        state: Mutex::new(Some(state.clone_handle())),
        stopped: AtomicBool::new(false),
    });
    let collector = Collector::default();
    let (_, worker) = state.spawn_move(select(vec![id.clone()]), sink).unwrap();
    worker.join().unwrap();
    drop(collector);

    let status = state
        .move_status()
        .expect("the outcome is kept for a listener that missed it");
    assert_eq!(status.phase, MovePhase::Cancelled);
    assert!(status.cancelling);
    let report = status.report.unwrap();
    assert!(report.cancelled);
    assert_eq!(report.moved_files, 0);
    assert_eq!(report.remaining, 201, "what was never attempted is said so");
    assert!(world.path("Library/Caches/owner/big.bin").exists());
    assert!(
        state.store().candidate(&id).is_ok(),
        "an untouched finding stays"
    );
    assert_eq!(state.current_operation(), None);
}

#[test]
fn the_last_outcome_is_kept_for_a_late_listener_until_it_is_dismissed() {
    let world = cache_world(10);
    let (state, options) = state_for(&world);
    let scan_id = scan(&state, options);
    let id = cache_id(&state, &scan_id);
    let (_, worker) = state
        .spawn_move(select(vec![id]), Arc::new(Collector::default()))
        .unwrap();
    worker.join().unwrap();

    let status = state.move_status().expect("kept");
    assert_eq!(status.phase, MovePhase::Completed);
    assert!(status.report.is_some());

    state.dismiss_move();
    assert!(state.move_status().is_none());
}

#[test]
fn a_mixed_selection_reports_each_finding_honestly() {
    let world = {
        let w = cache_world(20);
        w.file("Downloads/Installer.dmg", 200, 12 * MB);
        w
    };
    let (state, options) = state_for(&world);
    let scan_id = scan(&state, options);
    let cache = cache_id(&state, &scan_id);
    let installer = state
        .store()
        .candidates_for_scan(&scan_id)
        .unwrap()
        .into_iter()
        .find(|c| c.path.ends_with("Installer.dmg"))
        .expect("the installer is found")
        .id;

    // The world moves before the click: one cached file changes, and the
    // installer disappears.
    std::fs::write(
        world.path("Library/Caches/owner/d0/s0.bin"),
        "rewritten and longer than before",
    )
    .unwrap();
    let installer_path = state.store().candidate(&installer).unwrap().path;
    std::fs::remove_file(&installer_path).unwrap();

    let (_, worker) = state
        .spawn_move(
            select(vec![
                cache.clone(),
                installer.clone(),
                "no-such-finding".into(),
            ]),
            Arc::new(Collector::default()),
        )
        .unwrap();
    worker.join().unwrap();

    let report = state.move_status().unwrap().report.unwrap();
    assert_eq!(report.outcome, MovePhase::Partial);
    assert_eq!(
        report.moved_files, 20,
        "every reviewed file that was still as reviewed"
    );
    assert_eq!(
        report.skipped,
        1 + 1 + 1,
        "one changed file, one gone installer, one unknown id"
    );

    let cache_result = report
        .findings
        .iter()
        .find(|f| f.finding_id == cache)
        .unwrap();
    assert_eq!(cache_result.status, FindingStatus::Partial);
    assert!(
        cache_result.needs_refresh,
        "what is left asks to be reviewed again"
    );
    assert!(cache_result.record_id.is_some());
    assert_eq!(cache_result.moved, 20);
    assert_eq!(cache_result.issues.len(), 1);

    let installer_result = report
        .findings
        .iter()
        .find(|f| f.finding_id == installer)
        .unwrap();
    assert_eq!(installer_result.status, FindingStatus::Skipped);
    assert!(installer_result.refusal.is_some());
    assert_eq!(installer_result.moved, 0);

    // The Drawer agrees with the report, to the byte.
    let held = state.store().held_quarantine().unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].size, report.moved_bytes);
}

#[test]
fn a_partial_move_can_be_restored_in_full_and_a_retry_never_takes_new_files() {
    let world = cache_world(10);
    let (state, options) = state_for(&world);
    let scan_id = scan(&state, options);
    let id = cache_id(&state, &scan_id);
    std::fs::write(
        world.path("Library/Caches/owner/d0/s0.bin"),
        "changed after review!!",
    )
    .unwrap();
    std::fs::write(
        world.path("Library/Caches/owner/late.bin"),
        "arrived after review",
    )
    .unwrap();

    let (_, worker) = state
        .spawn_move(select(vec![id.clone()]), Arc::new(Collector::default()))
        .unwrap();
    worker.join().unwrap();

    // The set now needs review; a plain second click does not go ahead.
    let (_, second) = state
        .spawn_move(select(vec![id.clone()]), Arc::new(Collector::default()))
        .unwrap();
    second.join().unwrap();
    let again = state.move_status().unwrap().report.unwrap();
    assert_eq!(again.moved_files, 0);
    assert!(again.findings[0].needs_refresh);
    assert_eq!(
        std::fs::read_to_string(world.path("Library/Caches/owner/late.bin")).unwrap(),
        "arrived after review"
    );

    // What did move restores exactly.
    let record = state.store().held_quarantine().unwrap().remove(0);
    let outcome = state.quarantine().unwrap().restore(&record.id, 0).unwrap();
    assert_eq!(outcome.restored, 10, "the big file and nine small ones");
    assert!(world.path("Library/Caches/owner/big.bin").exists());
}

#[test]
fn reviewing_again_takes_a_fresh_set_and_moves_nothing() {
    let world = cache_world(6);
    let (state, options) = state_for(&world);
    let scan_id = scan(&state, options);
    let id = cache_id(&state, &scan_id);
    std::fs::write(
        world.path("Library/Caches/owner/d0/s0.bin"),
        "changed after review!!",
    )
    .unwrap();
    let (_, worker) = state
        .spawn_move(select(vec![id.clone()]), Arc::new(Collector::default()))
        .unwrap();
    worker.join().unwrap();

    let refreshed = state.refresh_findings(std::slice::from_ref(&id)).unwrap();
    assert_eq!(refreshed.len(), 1);
    assert_eq!(
        state.store().held_quarantine().unwrap().len(),
        1,
        "a refresh moves nothing"
    );
    let info = state.store().snapshot_info(&id).unwrap();
    assert_eq!(info.state, scuttle_core::storage::SnapshotState::Complete);
    assert_eq!(
        info.files, 1,
        "only the one file that had not moved is reviewed"
    );

    let (_, again) = state
        .spawn_move(select(vec![id.clone()]), Arc::new(Collector::default()))
        .unwrap();
    again.join().unwrap();
    assert_eq!(state.move_status().unwrap().report.unwrap().moved_files, 1);
}
