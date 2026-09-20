//! End-to-end tests for the destructive half of Scuttle.
//!
//! Scan, quarantine, restore, remove. Every test here answers a version of the
//! same question: *what happens if the detector was wrong?* The answers must
//! all be "nothing irreversible, and the user can see why".

mod fixtures;

use std::sync::Arc;

use fixtures::*;
use scuttle_core::detectors;
use scuttle_core::model::{CleanupCandidate, RecommendedAction};
use scuttle_core::platform::PlatformService;
use scuttle_core::quarantine::Quarantine;
use scuttle_core::safety::{ActionContext, ProtectedPaths};
use scuttle_core::scanning::{self, SilentObserver};
use scuttle_core::storage::{QuarantineStatus, Store};

const NOW: i64 = 1_700_000_000;

/// The pieces a real action needs, wired together the way the app wires them.
struct Bench {
    world: World,
    store: Arc<Store>,
    quarantine: Quarantine,
    protected: ProtectedPaths,
    roots: Vec<std::path::PathBuf>,
    candidates: Vec<CleanupCandidate>,
}

fn bench(world: World) -> Bench {
    let ctx = world.context();
    let detectors = detectors::default_set(&ctx.options);
    let outcome = scanning::run("scan", &ctx, detectors, &SilentObserver);

    let store = Arc::new(Store::in_memory().expect("store"));
    store
        .begin_scan("scan", &ctx.options, NOW)
        .expect("begin scan");
    store
        .save_candidates("scan", &outcome.candidates)
        .expect("save");

    let quarantine = Quarantine::new(world.platform.quarantine_root(), Arc::clone(&store), 14);
    let mut protected = ProtectedPaths::for_home(&world.home);
    protected.also_protect("Scuttle's own files", world.platform.data_dir());

    Bench {
        roots: world.options.roots.clone(),
        candidates: outcome.candidates,
        world,
        store,
        quarantine,
        protected,
    }
}

impl Bench {
    fn ctx(&self) -> ActionContext<'_> {
        ActionContext {
            protected: &self.protected,
            allowed_roots: &self.roots,
            // Test harnesses take the strict bidding, so every existing
            // assertion keeps meaning what it meant.
            bidding: scuttle_core::safety::Bidding::Scuttle,
        }
    }

    fn find(&self, name: &str) -> &CleanupCandidate {
        self.candidates
            .iter()
            .find(|c| c.display_name == name)
            .unwrap_or_else(|| {
                panic!(
                    "no finding called {name}; got {:?}",
                    self.candidates
                        .iter()
                        .map(|c| &c.display_name)
                        .collect::<Vec<_>>()
                )
            })
    }
}

// ---------------------------------------------------------------------------

#[test]
fn a_found_installer_can_be_quarantined_and_put_back_unchanged() {
    let bench = bench(old_installers());
    let candidate = bench.find("Google Chrome.dmg").clone();
    let original = candidate.path.clone();
    let contents_before = std::fs::read(&original).expect("read before");

    let record = bench
        .quarantine
        .hold(&candidate, &bench.ctx(), NOW)
        .expect("hold");

    assert!(!original.exists(), "the original should have moved");
    assert!(record.stored_path.exists());

    let outcome = bench
        .quarantine
        .restore(&record.id, NOW + 60)
        .expect("restore");
    assert_eq!(outcome.path, original);
    assert!(!outcome.renamed);
    assert_eq!(
        std::fs::read(&original).expect("read after"),
        contents_before,
        "a round trip through the drawer must not change a single byte"
    );
}

#[test]
fn a_whole_ghost_directory_survives_a_round_trip() {
    let bench = bench(abandoned_game());
    let candidate = bench.find("Cyberpunk 2077").clone();
    let original = candidate.path.clone();

    let before: Vec<_> = walk_names(&original);
    assert!(before.len() > 5, "fixture should be a real tree");

    let record = bench
        .quarantine
        .hold(&candidate, &bench.ctx(), NOW)
        .expect("hold");
    assert!(!original.exists());

    bench
        .quarantine
        .restore(&record.id, NOW + 60)
        .expect("restore");
    assert_eq!(
        walk_names(&original),
        before,
        "the tree came back different"
    );
}

#[test]
fn nothing_scuttle_refuses_to_recommend_can_be_quarantined_anyway() {
    // The frontend could ask; the backend must still say no.
    let bench = bench(abandoned_game());
    let refused = bench
        .candidates
        .iter()
        .find(|c| c.recommended_action == RecommendedAction::InspectOnly)
        .expect("the fixture has a save-data folder in it")
        .clone();
    let path = refused.path.clone();

    let error = bench
        .quarantine
        .hold(&refused, &bench.ctx(), NOW)
        .expect_err("should be refused");
    assert_eq!(error.code(), "refused");
    assert!(path.exists(), "a refused action must not move anything");
}

#[test]
fn a_file_that_changed_since_the_scan_is_refused_as_stale() {
    let bench = bench(old_installers());
    let candidate = bench.find("Google Chrome.dmg").clone();

    // Someone opened it, or a download resumed, or the disk lied to us.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&candidate.path, b"different contents entirely").expect("write");

    let error = bench
        .quarantine
        .hold(&candidate, &bench.ctx(), NOW)
        .expect_err("should be stale");
    assert_eq!(error.code(), "stale");
    assert!(candidate.path.exists());
}

#[test]
fn restoring_onto_an_occupied_path_never_overwrites() {
    let bench = bench(old_installers());
    let candidate = bench.find("Google Chrome.dmg").clone();
    let original = candidate.path.clone();

    let record = bench
        .quarantine
        .hold(&candidate, &bench.ctx(), NOW)
        .expect("hold");
    std::fs::write(&original, b"something new and important").expect("write");

    let outcome = bench
        .quarantine
        .restore(&record.id, NOW + 60)
        .expect("restore");
    assert!(outcome.renamed);
    assert_ne!(outcome.path, original);
    assert_eq!(
        std::fs::read(&original).expect("read"),
        b"something new and important",
        "the newer file must survive untouched"
    );
}

#[test]
fn permanent_removal_only_happens_when_it_is_asked_for() {
    let bench = bench(old_installers());
    let candidate = bench.find("Google Chrome.dmg").clone();
    let record = bench
        .quarantine
        .hold(&candidate, &bench.ctx(), NOW)
        .expect("hold");

    // Well inside the retention window: the sweep must leave it alone.
    assert_eq!(
        bench.quarantine.sweep_expired(NOW + 86_400).expect("sweep"),
        0
    );
    assert_eq!(
        bench
            .store
            .quarantine_record(&record.id)
            .expect("record")
            .status,
        QuarantineStatus::Held
    );

    bench
        .quarantine
        .purge(&record.id, NOW + 120)
        .expect("purge");
    assert!(!record.stored_path.exists());
    assert_eq!(
        bench
            .store
            .quarantine_record(&record.id)
            .expect("record")
            .status,
        QuarantineStatus::Removed
    );
}

#[test]
fn the_sweep_clears_the_drawer_only_after_the_window_closes() {
    let bench = bench(old_installers());
    let candidate = bench.find("Google Chrome.dmg").clone();
    bench
        .quarantine
        .hold(&candidate, &bench.ctx(), NOW)
        .expect("hold");

    assert_eq!(
        bench
            .quarantine
            .sweep_expired(NOW + 13 * 86_400)
            .expect("sweep"),
        0
    );
    assert_eq!(
        bench
            .quarantine
            .sweep_expired(NOW + 15 * 86_400)
            .expect("sweep"),
        1
    );
    assert!(bench.store.held_quarantine().expect("held").is_empty());
}

#[test]
fn quarantined_items_never_turn_up_in_the_next_rummage() {
    // The drawer lives inside a directory Scuttle scans. If it were visible,
    // Scuttle would offer to quarantine things it had already quarantined.
    let bench = bench(old_installers());
    let candidate = bench.find("Google Chrome.dmg").clone();
    bench
        .quarantine
        .hold(&candidate, &bench.ctx(), NOW)
        .expect("hold");

    let ctx = bench.world.context();
    let detectors = detectors::default_set(&ctx.options);
    let again = scanning::run("scan2", &ctx, detectors, &SilentObserver);

    assert!(
        !again
            .candidates
            .iter()
            .any(|c| c.path.to_string_lossy().contains(".scuttle")),
        "Scuttle found its own drawer"
    );
}

#[test]
fn the_evidence_shown_at_the_time_is_kept_with_the_record() {
    let bench = bench(old_installers());
    let candidate = bench.find("Google Chrome.dmg").clone();
    let record = bench
        .quarantine
        .hold(&candidate, &bench.ctx(), NOW)
        .expect("hold");

    assert_eq!(record.evidence.len(), candidate.evidence.len());
    // ...and it is readable without the database, from the drawer itself.
    let manifest = record
        .stored_path
        .parent()
        .expect("cell")
        .join("scuttle-manifest.json");
    let text = std::fs::read_to_string(manifest).expect("manifest");
    assert!(text.contains("Google Chrome.dmg"));
    assert!(text.contains("already installed"));
}

fn walk_names(root: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = walkdir_names(root);
    names.sort();
    names
}

fn walkdir_names(root: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        if let Some(name) = name {
            out.push(name);
        }
        if path.is_dir() {
            out.extend(walkdir_names(&path));
        }
    }
    out
}
