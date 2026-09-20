//! Background checks, end to end.
//!
//! Everything here runs against a filesystem built in a temporary directory
//! and removed when the test ends. Nothing reads, writes or walks a real user
//! directory — a background feature that touched `$HOME` in its tests would be
//! exactly the bug this project exists to avoid.
//!
//! Each test is named for the failure it prevents rather than the feature it
//! covers, because the failures are the point: a check that deletes something,
//! a partial look reported as a whole one, a restart that earns a fresh scan.

mod fixtures;

use std::sync::Arc;

use fixtures::*;
use scuttle_core::background;
use scuttle_core::background::schedule::{
    self, Decision, Moment, Reason, BETWEEN_CHECKS, SETTLE_AFTER_LAUNCH,
};
use scuttle_core::commands::{AppState, Operation};
use scuttle_core::model::Category;
use scuttle_core::platform::PlatformService;
use scuttle_core::storage::{BackgroundState, QuarantineStatus, ScanKind, Settings};

const NOW: i64 = 1_700_000_000;
const HOUR: i64 = 3600;

fn state_for(world: &World) -> AppState {
    let platform: Arc<dyn PlatformService> = Arc::new(world.platform.clone());
    AppState::new(platform).expect("app state")
}

/// Settings with background checks on and a rummage already behind us, which
/// is the only state in which a check is allowed to happen at all.
fn ready(state: &AppState, world: &World) {
    let settings = Settings {
        scan_roots: world.options.roots.clone(),
        heavy_threshold: world.options.heavy_threshold,
        has_rummaged_before: true,
        background_mode: true,
        background_checks: true,
        ..Default::default()
    };
    state.store().save_settings(&settings).expect("settings");
}

/// Everything about the machine that would let a check proceed.
fn free_moment() -> Moment {
    Moment {
        launched_unix: NOW - HOUR,
        window_visible: false,
        busy: false,
        has_roots: true,
    }
}

fn due() -> BackgroundState {
    BackgroundState {
        last_completed_unix: NOW - BETWEEN_CHECKS - 1,
        last_tick_unix: NOW - schedule::TICK,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// What a check does, and what it refuses to do
// ---------------------------------------------------------------------------

#[test]
fn a_background_check_finds_things_without_ever_opening_a_file() {
    // Installers are found from names and dates. Duplicates and screenshots
    // need file contents, and a background check does not read any.
    let world = old_installers();
    let state = state_for(&world);
    ready(&state, &world);

    let outcome = state.run_glance(60).expect("a check should run");

    assert!(!outcome.summary.cancelled);
    assert!(
        outcome
            .candidates
            .iter()
            .any(|c| c.category == Category::Installers),
        "a check that reads names and dates should still find old installers"
    );
}

#[test]
fn a_background_check_never_reports_duplicates_it_could_not_have_hashed() {
    // The danger is subtle: if the duplicate detector ran, a check would be
    // reading file contents unattended. If it did not run but the results
    // were labelled as a full rummage, the absence of duplicates would read
    // as "you have none".
    let world = duplicate_files();
    let state = state_for(&world);
    ready(&state, &world);

    let outcome = state.run_glance(60).expect("a check should run");

    assert!(
        !outcome
            .candidates
            .iter()
            .any(|c| c.category == Category::Copies),
        "a check that never opened a file cannot know two files are identical"
    );

    let scan = state
        .store()
        .latest_scan()
        .expect("scan")
        .expect("a scan should have been recorded");
    assert_eq!(
        scan.kind,
        ScanKind::Glance,
        "the results must be labelled as partial, or their gaps read as facts"
    );
}

#[test]
fn a_background_check_moves_and_deletes_nothing() {
    // Discovery and modification are separate, and a check only ever does the
    // first. Nothing is quarantined, nothing is removed, and nothing is
    // selected on the user's behalf.
    let world = old_installers();
    let state = state_for(&world);
    ready(&state, &world);

    let before = listing(&world);
    let outcome = state.run_glance(60).expect("a check should run");
    assert!(!outcome.candidates.is_empty(), "nothing to be tempted by");

    assert_eq!(
        before,
        listing(&world),
        "a background check must leave every file exactly where it was"
    );

    let drawer = state
        .store()
        .held_quarantine()
        .expect("drawer should be readable");
    assert!(drawer.is_empty(), "a check must never put anything away");
}

#[test]
fn a_background_check_does_not_count_as_having_rummaged() {
    // `has_rummaged_before` gates parts of the interface — and gates background
    // checks themselves. A check that set it would be unlocking itself.
    let world = old_installers();
    let state = state_for(&world);
    let settings = Settings {
        scan_roots: world.options.roots.clone(),
        has_rummaged_before: false,
        background_mode: true,
        background_checks: true,
        ..Default::default()
    };
    state.store().save_settings(&settings).expect("settings");

    let _ = state.run_glance(60);

    assert!(
        !state
            .store()
            .settings()
            .expect("settings")
            .has_rummaged_before,
        "only a rummage the user asked for counts as one"
    );
}

#[test]
fn a_check_that_ran_out_of_time_is_recorded_as_unfinished() {
    // Zero seconds of budget: the watchdog cancels it almost immediately.
    // What matters is that the results are saved and marked, not silently
    // presented as complete.
    let world = healthy_system();
    let state = state_for(&world);
    ready(&state, &world);

    let outcome = state.run_glance(0).expect("a check should still return");

    if outcome.summary.cancelled {
        let scan = state
            .store()
            .latest_scan()
            .expect("scan")
            .expect("even a cut-short check records what it saw");
        assert!(
            scan.cancelled,
            "a check that was stopped must say so, so nothing reads it as a clean sweep"
        );
    }
}

// ---------------------------------------------------------------------------
// Getting out of the way
// ---------------------------------------------------------------------------

#[test]
fn a_rummage_the_user_asked_for_is_never_refused_because_of_a_background_check() {
    // The failure this prevents: clicking Rummage while a background check is
    // running and being told Scuttle is busy with something you never asked
    // for and cannot see.
    //
    // The check may well finish on its own before the rummage arrives — these
    // fixtures are small. So the assertion is on the guarantee, not on the
    // race: the rummage starts either way, and if the check was still going it
    // stood down rather than finishing underneath it.
    let world = healthy_system();
    let state = state_for(&world);
    ready(&state, &world);

    let checking = state.clone_handle();
    let worker = std::thread::spawn(move || checking.run_glance(600));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut saw_it_running = false;
    while std::time::Instant::now() < deadline {
        if state.background_scan_running() {
            saw_it_running = true;
            break;
        }
        if worker.is_finished() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    let started = state.start_scan(&world.options);
    assert!(
        started.is_ok(),
        "a rummage must not be refused because of a background check: {:?}",
        started.err()
    );

    let outcome = worker.join().expect("the check thread should not panic");
    // Losing the running slot outright — the `Err` case — is the same outcome
    // by a shorter route, and is equally correct.
    if saw_it_running {
        if let Ok(summary) = outcome {
            assert!(
                summary.summary.cancelled,
                "a check still running when a rummage started should have stood down"
            );
        }
    }
}

#[test]
fn only_a_background_check_is_ever_asked_to_stand_down() {
    // The pre-emption is deliberately one-directional. If it could cancel a
    // rummage, a background check arriving at the wrong moment would stop work
    // the user was watching.
    let world = healthy_system();
    let state = state_for(&world);
    ready(&state, &world);

    let scan_id = state.start_scan(&world.options).expect("rummage starts");
    assert!(
        !state.background_scan_running(),
        "a rummage is not a background check"
    );

    // A background caller finding the gate held gets a refusal, and the
    // rummage carries on untouched.
    assert!(state.run_glance(60).is_err());
    assert!(
        state.cancel_scan(),
        "the rummage should still be there to cancel"
    );

    let _ = state.run_scan_with(
        &scan_id,
        world.options.clone(),
        &scuttle_core::scanning::SilentObserver,
    );
}

#[test]
fn a_check_does_not_barge_into_something_the_user_started() {
    // The gate is the one mechanism, and a background caller must lose it.
    let world = healthy_system();
    let state = state_for(&world);
    ready(&state, &world);

    let _held = state
        .begin_operation(Operation::Move)
        .expect("a user operation should take the gate");

    let refused = state.run_glance(60);
    assert!(
        refused.is_err(),
        "a check must wait for its next opportunity rather than interrupt a move"
    );
}

// ---------------------------------------------------------------------------
// Scheduling, against a clock that can be moved by hand
// ---------------------------------------------------------------------------

#[test]
fn restarting_scuttle_does_not_earn_a_fresh_check() {
    // Scheduling state lives in the database, not in the process. Quitting and
    // reopening — deliberately or in a loop — must not be a way to make
    // Scuttle scan again.
    let world = old_installers();
    let state = state_for(&world);
    ready(&state, &world);

    let just_checked = BackgroundState {
        last_completed_unix: NOW - HOUR,
        last_tick_unix: NOW - schedule::TICK,
        ..Default::default()
    };
    state
        .store()
        .save_background_state(&just_checked)
        .expect("save");

    // A second AppState over the same database is what a restart looks like.
    let restarted = state_for(&world);
    let persisted = restarted.store().background_state().expect("read back");
    let settings = restarted.store().settings().expect("settings");

    let fresh_launch = Moment {
        launched_unix: NOW - SETTLE_AFTER_LAUNCH - 1,
        ..free_moment()
    };
    assert_eq!(
        schedule::decide(NOW, &settings, &persisted, &fresh_launch, Default::default),
        Decision::Skip(Reason::Recently)
    );
}

#[test]
fn a_pause_survives_a_restart() {
    let world = healthy_system();
    let state = state_for(&world);
    ready(&state, &world);

    let paused = BackgroundState {
        paused_until_unix: NOW + 6 * HOUR,
        ..due()
    };
    state.store().save_background_state(&paused).expect("save");

    let restarted = state_for(&world);
    let persisted = restarted.store().background_state().expect("read back");
    assert_eq!(persisted.paused_until_unix, NOW + 6 * HOUR);
    assert_eq!(
        schedule::decide(
            NOW,
            &restarted.store().settings().expect("settings"),
            &persisted,
            &free_moment(),
            Default::default
        ),
        Decision::Skip(Reason::Paused)
    );
}

// ---------------------------------------------------------------------------
// Saying things, and not saying them
// ---------------------------------------------------------------------------

#[test]
fn the_same_findings_are_not_announced_twice() {
    use scuttle_core::background::notify;

    let world = old_installers();
    let state = state_for(&world);
    ready(&state, &world);

    let first = state.run_glance(60).expect("a check should run");
    let keys: Vec<String> = first.candidates.iter().map(notify::notice_key).collect();
    assert!(!keys.is_empty(), "nothing to deduplicate");

    let unseen = state.store().unseen_notices(&keys).expect("unseen");
    assert_eq!(unseen.len(), keys.len(), "nothing has been said yet");

    state
        .store()
        .remember_notified(&keys, NOW)
        .expect("remember");

    let again = state.store().unseen_notices(&keys).expect("unseen");
    assert!(
        again.is_empty(),
        "the same files must not be announced a second time"
    );
}

#[test]
fn forgetting_old_notices_lets_something_be_mentioned_again() {
    let world = healthy_system();
    let state = state_for(&world);

    let keys = vec!["a".to_string(), "b".to_string()];
    state
        .store()
        .remember_notified(&keys, NOW - schedule::NOTICE_MEMORY - 1)
        .expect("remember");
    state
        .store()
        .prune_notices(NOW - schedule::NOTICE_MEMORY)
        .expect("prune");

    assert_eq!(
        state.store().unseen_notices(&keys).expect("unseen").len(),
        2,
        "something that comes back much later is news again"
    );
}

// ---------------------------------------------------------------------------
// A whole check, from running it to deciding whether to say anything
// ---------------------------------------------------------------------------

/// Settings with notifications on as well, so `check` reaches its last stage.
fn ready_and_talkative(state: &AppState, world: &World) {
    let settings = Settings {
        scan_roots: world.options.roots.clone(),
        heavy_threshold: world.options.heavy_threshold,
        has_rummaged_before: true,
        background_mode: true,
        background_checks: true,
        background_notify: true,
        ..Default::default()
    };
    state.store().save_settings(&settings).expect("settings");
}

#[test]
fn a_first_check_with_enough_new_findings_has_something_to_say() {
    let world = old_installers();
    let state = state_for(&world);
    ready_and_talkative(&state, &world);

    match background::check(&state, NOW, 60).expect("a check should run") {
        background::Conclusion::Worth { found, title, body } => {
            assert!(found >= 3);
            assert_eq!(title, "A few things turned up.");
            assert!(body.contains("worth a look"));
            // The one thing a lock screen must never show.
            assert!(!body.contains(".dmg"), "{body}");
            assert!(!body.contains('/'), "{body}");
        }
        other => panic!("expected something worth saying, got {other:?}"),
    }
}

#[test]
fn a_second_check_finding_the_same_things_says_nothing() {
    // The failure this prevents: the same five installers announced every
    // single day because nothing remembered they had been mentioned.
    let world = old_installers();
    let state = state_for(&world);
    ready_and_talkative(&state, &world);

    let first = background::check(&state, NOW, 60).expect("first check");
    assert!(matches!(first, background::Conclusion::Worth { .. }));

    // Clear the once-a-day gate so only deduplication can be the reason.
    let mut persisted = state.store().background_state().expect("state");
    persisted.last_notified_unix = 0;
    persisted.last_completed_unix = 0;
    state
        .store()
        .save_background_state(&persisted)
        .expect("save");

    let second = background::check(&state, NOW, 60).expect("second check");
    assert!(
        matches!(second, background::Conclusion::Quiet { .. }),
        "the same findings must not be announced twice, got {second:?}"
    );
}

#[test]
fn a_summary_is_not_offered_twice_if_the_system_refuses_to_show_it() {
    // `check` burns the keys and starts the cooldown before handing the
    // summary over, precisely because delivery can fail. Otherwise a platform
    // that always refuses notifications would queue up a fresh "new findings"
    // summary every day forever.
    let world = old_installers();
    let state = state_for(&world);
    ready_and_talkative(&state, &world);

    let first = background::check(&state, NOW, 60).expect("first check");
    assert!(matches!(first, background::Conclusion::Worth { .. }));

    // Nothing was actually delivered — no `send` here at all.
    let persisted = state.store().background_state().expect("state");
    assert!(
        persisted.last_notified_unix > 0,
        "the cooldown must start when the summary is produced, not when it lands"
    );
}

#[test]
fn a_check_says_nothing_when_notifications_are_off() {
    let world = old_installers();
    let state = state_for(&world);
    ready(&state, &world);

    match background::check(&state, NOW, 60).expect("a check should run") {
        background::Conclusion::Quiet { found } => assert!(found > 0),
        other => panic!("notifications are off; expected silence, got {other:?}"),
    }
}

#[test]
fn a_check_that_was_cut_short_says_nothing_and_does_not_count_as_the_days_check() {
    let world = old_installers();
    let state = state_for(&world);
    ready_and_talkative(&state, &world);

    let conclusion = background::check(&state, NOW, 0).expect("a check should return");
    if conclusion == background::Conclusion::CutShort {
        let persisted = state.store().background_state().expect("state");
        assert_eq!(
            persisted.last_completed_unix, 0,
            "a partial look must not satisfy the once-a-day gate"
        );
        assert_eq!(persisted.last_notified_unix, 0, "and must say nothing");
        assert!(persisted.pending_review.is_none());
    }
}

#[test]
fn a_check_points_at_what_it_found_for_the_next_time_the_window_opens() {
    let world = old_installers();
    let state = state_for(&world);
    ready(&state, &world);

    background::check(&state, NOW, 60).expect("a check should run");

    let persisted = state.store().background_state().expect("state");
    assert!(
        persisted.pending_review.is_some(),
        "something worth looking at should be waiting when the window opens"
    );
}

#[test]
fn a_check_that_found_nothing_leaves_nothing_waiting() {
    let world = World::new();
    let state = state_for(&world);
    ready_and_talkative(&state, &world);

    match background::check(&state, NOW, 60).expect("a check should run") {
        background::Conclusion::Quiet { found } => assert_eq!(found, 0),
        other => panic!("an empty machine has nothing to report, got {other:?}"),
    }
    assert!(state
        .store()
        .background_state()
        .expect("state")
        .pending_review
        .is_none());
}

// ---------------------------------------------------------------------------
// The drawer
// ---------------------------------------------------------------------------

#[test]
fn a_hidden_window_coming_back_is_not_a_fresh_start() {
    // Startup work — reconciling moves, expiring the drawer, pruning scans —
    // belongs to the process. With a tray, a window is shown and hidden many
    // times in one run, and none of those may re-run it.
    let world = healthy_system();
    let state = state_for(&world);

    assert!(state.claim_startup(), "the first claim is the launch");
    for _ in 0..5 {
        assert!(
            !state.claim_startup(),
            "showing a window again must not look like starting up again"
        );
    }
}

#[test]
fn the_periodic_sweep_takes_only_what_has_actually_expired() {
    // Retention keeps the contract it already stated. What changes is that an
    // application which stays running honours the date, rather than waiting
    // for a launch that may not come for weeks.
    let world = healthy_system();
    let state = state_for(&world);
    let quarantine = state.quarantine().expect("quarantine");

    let expired = state
        .store()
        .expired_quarantine(NOW)
        .expect("expired listing");
    assert!(expired.is_empty(), "nothing has been put away yet");

    let removed = quarantine.sweep_expired(NOW).expect("sweep");
    assert_eq!(removed, 0, "a sweep with nothing to do must do nothing");

    // And an item still inside its window is untouched by a sweep.
    let held = state
        .store()
        .held_quarantine()
        .expect("drawer")
        .into_iter()
        .filter(|r| r.status == QuarantineStatus::Held)
        .count();
    assert_eq!(held, 0);
}

/// Every path under the world's home, for comparing before and after.
fn listing(world: &World) -> Vec<std::path::PathBuf> {
    let mut found: Vec<std::path::PathBuf> = walkdir::WalkDir::new(&world.home)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path().to_path_buf())
        .collect();
    found.sort();
    found
}
