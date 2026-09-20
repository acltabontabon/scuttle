//! End-to-end tests: a real filesystem, a real scan, a real safety layer.
//!
//! These run the whole pipeline — traversal, every detector, the evidence
//! arithmetic, the safety net — against the fixture trees in `fixtures.rs`.
//! Unit tests prove each piece behaves; these prove the pieces still behave
//! when assembled.

mod fixtures;

use fixtures::*;
use scuttle_core::detectors;
use scuttle_core::model::{Category, CleanupCandidate, RecommendedAction, Risk};
use scuttle_core::scanning::{self, ScanOutcome, SilentObserver};

fn rummage(world: &World) -> ScanOutcome {
    let ctx = world.context();
    let detectors = detectors::default_set(&ctx.options);
    scanning::run("test-scan", &ctx, detectors, &SilentObserver)
}

fn in_category(outcome: &ScanOutcome, category: Category) -> Vec<&CleanupCandidate> {
    outcome
        .candidates
        .iter()
        .filter(|c| c.category == category)
        .collect()
}

fn named<'a>(outcome: &'a ScanOutcome, name: &str) -> Option<&'a CleanupCandidate> {
    outcome.candidates.iter().find(|c| c.display_name == name)
}

// ---------------------------------------------------------------------------

#[test]
fn a_tidy_machine_produces_a_quiet_scan() {
    // If this ever starts failing, Scuttle has begun inventing problems, which
    // is the single behaviour the whole project is organised against.
    let outcome = rummage(&healthy_system());
    let suggested: Vec<_> = outcome
        .candidates
        .iter()
        .filter(|c| c.recommended_action == RecommendedAction::Quarantine)
        .collect();
    assert!(
        suggested.is_empty(),
        "nothing on a healthy system should be proposed for cleanup, got {:?}",
        suggested
            .iter()
            .map(|c| (&c.display_name, c.category))
            .collect::<Vec<_>>()
    );
}

#[test]
fn nothing_protected_ever_reaches_the_results() {
    let world = dangerous_paths();
    let outcome = rummage(&world);

    for candidate in &outcome.candidates {
        assert_ne!(candidate.risk, Risk::Protected);
        for forbidden in [
            ".ssh",
            ".gnupg",
            ".aws",
            "vault.kdbx",
            "server.pem",
            "Dropbox",
            ".git",
            "Keychains",
        ] {
            assert!(
                !candidate.path.to_string_lossy().contains(forbidden),
                "{} surfaced a protected path: {}",
                candidate.detector,
                candidate.path.display()
            );
        }
    }
}

#[test]
fn a_protected_tree_is_not_even_walked() {
    let world = dangerous_paths();
    let outcome = rummage(&world);
    // The 40 MB password vault and the 30 MB git object are both well over the
    // heavy threshold. Neither may be counted, let alone reported.
    assert!(
        named(&outcome, "vault.kdbx").is_none(),
        "a password vault must be invisible, not merely unrecommended"
    );
    assert!(named(&outcome, "cdef").is_none());
}

#[test]
fn an_uninstalled_game_is_found_and_its_save_data_is_spared() {
    let world = abandoned_game();
    let outcome = rummage(&world);

    let cyberpunk = named(&outcome, "Cyberpunk 2077").expect("the uninstalled game");
    assert_eq!(cyberpunk.category, Category::Ghosts);
    assert_eq!(cyberpunk.recommended_action, RecommendedAction::Quarantine);
    assert!(cyberpunk
        .evidence
        .iter()
        .any(|e| e.summary.contains("Steam has no installation")));

    // Same situation, but with saved progress in it.
    let rpg = named(&outcome, "Old RPG").expect("the game with saves");
    assert_eq!(rpg.risk, Risk::High);
    assert_eq!(
        rpg.recommended_action,
        RecommendedAction::InspectOnly,
        "a folder with save data in it must never be proposed for cleanup"
    );

    // And the game that is still installed is left entirely alone.
    assert!(named(&outcome, "Hades").is_none());
}

#[test]
fn used_installers_are_found_and_repeats_are_counted() {
    let outcome = rummage(&old_installers());
    let installers = in_category(&outcome, Category::Installers);
    assert!(installers.len() >= 5, "got {}", installers.len());

    let chrome = named(&outcome, "Google Chrome.dmg").expect("the newest Chrome installer");
    assert_eq!(chrome.recommended_action, RecommendedAction::Quarantine);
    assert_eq!(chrome.risk, Risk::Low);
    assert!(chrome
        .evidence
        .iter()
        .any(|e| e.summary.contains("Google Chrome is already installed")));

    // The tool nobody has heard of is not proposed for anything.
    let obscure = named(&outcome, "ObscureTool.dmg").expect("the unknown installer");
    assert_ne!(obscure.recommended_action, RecommendedAction::Quarantine);

    // Plain text files are not installers.
    assert!(named(&outcome, "notes.txt").is_none());
}

#[test]
fn identical_files_are_grouped_and_lookalikes_are_not() {
    let outcome = rummage(&duplicate_files());
    let copies = in_category(&outcome, Category::Copies);
    assert_eq!(copies.len(), 1, "one group, not one finding per file");

    let group = &copies[0];
    assert_eq!(group.group.len(), 3);
    assert_eq!(group.group.iter().filter(|m| m.suggested_keep).count(), 1);

    // The decoy is the same size and shares its head and tail; only the full
    // hash separates it.
    assert!(
        !group.group.iter().any(|m| m.path.ends_with("decoy.pdf")),
        "a same-size lookalike was called a duplicate"
    );
}

#[test]
fn large_files_are_surfaced_but_never_recommended() {
    let outcome = rummage(&developer_project());
    for candidate in in_category(&outcome, Category::HeavyStrays) {
        assert_eq!(
            candidate.recommended_action,
            RecommendedAction::InspectOnly,
            "{} was recommended; large is not the same as disposable",
            candidate.display_name
        );
    }
}

#[test]
fn developer_debris_stays_hidden_until_it_is_asked_for() {
    let mut world = developer_project();
    let quiet = rummage(&world);
    assert!(
        in_category(&quiet, Category::DeveloperDebris).is_empty(),
        "build output is opt-in"
    );

    world.options.include_developer_debris = true;
    let loud = rummage(&world);
    let debris = in_category(&loud, Category::DeveloperDebris);
    assert!(!debris.is_empty(), "asked for, and still nothing turned up");

    // A folder called `build` with no manifest anywhere near it is not debris.
    assert!(
        !debris
            .iter()
            .any(|c| c.path.to_string_lossy().contains("Photos")),
        "a photographer's build folder is not build output"
    );

    // The project someone is actively working on is pointed at, not proposed.
    if let Some(active) = debris
        .iter()
        .find(|c| c.path.to_string_lossy().contains("live"))
    {
        assert_eq!(active.recommended_action, RecommendedAction::InspectOnly);
    }
}

#[test]
fn a_symlink_loop_terminates_and_does_not_escape() {
    let world = symlink_loop();
    // A broken walker would not return from this at all.
    let outcome = rummage(&world);

    for candidate in &outcome.candidates {
        assert!(
            !candidate.path.to_string_lossy().contains("secret.txt"),
            "the scan followed a link out of its own root"
        );
    }
    assert_eq!(
        outcome.summary.hiccups.loops_avoided, 0,
        "links are stepped over, so no loop should ever be entered"
    );
}

#[test]
fn a_cancelled_scan_stops_and_says_so() {
    let world = old_installers();
    let ctx = world.context();
    let detectors = detectors::default_set(&ctx.options);
    // Cancel before it begins: the outcome must be honest rather than empty.
    ctx.request_cancel();
    let outcome = scanning::run("cancelled", &ctx, detectors, &SilentObserver);
    assert!(outcome.summary.cancelled);
}

#[test]
fn every_finding_carries_evidence_and_an_explicable_verdict() {
    for world in [
        abandoned_game(),
        old_installers(),
        duplicate_files(),
        developer_project(),
    ] {
        let outcome = rummage(&world);
        for candidate in &outcome.candidates {
            assert!(
                !candidate.evidence.is_empty(),
                "{} has no reasons behind it",
                candidate.display_name
            );
            for reason in &candidate.evidence {
                assert!(!reason.summary.trim().is_empty());
            }
            // High-risk findings are never proposed, whatever the confidence.
            if candidate.risk >= Risk::High {
                assert_eq!(candidate.recommended_action, RecommendedAction::InspectOnly);
            }
        }
    }
}

#[test]
fn ignored_decisions_are_respected_on_the_next_rummage() {
    let mut world = old_installers();
    let before = rummage(&world);
    assert!(named(&before, "Google Chrome.dmg").is_some());

    world.ignores.paths.push(world.path("Downloads"));
    let after = rummage(&world);
    assert!(
        in_category(&after, Category::Installers).is_empty(),
        "Scuttle brought up something it was told to forget"
    );
}
