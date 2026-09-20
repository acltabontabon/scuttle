//! The path a real rummage takes: start a scan, run it, persist the results,
//! read them back, act on one, put it back.
//!
//! The unit tests prove each piece works and the other integration tests prove
//! the scanner works. This proves the application's own wiring works — the
//! part that a passing test suite and a broken app have in common.

mod fixtures;

use std::sync::Arc;

use fixtures::*;
use scuttle_core::commands::AppState;
use scuttle_core::model::{Category, RecommendedAction};
use scuttle_core::platform::PlatformService;
use scuttle_core::scanning::{ScanObserver, ScanOptions, SilentObserver};

/// Records what the interface would have been told during a scan.
#[derive(Default)]
struct Recorder {
    phases: std::sync::Mutex<Vec<String>>,
    candidates: std::sync::Mutex<Vec<String>>,
    progress: std::sync::Mutex<u32>,
}

impl ScanObserver for Recorder {
    fn phase(&self, phase: scuttle_core::scanning::Phase) {
        self.phases.lock().unwrap().push(format!("{phase:?}"));
    }
    fn progress(&self, _p: &scuttle_core::scanning::Progress) {
        *self.progress.lock().unwrap() += 1;
    }
    fn candidate(&self, c: &scuttle_core::model::CleanupCandidate) {
        self.candidates.lock().unwrap().push(c.display_name.clone());
    }
    fn finished(&self, _s: &scuttle_core::scanning::ScanSummary) {}
}

fn state_for(world: &World) -> (AppState, ScanOptions) {
    let platform: Arc<dyn PlatformService> = Arc::new(world.platform.clone());
    let state = AppState::new(platform).expect("app state");
    (state, world.options.clone())
}

#[test]
fn a_rummage_persists_what_it_found_and_reads_it_back() {
    let world = old_installers();
    let (state, options) = state_for(&world);

    let scan_id = state.start_scan(&options).expect("start");
    let recorder = Recorder::default();
    let summary = state
        .run_scan_with(&scan_id, options, &recorder)
        .expect("scan");

    assert!(
        !summary.cancelled,
        "a scan nobody stopped must not report itself cancelled"
    );
    assert!(summary.files_seen > 0);
    assert!(summary.candidates_found > 0);

    // The interface was told about them as they turned up...
    assert_eq!(
        recorder.candidates.lock().unwrap().len(),
        summary.candidates_found as usize
    );
    assert!(recorder
        .phases
        .lock()
        .unwrap()
        .iter()
        .any(|p| p.contains("Rummaging")));

    // ...and they survived the trip through SQLite.
    let stored = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("read back");
    assert_eq!(stored.len(), summary.candidates_found as usize);
    assert!(stored.iter().all(|c| !c.evidence.is_empty()));

    let latest = state.latest_scan_id().expect("latest scan");
    assert_eq!(latest, scan_id);
}

#[test]
fn a_second_rummage_is_refused_while_one_is_running() {
    let world = healthy_system();
    let (state, options) = state_for(&world);

    let _first = state.start_scan(&options).expect("start");
    let error = state.start_scan(&options).expect_err("should be busy");
    assert_eq!(error.code(), "scan_busy");
}

#[test]
fn cancelling_stops_the_scan_and_says_so() {
    let world = old_installers();
    let (state, options) = state_for(&world);

    let scan_id = state.start_scan(&options).expect("start");
    state.cancel_scan();

    let summary = state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");
    assert!(summary.cancelled);
}

#[test]
fn a_finished_scan_releases_the_lock_so_the_next_one_can_start() {
    // Getting this wrong means the app scans once and then refuses forever.
    let world = healthy_system();
    let (state, options) = state_for(&world);

    for _ in 0..3 {
        let scan_id = state.start_scan(&options).expect("start");
        state
            .run_scan_with(&scan_id, options.clone(), &SilentObserver)
            .expect("scan");
    }
}

#[test]
fn a_found_thing_can_be_quarantined_and_restored_through_the_app_state() {
    let world = old_installers();
    let (state, options) = state_for(&world);

    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let candidate = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings")
        .into_iter()
        .find(|c| c.recommended_action == RecommendedAction::Quarantine)
        .expect("something worth quarantining");
    let original = candidate.path.clone();

    let record = state.hold(&candidate).expect("hold");
    assert!(!original.exists());

    // It leaves the findings, and it appears in the drawer.
    assert_eq!(
        state.store().candidate(&candidate.id).unwrap_err().code(),
        "not_found"
    );
    assert_eq!(state.store().held_quarantine().expect("drawer").len(), 1);

    let outcome = state
        .quarantine()
        .expect("quarantine")
        .restore(&record.id, 0)
        .expect("restore");
    assert_eq!(outcome.path, original);
    assert!(original.exists());
}

#[test]
fn the_space_overview_reflects_the_last_rummage() {
    let world = old_installers();
    let (state, options) = state_for(&world);

    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let overview = state.space_overview().expect("overview");
    assert!(overview.has_findings);
    assert!(overview
        .worth_checking
        .iter()
        .any(|tally| tally.category == scuttle_core::model::Category::Installers));
    // The phrasing is the core's, and it must stay hedged.
    assert!(!overview.summary.to_lowercase().contains("safely delete"));
}

#[test]
fn keeping_the_newest_holds_every_other_copy() {
    let world = duplicate_files();
    let (state, options) = state_for(&world);

    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let group = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings")
        .into_iter()
        .find(|c| c.group.len() > 1)
        .expect("a duplicate group");
    assert_eq!(group.group.len(), 3);

    let newest = group
        .group
        .iter()
        .max_by_key(|m| m.modified_unix.unwrap_or(i64::MIN))
        .expect("newest")
        .path
        .clone();

    let outcome = state
        .quarantine_group(&group.id, scuttle_core::commands::KeepChoice::Newest)
        .expect("group action");

    assert_eq!(outcome.held.len(), 2, "refused: {:?}", outcome.refused);
    assert!(outcome.refused.is_empty());
    assert!(newest.exists(), "the copy Scuttle kept must still be there");
    for record in &outcome.held {
        assert!(!record.original_path.exists());
        assert_ne!(record.original_path, newest);
    }
}

#[test]
fn keeping_the_oldest_keeps_the_other_end_of_the_group() {
    let world = duplicate_files();
    let (state, options) = state_for(&world);
    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let group = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings")
        .into_iter()
        .find(|c| c.group.len() > 1)
        .expect("a duplicate group");
    let oldest = group
        .group
        .iter()
        .min_by_key(|m| m.modified_unix.unwrap_or(i64::MAX))
        .expect("oldest")
        .path
        .clone();

    let outcome = state
        .quarantine_group(&group.id, scuttle_core::commands::KeepChoice::Oldest)
        .expect("group action");
    assert_eq!(outcome.held.len(), 2);
    assert!(oldest.exists());
}

#[test]
fn a_group_action_keeps_going_when_one_member_has_changed() {
    // Partial success is the normal case: one file of eighteen having moved
    // on since the scan is no reason to abandon the other seventeen.
    let world = duplicate_files();
    let (state, options) = state_for(&world);
    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let group = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings")
        .into_iter()
        .find(|c| c.group.len() > 1)
        .expect("a duplicate group");

    // Disturb one of the copies that is not the keeper.
    let newest = group
        .group
        .iter()
        .max_by_key(|m| m.modified_unix.unwrap_or(i64::MIN))
        .expect("newest")
        .path
        .clone();
    let victim = group
        .group
        .iter()
        .find(|m| m.path != newest)
        .expect("another copy")
        .path
        .clone();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&victim, b"changed underneath us").expect("write");

    let outcome = state
        .quarantine_group(&group.id, scuttle_core::commands::KeepChoice::Newest)
        .expect("group action");

    assert_eq!(outcome.held.len(), 1);
    assert_eq!(outcome.refused.len(), 1);
    assert_eq!(outcome.refused[0].code, "stale");
    assert!(victim.exists(), "the changed file must be left alone");
}

#[test]
fn a_group_action_is_refused_on_a_finding_scuttle_would_not_act_on() {
    let world = abandoned_game();
    let (state, options) = state_for(&world);
    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let inspect_only = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings")
        .into_iter()
        .find(|c| c.recommended_action == RecommendedAction::InspectOnly)
        .expect("something inspect-only");

    let error = state
        .quarantine_group(&inspect_only.id, scuttle_core::commands::KeepChoice::Newest)
        .expect_err("should refuse");
    assert_eq!(error.code(), "refused");
}

#[test]
fn a_bulk_action_moves_only_what_scuttle_was_confident_about() {
    // The safety line for bulk handling: anything rated Review or
    // InspectOnly has to be opened and acted on individually, because those
    // ratings exist precisely to say "look at this yourself".
    let world = abandoned_game();
    let (state, options) = state_for(&world);
    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let before = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings");
    let confident: Vec<_> = before
        .iter()
        .filter(|c| c.category == Category::Ghosts)
        .filter(|c| c.recommended_action == RecommendedAction::Quarantine)
        .cloned()
        .collect();
    let spared: Vec<_> = before
        .iter()
        .filter(|c| c.category == Category::Ghosts)
        .filter(|c| c.recommended_action != RecommendedAction::Quarantine)
        .cloned()
        .collect();
    assert!(
        !confident.is_empty(),
        "fixture should have something confident"
    );
    assert!(!spared.is_empty(), "fixture should have something to spare");

    let outcome = state
        .quarantine_confident(Category::Ghosts)
        .expect("bulk action");

    assert_eq!(
        outcome.held.len(),
        confident.len(),
        "refused: {:?}",
        outcome.refused
    );
    assert_eq!(
        outcome.bytes,
        outcome.held.iter().map(|r| r.size).sum::<u64>()
    );

    for candidate in &confident {
        assert!(
            !candidate.path.exists(),
            "{} should have moved",
            candidate.display_name
        );
    }
    for candidate in &spared {
        assert!(
            candidate.path.exists(),
            "{} was swept up by a bulk action despite being {:?}",
            candidate.display_name,
            candidate.recommended_action
        );
    }
}

#[test]
fn a_bulk_action_stays_inside_the_pile_it_was_given() {
    let world = old_installers();
    let (state, options) = state_for(&world);
    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let outcome = state
        .quarantine_confident(Category::Caches)
        .expect("bulk action");
    assert!(outcome.held.is_empty(), "no caches in this fixture");

    // ...and the installers are all still there.
    assert!(state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings")
        .iter()
        .any(|c| c.category == Category::Installers));
}

#[test]
fn a_bulk_action_keeps_going_when_one_finding_has_gone_stale() {
    let world = old_installers();
    let (state, options) = state_for(&world);
    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let confident: Vec<_> = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings")
        .into_iter()
        .filter(|c| c.category == Category::Installers)
        .filter(|c| c.recommended_action == RecommendedAction::Quarantine)
        .collect();
    assert!(confident.len() >= 2, "need at least two to disturb one");

    let victim = confident[0].path.clone();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&victim, b"changed underneath us").expect("write");

    let outcome = state
        .quarantine_confident(Category::Installers)
        .expect("bulk action");

    assert_eq!(outcome.held.len(), confident.len() - 1);
    assert_eq!(outcome.refused.len(), 1);
    assert_eq!(outcome.refused[0].code, "stale");
    assert!(victim.exists(), "the changed file must be left where it is");
}
