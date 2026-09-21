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
use scuttle_core::safety::assess::{CautionKind, Eligibility};
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
        .quarantine_group(
            &group.id,
            scuttle_core::commands::KeepChoice::Newest,
            &CautionKind::ALL,
        )
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
        .quarantine_group(
            &group.id,
            scuttle_core::commands::KeepChoice::Oldest,
            &CautionKind::ALL,
        )
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
        .quarantine_group(
            &group.id,
            scuttle_core::commands::KeepChoice::Newest,
            &CautionKind::ALL,
        )
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
        .quarantine_group(
            &inspect_only.id,
            scuttle_core::commands::KeepChoice::Newest,
            &CautionKind::ALL,
        )
        .expect_err("should refuse");
    assert_eq!(error.code(), "refused");
}

#[test]
fn a_bulk_action_moves_only_what_scuttle_suggests() {
    // The safety line for bulk handling: a sweep nobody looked at item by item
    // may only move what Scuttle suggests — rebuildable, re-downloadable
    // things with nothing to acknowledge. An application's data never
    // qualifies, however confident Scuttle is that the application is gone.
    let world = abandoned_game();
    let (state, options) = state_for(&world);
    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let ghosts: Vec<_> = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings")
        .into_iter()
        .filter(|c| c.category == Category::Ghosts)
        .collect();
    assert!(!ghosts.is_empty(), "the fixture has leftovers");
    assert!(ghosts
        .iter()
        .all(|c| c.assessment.eligibility != Eligibility::Suggested));

    let outcome = state
        .quarantine_confident(Category::Ghosts)
        .expect("bulk action");
    assert!(outcome.held.is_empty(), "held: {:?}", outcome.held);
    for candidate in &ghosts {
        assert!(
            candidate.path.exists(),
            "{} was swept",
            candidate.display_name
        );
    }

    // The same leftovers are still a person's to move, once they have seen
    // what Scuttle has to say about them.
    let ids: Vec<String> = ghosts
        .iter()
        .filter(|c| c.risk != scuttle_core::model::Risk::Protected)
        .map(|c| c.id.clone())
        .collect();
    let refused = state.quarantine_many(&ids, &[]).expect("selection");
    assert!(refused.held.is_empty());
    assert!(refused
        .refused
        .iter()
        .all(|r| r.code == "needs_acknowledgement"));
    let chosen = state
        .quarantine_many(&ids, &CautionKind::ALL)
        .expect("selection");
    assert_eq!(
        chosen.held.len(),
        ids.len(),
        "refused: {:?}",
        chosen.refused
    );
}

#[test]
fn installers_scuttle_is_sure_of_are_swept_and_nothing_else() {
    let world = old_installers();
    let (state, options) = state_for(&world);
    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");
    let before = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings");
    let suggested: Vec<_> = before
        .iter()
        .filter(|c| c.assessment.eligibility == Eligibility::Suggested)
        .collect();
    assert!(!suggested.is_empty(), "old installers for installed apps");
    let outcome = state.quarantine_all_confident().expect("sweep");
    assert_eq!(outcome.held.len(), suggested.len(), "{:?}", outcome.refused);
    for c in before
        .iter()
        .filter(|c| c.assessment.eligibility != Eligibility::Suggested)
    {
        assert!(
            c.path.exists(),
            "{} moved without being suggested",
            c.display_name
        );
    }
}

/// The incident, end to end, through every detector: a Discord-shaped
/// installation beneath a Windows-like `AppData/Local`, installed, closed,
/// with an old version folder beside the current one — and a standalone
/// installer in Downloads. Only the installer is ever offered, and the sweep
/// cannot reach the application however it is asked.
#[test]
fn a_discord_like_install_survives_a_rummage_and_a_sweep() {
    let mut world = World::new();
    let base = "AppData/Local/Discord";
    world.file(&format!("{base}/Update.exe"), 200, 2 * MB);
    world.file(&format!("{base}/app-1.0.9001/Discord.exe"), 3, 2 * MB);
    world.file(&format!("{base}/app-1.0.9001/ffmpeg.dll"), 3, MB);
    world.file(&format!("{base}/app-1.0.9001/resources/app.asar"), 3, MB);
    world.file(&format!("{base}/app-1.0.9000/Discord.exe"), 120, 2 * MB);
    world.file(&format!("{base}/app-1.0.9000/ffmpeg.dll"), 120, MB);
    world.file(
        &format!("{base}/packages/Discord-1.0.9001-full.nupkg"),
        3,
        2 * MB,
    );
    world.file(&format!("{base}/packages/DiscordSetup.exe"), 3, 2 * MB);
    for n in 0..4 {
        world.file(
            &format!("AppData/Roaming/discord/Cache/Cache_Data/f_{n}"),
            90,
            6 * MB,
        );
    }
    world.file("Downloads/DiscordSetup.exe", 150, 2 * MB);
    world.platform = world
        .platform
        .clone()
        .with_app("Discord", None)
        .with_process("finder")
        .with_data_root("AppData/Local")
        .with_data_root("AppData/Roaming")
        .with_cache_rule(scuttle_core::platform::caches::CacheRule {
            owner: "Discord",
            label: "Discord cache",
            path: world.path("AppData/Roaming/discord/Cache"),
            safety: scuttle_core::platform::caches::CacheSafety::RegeneratesWhenClosed,
            owner_process: Some("discord"),
            developer_only: false,
            settle_secs: 0,
        });

    let (state, options) = state_for(&world);
    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let install = world.path(base);
    let findings = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings");
    for c in &findings {
        let inside = c.path.starts_with(&install) || install.starts_with(&c.path);
        if inside {
            assert_eq!(
                c.assessment.eligibility,
                Eligibility::ExplicitOnly,
                "{} ({:?}) touches the install and was offered as {:?}",
                c.path.display(),
                c.category,
                c.assessment.eligibility
            );
            assert_ne!(
                c.category,
                Category::Installers,
                "never 'just an installer'"
            );
        }
    }
    let downloaded = findings
        .iter()
        .find(|c| c.path.ends_with("Downloads/DiscordSetup.exe"))
        .expect("the real installer is still found");
    assert_eq!(downloaded.category, Category::Installers);
    // A cache is found as the cache, never as the application around it.
    if let Some(cache) = findings.iter().find(|c| c.category == Category::Caches) {
        assert!(
            cache.path.ends_with("discord/Cache"),
            "{}",
            cache.path.display()
        );
    }

    state.quarantine_all_confident().expect("sweep");
    for rel in [
        "Update.exe",
        "app-1.0.9001/Discord.exe",
        "app-1.0.9000/Discord.exe",
        "packages/DiscordSetup.exe",
        "packages/Discord-1.0.9001-full.nupkg",
    ] {
        assert!(install.join(rel).exists(), "{rel} was moved by a sweep");
    }

    // A hand-built batch of everything cannot reach it either.
    let every: Vec<String> = findings.iter().map(|c| c.id.clone()).collect();
    state
        .quarantine_many(&every, &CautionKind::ALL)
        .expect("batch");
    assert!(install.join("Update.exe").exists());
    assert!(install.join("app-1.0.9001/Discord.exe").exists());
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

#[test]
fn a_hand_picked_selection_may_include_what_scuttle_would_not_sweep() {
    // The line between an opinion and a prohibition.
    //
    // `quarantine_confident` decides on the user's behalf, so it may only ever
    // touch findings rated Quarantine. `quarantine_many` acts on boxes a
    // person ticked one at a time with the size and risk of each in front of
    // them, so Review and InspectOnly are theirs to choose.
    //
    // Refusing them here instead made the largest piles most people have —
    // screenshots, heavy strays, where Scuttle vouches for nothing — into
    // things that could not be dealt with at all, which is not caution.
    let world = abandoned_game();
    let (state, options) = state_for(&world);
    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let found = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings");

    let hesitant: Vec<_> = found
        .iter()
        .filter(|c| c.recommended_action != RecommendedAction::Quarantine)
        .filter(|c| c.risk != scuttle_core::model::Risk::Protected)
        .cloned()
        .collect();
    assert!(
        !hesitant.is_empty(),
        "fixture should hold something Scuttle would not sweep on its own"
    );

    let ids: Vec<String> = hesitant.iter().map(|c| c.id.clone()).collect();
    let outcome = state
        .quarantine_many(&ids, &CautionKind::ALL)
        .expect("hand-picked selection");

    // What must not appear is a refusal on the grounds of the verdict itself.
    // Some of this fixture's findings nest inside each other, so once the
    // outer directory has moved the ones underneath it are honestly reported
    // as gone — that is the staleness check doing its job, not the gate
    // second-guessing the user.
    for refusal in &outcome.refused {
        assert_eq!(
            refusal.code, "stale",
            "{} was refused for its verdict, not for the state of the disk: {}",
            refusal.display_name, refusal.reason
        );
    }
    assert!(
        !outcome.held.is_empty(),
        "nothing a person picked by hand was moved; refused: {:?}",
        outcome.refused
    );
    for candidate in &hesitant {
        assert!(
            !candidate.path.exists(),
            "{} was picked by hand and should be gone from where it was",
            candidate.display_name
        );
    }
}

#[test]
fn a_hand_picked_selection_still_cannot_touch_anything_protected() {
    // The actual prohibition, and the reason the verdict check above can go:
    // protection is enforced by the safety gate against the live filesystem,
    // for every item, and no selection can talk its way past it.
    let world = dangerous_paths();
    let (state, options) = state_for(&world);
    let scan_id = state.start_scan(&options).expect("start");
    state
        .run_scan_with(&scan_id, options, &SilentObserver)
        .expect("scan");

    let found = state
        .store()
        .candidates_for_scan(&scan_id)
        .expect("findings");

    // Ask for everything the scan turned up, protected or not.
    let ids: Vec<String> = found.iter().map(|c| c.id.clone()).collect();
    let outcome = state
        .quarantine_many(&ids, &CautionKind::ALL)
        .expect("selection");

    for candidate in found
        .iter()
        .filter(|c| c.risk == scuttle_core::model::Risk::Protected)
    {
        assert!(
            candidate.path.exists(),
            "{} is protected and must not move for any selection",
            candidate.display_name
        );
        assert!(
            outcome
                .refused
                .iter()
                .any(|r| r.display_name == candidate.display_name),
            "{} must be reported as refused, not silently skipped",
            candidate.display_name
        );
    }
}

/// The whole path for a shared folder, as the bug report describes it: a
/// cache that is being written to between the scan and the click.
#[test]
fn a_live_cache_is_cleaned_from_its_reviewed_files_and_its_folder_stays() {
    use scuttle_core::platform::caches::{CacheRule, CacheSafety};
    use scuttle_core::storage::SnapshotState;

    let mut world = World::new();
    let cache = world.path("Library/Caches/owner");
    world.file("Library/Caches/owner/big.bin", 40, 60 * MB);
    for n in 0..20 {
        world.file(&format!("Library/Caches/owner/shard-{n}.bin"), 40, 1024);
    }
    world.platform = world.platform.clone().with_cache_rule(CacheRule {
        owner: "Owner",
        label: "Owner cache",
        path: cache.clone(),
        safety: CacheSafety::Regenerates,
        owner_process: None,
        developer_only: false,
        settle_secs: 0,
    });
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
        .find(|c| c.category == Category::Caches)
        .expect("the cache is found");

    // A scan records what it reviewed, and the number shown is that set.
    let info = state.store().snapshot_info(&candidate.id).unwrap();
    assert_eq!(info.state, SnapshotState::Complete);
    assert_eq!(info.files, 21);
    assert_eq!(candidate.size, info.bytes);

    // The program that owns the cache keeps writing to it.
    world.file("Library/Caches/owner/written-after-the-scan.bin", 0, 2048);
    std::fs::remove_file(cache.join("shard-0.bin")).unwrap();

    let record = state.hold(&candidate).expect("the live cache is cleaned");

    assert_eq!(
        record.item_count, 20,
        "every reviewed file that still exists"
    );
    assert!(cache.is_dir(), "the shared folder is never what moves");
    assert!(
        cache.join("written-after-the-scan.bin").exists(),
        "a file that arrived after the review is not touched"
    );
    assert!(!cache.join("big.bin").exists());
    assert_eq!(state.store().held_quarantine().unwrap().len(), 1);

    let outcome = state
        .quarantine()
        .unwrap()
        .restore(&record.id, 0)
        .expect("restore");
    assert_eq!(outcome.restored, 20);
    assert!(cache.join("big.bin").exists());
    assert!(cache.join("written-after-the-scan.bin").exists());
}
