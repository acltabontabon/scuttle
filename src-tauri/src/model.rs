//! The vocabulary Scuttle thinks in.
//!
//! Everything the UI renders about a finding originates here. The frontend
//! never decides what a thing *is*, only how it looks.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::evidence::Evidence;

/// The piles Scuttle empties its pockets into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Files belonging to software that is no longer installed.
    Ghosts,
    /// Screenshots, especially the twenty near-identical ones.
    Screenshots,
    /// Installers whose job is finished.
    Installers,
    /// Large files worth a look. Not junk. Just large.
    HeavyStrays,
    /// Byte-identical duplicates.
    Copies,
    /// Caches Scuttle actually knows the owner of.
    Caches,
    /// Build output and package caches. Opt-in.
    DeveloperDebris,
    /// Things Scuttle noticed but cannot confidently name.
    Oddments,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::Ghosts,
        Category::Screenshots,
        Category::Installers,
        Category::HeavyStrays,
        Category::Copies,
        Category::Caches,
        Category::DeveloperDebris,
        Category::Oddments,
    ];

    pub fn slug(&self) -> &'static str {
        match self {
            Category::Ghosts => "ghosts",
            Category::Screenshots => "screenshots",
            Category::Installers => "installers",
            Category::HeavyStrays => "heavy_strays",
            Category::Copies => "copies",
            Category::Caches => "caches",
            Category::DeveloperDebris => "developer_debris",
            Category::Oddments => "oddments",
        }
    }

    pub fn from_slug(s: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.slug() == s)
    }

    /// Display name. Lives in the core so every surface agrees.
    pub fn title(&self) -> &'static str {
        match self {
            Category::Ghosts => "Ghosts",
            Category::Screenshots => "Screenshots",
            Category::Installers => "Installers",
            Category::HeavyStrays => "Heavy strays",
            Category::Copies => "Copies",
            Category::Caches => "Caches",
            Category::DeveloperDebris => "Developer debris",
            Category::Oddments => "Oddments",
        }
    }
}

/// How sure Scuttle is about its *classification*. Deliberately coarse: fake
/// precision ("83.742%") is a tell of software that is guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    /// Internal weighted scoring is fine; it just never reaches the user as a
    /// number.
    pub fn from_score(score: i32) -> Confidence {
        if score >= 70 {
            Confidence::High
        } else if score >= 35 {
            Confidence::Medium
        } else {
            Confidence::Low
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Confidence::Low => "Low",
            Confidence::Medium => "Medium",
            Confidence::High => "High",
        }
    }
}

/// What it would cost the user if Scuttle were wrong. Orthogonal to
/// confidence: Scuttle can be *certain* a 40 GB archive is old and still have
/// no business recommending its removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Regenerated automatically, or provably disposable.
    Low,
    /// Costs a download or a re-export to get back.
    Moderate,
    /// May be irreplaceable user content.
    High,
    /// Scuttle will not act on this at all.
    Protected,
}

impl Risk {
    pub fn label(&self) -> &'static str {
        match self {
            Risk::Low => "Low",
            Risk::Moderate => "Moderate",
            Risk::High => "High",
            Risk::Protected => "Protected",
        }
    }

    pub fn raise_to(&mut self, other: Risk) {
        if other > *self {
            *self = other;
        }
    }
}

/// What Scuttle suggests. Never what Scuttle does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedAction {
    /// Confident and cheap to be wrong about.
    Quarantine,
    /// Worth your eyes. Scuttle has no opinion strong enough to act on.
    Review,
    /// Surfaced for awareness only. Cleanup is not suggested.
    InspectOnly,
}

/// Whether the candidate points at a file or a whole directory. Directories
/// get materially more caution everywhere downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    File,
    Directory,
}

/// One more file in a group finding (duplicates, screenshot bursts). Members
/// are siblings, not children: Scuttle never presumes which copy is the
/// disposable one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMember {
    pub path: PathBuf,
    pub size: u64,
    pub modified_unix: Option<i64>,
    /// Set by the core when it has a defensible opinion about which member is
    /// the obvious keeper (e.g. the newest of an identical set). Advisory.
    pub suggested_keep: bool,
}

/// A thing Scuttle dragged out from under the filesystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupCandidate {
    /// Stable within a scan. The frontend addresses findings by this and never
    /// by path.
    pub id: String,
    pub detector: String,
    pub category: Category,
    pub target_kind: TargetKind,
    pub path: PathBuf,
    /// Human-facing name. Often the file name; sometimes the application it
    /// belonged to.
    pub display_name: String,
    /// The application, game or tool this appears to belong to, if known.
    pub associated_app: Option<String>,
    /// Bytes Scuttle believes would be reclaimed. For groups this is the sum
    /// of the redundant members, not the whole group.
    pub size: u64,
    pub confidence: Confidence,
    pub risk: Risk,
    pub recommended_action: RecommendedAction,
    pub evidence: Vec<Evidence>,
    /// One restrained line of Scuttle. Generated by the core from the same
    /// evidence the user can read.
    pub remark: Option<String>,
    pub modified_unix: Option<i64>,
    pub accessed_unix: Option<i64>,
    pub created_unix: Option<i64>,
    /// Sibling files for group findings. Empty for singular findings.
    pub group: Vec<GroupMember>,
    /// Snapshot of file state at scan time, used to detect staleness later.
    pub fingerprint: StateFingerprint,
}

/// Enough of the file's state to notice if the world moved underneath us.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateFingerprint {
    pub size: u64,
    pub modified_unix: Option<i64>,
    pub is_dir: bool,
    /// Directories: number of immediate children at scan time. Cheap, and
    /// catches "the user started using this again".
    pub child_count: Option<u64>,
}

impl CleanupCandidate {
    /// Whether the user is allowed to move this into quarantine from the UI.
    /// The backend re-derives this; it does not trust the frontend's copy.
    pub fn is_actionable(&self) -> bool {
        self.risk != Risk::Protected && self.recommended_action != RecommendedAction::InspectOnly
    }
}

/// Bytes, phrased the way a person would say them.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_thresholds_are_coarse_on_purpose() {
        assert_eq!(Confidence::from_score(100), Confidence::High);
        assert_eq!(Confidence::from_score(70), Confidence::High);
        assert_eq!(Confidence::from_score(69), Confidence::Medium);
        assert_eq!(Confidence::from_score(35), Confidence::Medium);
        assert_eq!(Confidence::from_score(34), Confidence::Low);
        assert_eq!(Confidence::from_score(-40), Confidence::Low);
    }

    #[test]
    fn risk_only_ever_goes_up() {
        let mut r = Risk::Low;
        r.raise_to(Risk::High);
        assert_eq!(r, Risk::High);
        r.raise_to(Risk::Low);
        assert_eq!(r, Risk::High, "risk must never be quietly downgraded");
        r.raise_to(Risk::Protected);
        assert_eq!(r, Risk::Protected);
    }

    #[test]
    fn protected_findings_are_never_actionable() {
        let mut c = candidate_fixture();
        c.risk = Risk::Protected;
        assert!(!c.is_actionable());
    }

    #[test]
    fn inspect_only_findings_are_never_actionable() {
        let mut c = candidate_fixture();
        c.recommended_action = RecommendedAction::InspectOnly;
        assert!(!c.is_actionable());
    }

    #[test]
    fn bytes_read_like_a_person_wrote_them() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1024), "1.00 KB");
        assert_eq!(human_bytes(1024 * 1024 * 6 + 1024 * 200), "6.20 MB");
        assert_eq!(human_bytes(1024u64.pow(3) * 318), "318 GB");
    }

    #[test]
    fn category_slugs_round_trip() {
        for c in Category::ALL {
            assert_eq!(Category::from_slug(c.slug()), Some(c));
        }
        assert_eq!(Category::from_slug("nonsense"), None);
    }

    fn candidate_fixture() -> CleanupCandidate {
        CleanupCandidate {
            id: "x".into(),
            detector: "test".into(),
            category: Category::Oddments,
            target_kind: TargetKind::File,
            path: PathBuf::from("/tmp/x"),
            display_name: "x".into(),
            associated_app: None,
            size: 1,
            confidence: Confidence::High,
            risk: Risk::Low,
            recommended_action: RecommendedAction::Quarantine,
            evidence: vec![],
            remark: None,
            modified_unix: None,
            accessed_unix: None,
            created_unix: None,
            group: vec![],
            fingerprint: StateFingerprint::default(),
        }
    }
}
