//! Eligibility is a conjunction of evidence, never a confidence score.
use serde::{Deserialize, Serialize};

pub const POLICY_VERSION: u32 = 1;
pub const DEFAULT_RETENTION_DAYS: u32 = 90;

pub fn retention_days(days: u32) -> u32 {
    match days {
        90 | 180 | 365 => days,
        _ => DEFAULT_RETENTION_DAYS,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Downloaded,
    Local,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Eligible,
    Kept,
    InsufficientEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    UserKept,
    ProjectReference,
    LocalArtifact,
    Snapshot,
    RecentlyChanged,
    RecentlyUsed,
    IncompleteInventory,
    UnknownOrigin,
    UnknownUsage,
    IncompleteProjectCoverage,
    RecoveryUnverified,
    CoordinationUnverified,
    RetentionSatisfied,
}

impl Reason {
    pub fn description(self) -> &'static str {
        match self {
            Self::UserKept => "Covered by a Keep or ignore decision.",
            Self::ProjectReference => {
                "Referenced by a configured project. Older versions can still be needed."
            }
            Self::LocalArtifact => {
                "Locally installed; a downloadable replacement is not established."
            }
            Self::Snapshot => "Snapshot versions are preserved.",
            Self::RecentlyChanged => "Changed within the retention window; preserved.",
            Self::RecentlyUsed => {
                "Reliable usage evidence places this within the retention window."
            }
            Self::IncompleteInventory => "The inventory is incomplete; its size is a lower bound.",
            Self::UnknownOrigin => "The artifact's origin could not be established.",
            Self::UnknownUsage => {
                "Last use is unknown. Modification time is not last-use evidence."
            }
            Self::IncompleteProjectCoverage => {
                "Project information does not cover all dependencies, profiles and branches."
            }
            Self::RecoveryUnverified => {
                "A recoverable move and restore have not been verified for this cache format."
            }
            Self::CoordinationUnverified => {
                "Coordination with concurrent tool processes has not been verified."
            }
            Self::RetentionSatisfied => {
                "All protection, retention, recovery and coordination checks passed."
            }
        }
    }
}

pub struct Facts {
    pub ignored: bool,
    pub referenced: bool,
    pub origin: Origin,
    pub snapshot: bool,
    pub newest_modified_unix: Option<i64>,
    /// None is unknown, not never used. Filesystem atime is not accepted.
    pub last_used_unix: Option<i64>,
    pub complete: bool,
    pub project_coverage_complete: bool,
    pub recovery_verified: bool,
    pub coordination_verified: bool,
}

/// Same facts, policy and evaluation time always yield the same verdict.
pub fn evaluate(f: &Facts, days: u32, now: i64) -> (Decision, Vec<Reason>) {
    let cutoff = now.saturating_sub(i64::from(retention_days(days)) * 86_400);
    let mut keep = Vec::new();
    for (condition, reason) in [
        (f.ignored, Reason::UserKept),
        (f.referenced, Reason::ProjectReference),
        (f.origin == Origin::Local, Reason::LocalArtifact),
        (f.snapshot, Reason::Snapshot),
        (
            f.newest_modified_unix.is_some_and(|t| t >= cutoff),
            Reason::RecentlyChanged,
        ),
        (
            f.last_used_unix.is_some_and(|t| t >= cutoff),
            Reason::RecentlyUsed,
        ),
    ] {
        if condition {
            keep.push(reason);
        }
    }
    if !keep.is_empty() {
        return (Decision::Kept, keep);
    }
    let mut missing = Vec::new();
    for (condition, reason) in [
        (
            !f.complete || f.newest_modified_unix.is_none(),
            Reason::IncompleteInventory,
        ),
        (f.origin == Origin::Unknown, Reason::UnknownOrigin),
        (f.last_used_unix.is_none(), Reason::UnknownUsage),
        (
            !f.project_coverage_complete,
            Reason::IncompleteProjectCoverage,
        ),
        (!f.recovery_verified, Reason::RecoveryUnverified),
        (!f.coordination_verified, Reason::CoordinationUnverified),
    ] {
        if condition {
            missing.push(reason);
        }
    }
    if missing.is_empty() {
        (Decision::Eligible, vec![Reason::RetentionSatisfied])
    } else {
        (Decision::InsufficientEvidence, missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const NOW: i64 = 2_000_000_000;
    fn verified() -> Facts {
        Facts {
            ignored: false,
            referenced: false,
            origin: Origin::Downloaded,
            snapshot: false,
            newest_modified_unix: Some(NOW - 100 * 86_400),
            last_used_unix: Some(NOW - 100 * 86_400),
            complete: true,
            project_coverage_complete: true,
            recovery_verified: true,
            coordination_verified: true,
        }
    }
    #[test]
    fn age_never_substitutes_for_usage_recovery_or_coverage() {
        let mut f = verified();
        assert_eq!(evaluate(&f, 90, NOW).0, Decision::Eligible);
        f.last_used_unix = None;
        assert_eq!(
            evaluate(&f, 90, NOW),
            (Decision::InsufficientEvidence, vec![Reason::UnknownUsage])
        );
        f.last_used_unix = Some(NOW - 100 * 86_400);
        f.project_coverage_complete = false;
        f.recovery_verified = false;
        f.coordination_verified = false;
        assert_eq!(evaluate(&f, 90, NOW).0, Decision::InsufficientEvidence);
    }
    #[test]
    fn every_protection_and_the_exact_boundary_preserve_the_artifact() {
        for index in 0..6 {
            let mut f = verified();
            match index {
                0 => f.ignored = true,
                1 => f.referenced = true,
                2 => f.origin = Origin::Local,
                3 => f.snapshot = true,
                4 => f.newest_modified_unix = Some(NOW - 90 * 86_400),
                _ => f.last_used_unix = Some(NOW + 1),
            }
            assert_eq!(evaluate(&f, 90, NOW).0, Decision::Kept);
        }
        assert_eq!(evaluate(&verified(), 180, NOW).0, Decision::Kept);
        assert_eq!(retention_days(0), 90);
    }
}
