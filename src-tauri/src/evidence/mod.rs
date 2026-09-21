//! Evidence, and the arithmetic that turns it into a verdict.
//!
//! Two rules govern this module:
//!
//! 1. Every user-visible sentence about *why* a finding exists is generated
//!    here, from structured data. Detection logic never hides inside a UI
//!    string.
//! 2. Confidence and risk are computed from the same evidence list the user
//!    can read. There is no second, secret score.

use serde::{Deserialize, Serialize};

use crate::model::{human_bytes, Confidence, RecommendedAction, Risk};

/// The specific observations Scuttle is allowed to make. Adding a detector
/// usually means adding a variant here rather than inventing prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceKind {
    /// The owning application could not be found on this machine.
    ApplicationNotInstalled { app: String },
    /// The owning application *is* installed — a strong argument to leave
    /// its data alone. Used for application data, never for installers.
    ApplicationInstalled { app: String },
    /// The thing this installer installs is already installed. The same
    /// observation as [`EvidenceKind::ApplicationInstalled`], but pointing the
    /// opposite way: here it argues *for* cleanup, and carries no risk floor,
    /// because throwing away a used installer costs a download at worst.
    InstalledAppSupersedes { app: String },
    /// A game library (Steam, Epic, ...) has no manifest for this title.
    GameNotInLibrary { app: String, library: String },
    /// Nothing has read or written this in a while.
    UntouchedFor { days: u32 },
    /// Something touched this recently. Negative signal.
    RecentlyTouched { days: u32 },
    /// The path matches a cache location Scuttle has a specific rule for.
    KnownCachePath { owner: String },
    /// The directory name or bundle id matches a known application id.
    MatchesApplicationId { id: String },
    /// Content that looks authored rather than generated. Strong negative.
    UserContentDetected { reason: String },
    /// Save games, profiles or mods live in here. Strong negative.
    SaveDataDetected { reason: String },
    /// Byte-identical copies exist elsewhere.
    ExactDuplicate { copies: u32 },
    /// Visually near-identical images in the same burst.
    NearDuplicateImage { group_size: u32 },
    /// A newer installer for the same product is sitting next to it.
    NewerInstallerPresent { newer: String },
    /// The file extension is an installer format on this platform.
    InstallerFormat { ext: String },
    /// Filename matches a platform screenshot convention.
    ScreenshotNamePattern { pattern: String },
    /// The file sits in a folder the screenshot tool actually writes to.
    InKnownScreenshotFolder { folder: String },
    /// Dimensions match a known display size.
    ScreenshotDimensions { width: u32, height: u32 },
    /// Big enough to be worth mentioning on its own.
    LargeSize { bytes: u64 },
    /// The owning tool recreates this on next use.
    RegeneratedOnDemand { owner: String },
    /// Contents look machine-generated (shader caches, blobs, logs).
    GeneratedContent { reason: String },
    /// A build directory with a project manifest beside it.
    BuildOutputOfProject { manifest: String },
    /// The project was worked on recently. Negative for developer debris.
    ProjectRecentlyActive { days: u32 },
    /// A version control directory is at or above this path.
    InsideRepository,
    /// A rule in the protected-path table matched.
    ProtectedByRule { rule: String },
    /// Scuttle could not work out what this is.
    Unclassified { reason: String },
    /// A running process holds this open, or its owner is running.
    InUse { by: String },
    /// Nothing appears to reference it.
    NoProcessUsingIt,
    /// Part of an installed or portable application, recognised by its
    /// structure. Moving it breaks the application; it is never a leftover.
    PartOfInstalledApplication { app: String, how: String },
    /// Inside a folder an application keeps for itself (`%LOCALAPPDATA%`,
    /// `~/Library/Application Support`). Not proof of anything either way,
    /// but a reason not to presume it is the user's to discard.
    InsideApplicationData,
    /// The operating system recorded that this arrived from the internet.
    DownloadedFromInternet,
    /// The file name reads like an installer ("Setup", "Installer").
    NamedLikeInstaller,
}

/// A single observation, with its arithmetic already resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub kind: EvidenceKind,
    /// The sentence the UI shows. Generated here.
    pub summary: String,
    /// Positive argues for cleanup, negative argues against.
    pub weight: i32,
    /// True when this evidence argues *against* acting. The UI marks these
    /// differently instead of hiding them.
    pub negative: bool,
    /// Evidence can raise risk but never lower it.
    pub risk_floor: Option<Risk>,
}

impl Evidence {
    pub fn new(kind: EvidenceKind) -> Evidence {
        let (summary, weight, risk_floor) = describe(&kind);
        Evidence {
            kind,
            summary,
            weight,
            negative: weight < 0,
            risk_floor,
        }
    }
}

/// The single place where an observation becomes a sentence, a weight and a
/// risk floor. Keeping this exhaustive means a new variant cannot ship without
/// someone deciding what it is worth.
fn describe(kind: &EvidenceKind) -> (String, i32, Option<Risk>) {
    use EvidenceKind::*;
    match kind {
        // Worded as what was checked, not as a fact about the world: missing
        // uninstall metadata is not proof an application is gone.
        ApplicationNotInstalled { app } => (
            format!("Scuttle found no installed application called {app}"),
            35,
            None,
        ),
        ApplicationInstalled { app } => (
            format!("{app} is still installed"),
            -60,
            Some(Risk::Moderate),
        ),
        // One signal, not a verdict: it takes a second one (a newer copy, a
        // download record, long disuse) to make an installer a confident find.
        InstalledAppSupersedes { app } => (format!("{app} is already installed"), 30, None),
        GameNotInLibrary { app, library } => {
            (format!("{library} has no installation of {app}"), 35, None)
        }
        UntouchedFor { days } => (
            format!("Untouched for {days} days"),
            // Age alone is weak evidence and stops counting for much past a
            // year — a file is not junk because it is old.
            (*days as i32 / 20).clamp(0, 25),
            None,
        ),
        RecentlyTouched { days } => (
            if *days <= 1 {
                "Touched today".to_string()
            } else {
                format!("Touched {days} days ago")
            },
            -45,
            Some(Risk::Moderate),
        ),
        KnownCachePath { owner } => (format!("A known cache location for {owner}"), 55, None),
        MatchesApplicationId { id } => (format!("Named for the application id {id}"), 25, None),
        UserContentDetected { reason } => (
            format!("May contain files you made — {reason}"),
            -70,
            Some(Risk::High),
        ),
        SaveDataDetected { reason } => (
            format!("Looks like save data — {reason}"),
            -80,
            Some(Risk::High),
        ),
        ExactDuplicate { copies } => (
            if *copies == 1 {
                "One other byte-identical copy exists".to_string()
            } else {
                format!("{copies} other byte-identical copies exist")
            },
            50,
            None,
        ),
        NearDuplicateImage { group_size } => (
            format!("{group_size} images here look nearly identical"),
            30,
            Some(Risk::Moderate),
        ),
        NewerInstallerPresent { newer } => (format!("A newer copy exists: {newer}"), 35, None),
        InstallerFormat { ext } => (format!("A .{ext} installer package"), 30, None),
        ScreenshotNamePattern { pattern } => (
            format!("Named like a screenshot ({pattern})"),
            25,
            Some(Risk::Moderate),
        ),
        InKnownScreenshotFolder { folder } => (
            format!("Sitting in {folder}, where captures land"),
            20,
            None,
        ),
        ScreenshotDimensions { width, height } => (
            format!("{width}x{height} — matches a display, not a camera"),
            10,
            None,
        ),
        LargeSize { bytes } => (format!("{} on disk", human_bytes(*bytes)), 10, None),
        RegeneratedOnDemand { owner } => {
            (format!("{owner} rebuilds this when it needs it"), 30, None)
        }
        GeneratedContent { reason } => (format!("Contents look generated — {reason}"), 25, None),
        BuildOutputOfProject { manifest } => (
            format!("Build output for the project described by {manifest}"),
            30,
            Some(Risk::Moderate),
        ),
        ProjectRecentlyActive { days } => (
            format!("The project was worked on {days} days ago"),
            -55,
            Some(Risk::Moderate),
        ),
        InsideRepository => (
            "Sits inside a version-controlled repository".to_string(),
            -30,
            Some(Risk::Moderate),
        ),
        ProtectedByRule { rule } => (format!("Protected: {rule}"), -1000, Some(Risk::Protected)),
        Unclassified { reason } => (
            format!("Scuttle is not sure what this is — {reason}"),
            0,
            Some(Risk::Moderate),
        ),
        InUse { by } => (format!("In use by {by}"), -70, Some(Risk::High)),
        // Shown, but worth nothing: an application that is closed right now
        // is still installed, and "not running" was how Scuttle once argued
        // itself into moving one.
        NoProcessUsingIt => (
            "Nothing appears to be using it right now".to_string(),
            0,
            None,
        ),
        PartOfInstalledApplication { app, how } => (
            format!("Part of {app} — {how}. Moving it would likely stop {app} working"),
            -200,
            Some(Risk::High),
        ),
        InsideApplicationData => (
            "Inside a folder applications keep their own data in".to_string(),
            -10,
            Some(Risk::Moderate),
        ),
        DownloadedFromInternet => (
            "Your computer recorded that this was downloaded".to_string(),
            15,
            None,
        ),
        NamedLikeInstaller => ("Named like an installer".to_string(), 10, None),
    }
}

/// The conclusion drawn from a pile of evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub confidence: Confidence,
    pub risk: Risk,
    pub action: RecommendedAction,
    /// Kept for debugging and the dry-run report; never shown as a number.
    pub score: i32,
}

/// Weigh the evidence.
///
/// `base_risk` is the detector's own starting position — a duplicate photo
/// starts riskier than a shader cache no matter what the evidence says.
pub fn weigh(evidence: &[Evidence], base_risk: Risk) -> Verdict {
    let score: i32 = evidence.iter().map(|e| e.weight).sum();

    let mut risk = base_risk;
    for e in evidence {
        if let Some(floor) = e.risk_floor {
            risk.raise_to(floor);
        }
    }

    let confidence = Confidence::from_score(score);

    // The action ladder. Note that high confidence never by itself unlocks
    // quarantine: being *sure* about something dangerous is not permission.
    let action = match risk {
        Risk::Protected => RecommendedAction::InspectOnly,
        Risk::High => RecommendedAction::InspectOnly,
        Risk::Moderate => {
            if confidence == Confidence::High {
                RecommendedAction::Review
            } else {
                RecommendedAction::InspectOnly
            }
        }
        Risk::Low => match confidence {
            Confidence::High => RecommendedAction::Quarantine,
            Confidence::Medium => RecommendedAction::Review,
            Confidence::Low => RecommendedAction::InspectOnly,
        },
    };

    Verdict {
        confidence,
        risk,
        action,
        score,
    }
}

/// Convenience for detectors: build an evidence list without ceremony.
pub fn ev(kind: EvidenceKind) -> Evidence {
    Evidence::new(kind)
}

#[cfg(test)]
mod tests {
    use super::EvidenceKind::*;
    use super::*;

    #[test]
    fn a_protected_rule_overwhelms_everything_else() {
        let e = vec![
            ev(ApplicationNotInstalled {
                app: "Whatever".into(),
            }),
            ev(KnownCachePath {
                owner: "Whatever".into(),
            }),
            ev(UntouchedFor { days: 900 }),
            ev(ProtectedByRule {
                rule: "SSH keys".into(),
            }),
        ];
        let v = weigh(&e, Risk::Low);
        assert_eq!(v.risk, Risk::Protected);
        assert_eq!(v.action, RecommendedAction::InspectOnly);
        assert_eq!(v.confidence, Confidence::Low);
    }

    #[test]
    fn certainty_about_a_risky_thing_is_not_permission() {
        // Highly confident this is an old duplicate photo. Still not ours to
        // suggest deleting.
        let e = vec![
            ev(ExactDuplicate { copies: 4 }),
            ev(UntouchedFor { days: 700 }),
            ev(UserContentDetected {
                reason: "in Pictures".into(),
            }),
            ev(LargeSize { bytes: 900_000_000 }),
        ];
        let v = weigh(&e, Risk::Low);
        assert_eq!(v.risk, Risk::High);
        assert_eq!(v.action, RecommendedAction::InspectOnly);
    }

    #[test]
    fn a_known_cache_of_a_dead_app_is_quarantinable() {
        let e = vec![
            ev(ApplicationNotInstalled {
                app: "Old Game".into(),
            }),
            ev(KnownCachePath {
                owner: "Old Game".into(),
            }),
            ev(GeneratedContent {
                reason: "shader blobs".into(),
            }),
            ev(NoProcessUsingIt),
        ];
        let v = weigh(&e, Risk::Low);
        assert_eq!(v.confidence, Confidence::High);
        assert_eq!(v.risk, Risk::Low);
        assert_eq!(v.action, RecommendedAction::Quarantine);
    }

    #[test]
    fn the_same_observation_can_point_both_ways() {
        // Figma being installed protects Figma's cache...
        let protect = weigh(
            &[
                ev(KnownCachePath {
                    owner: "Figma".into(),
                }),
                ev(ApplicationInstalled {
                    app: "Figma".into(),
                }),
            ],
            Risk::Low,
        );
        assert_ne!(protect.action, RecommendedAction::Quarantine);

        // ...but condemns the Figma installer sitting in Downloads.
        let condemn = weigh(
            &[
                ev(InstallerFormat { ext: "dmg".into() }),
                ev(InstalledAppSupersedes {
                    app: "Figma".into(),
                }),
                ev(UntouchedFor { days: 200 }),
            ],
            Risk::Low,
        );
        assert_eq!(condemn.confidence, Confidence::High);
        assert_eq!(condemn.risk, Risk::Low);
        assert_eq!(condemn.action, RecommendedAction::Quarantine);
    }

    #[test]
    fn an_installed_app_protects_its_own_data() {
        let e = vec![
            ev(KnownCachePath {
                owner: "Figma".into(),
            }),
            ev(ApplicationInstalled {
                app: "Figma".into(),
            }),
            ev(UntouchedFor { days: 200 }),
        ];
        let v = weigh(&e, Risk::Low);
        assert!(v.score < 35, "score was {}", v.score);
        assert_eq!(v.confidence, Confidence::Low);
        assert_ne!(v.action, RecommendedAction::Quarantine);
    }

    #[test]
    fn age_alone_never_reaches_high_confidence() {
        for days in [200u32, 400, 900, 5000] {
            let v = weigh(&[ev(UntouchedFor { days })], Risk::Low);
            assert_ne!(
                v.confidence,
                Confidence::High,
                "{days} days of age should not be enough on its own"
            );
        }
    }

    #[test]
    fn save_data_always_forces_high_risk() {
        let e = vec![
            ev(GameNotInLibrary {
                app: "X".into(),
                library: "Steam".into(),
            }),
            ev(ApplicationNotInstalled { app: "X".into() }),
            ev(UntouchedFor { days: 500 }),
            ev(SaveDataDetected {
                reason: "contains profile.sav".into(),
            }),
        ];
        let v = weigh(&e, Risk::Low);
        assert_eq!(v.risk, Risk::High);
        assert_eq!(v.action, RecommendedAction::InspectOnly);
    }

    #[test]
    fn every_evidence_kind_produces_a_non_empty_sentence() {
        let all = vec![
            ApplicationNotInstalled { app: "A".into() },
            ApplicationInstalled { app: "A".into() },
            InstalledAppSupersedes { app: "A".into() },
            GameNotInLibrary {
                app: "A".into(),
                library: "Steam".into(),
            },
            UntouchedFor { days: 3 },
            RecentlyTouched { days: 1 },
            RecentlyTouched { days: 4 },
            KnownCachePath { owner: "A".into() },
            MatchesApplicationId { id: "com.a".into() },
            UserContentDetected { reason: "r".into() },
            SaveDataDetected { reason: "r".into() },
            ExactDuplicate { copies: 1 },
            ExactDuplicate { copies: 5 },
            NearDuplicateImage { group_size: 3 },
            NewerInstallerPresent { newer: "n".into() },
            InstallerFormat { ext: "dmg".into() },
            ScreenshotNamePattern {
                pattern: "p".into(),
            },
            InKnownScreenshotFolder {
                folder: "Desktop".into(),
            },
            ScreenshotDimensions {
                width: 1,
                height: 2,
            },
            LargeSize { bytes: 5 },
            RegeneratedOnDemand { owner: "A".into() },
            GeneratedContent { reason: "r".into() },
            BuildOutputOfProject {
                manifest: "Cargo.toml".into(),
            },
            ProjectRecentlyActive { days: 2 },
            InsideRepository,
            ProtectedByRule { rule: "r".into() },
            Unclassified { reason: "r".into() },
            InUse { by: "p".into() },
            NoProcessUsingIt,
        ];
        for kind in all {
            let e = Evidence::new(kind.clone());
            assert!(!e.summary.trim().is_empty(), "{kind:?} had no sentence");
            assert!(
                e.summary.chars().next().unwrap().is_uppercase()
                    || e.summary.starts_with(|c: char| c.is_numeric()),
                "{kind:?} sentence should read like a sentence: {}",
                e.summary
            );
        }
    }

    #[test]
    fn negative_flag_tracks_the_sign_of_the_weight() {
        assert!(ev(ApplicationInstalled { app: "A".into() }).negative);
        assert!(ev(InUse { by: "p".into() }).negative);
        assert!(!ev(KnownCachePath { owner: "A".into() }).negative);
    }
}
