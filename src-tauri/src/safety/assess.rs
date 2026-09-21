//! Three questions, kept apart.
//!
//! Scuttle used to fold everything into one "risk" rating, and one rating
//! cannot say three different things:
//!
//! 1. **Confidence** — how sure Scuttle is about what an item *is*. Computed
//!    from the evidence by [`crate::evidence::weigh`].
//! 2. **Impact** — what moving it could disrupt: nothing (a cache rebuilds
//!    itself), a download, a person's own file, an application's data, or an
//!    application itself.
//! 3. **Eligibility** — which ways of asking may move it: a sweep Scuttle
//!    proposes, a batch a person picked, only a single deliberate choice, or
//!    nothing at all.
//!
//! The rules that follow from keeping them apart:
//!
//! * Low confidence about a person's own file is not a reason to stop them
//!   moving it. It is a reason to tell them what Scuttle is unsure about.
//! * Cautions are acknowledged once per reviewed batch, by kind — never file by
//!   file.
//! * Applications and their data are never suggested and never swept. An
//!   application folder moves only when a person chooses that one folder.
//! * Only the things that cannot be done safely are blocked outright.
//!
//! The frontend receives an [`Assessment`] to explain itself, but the gate
//! recomputes it from the stored evidence and the live filesystem. Nothing the
//! webview sends can change an item's eligibility.

use serde::{Deserialize, Serialize};

use crate::evidence::EvidenceKind;
use crate::model::{Category, CleanupCandidate, Confidence, RecommendedAction, Risk, TargetKind};
use crate::platform::InstallRoot;

/// What moving an item could disrupt. Ordered from least to most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Impact {
    /// Rebuilt automatically by whatever made it (caches, build output).
    Regenerable,
    /// Costs a download to get back.
    Redownloadable,
    /// A person's own file: a download, a screenshot, a document, an archive.
    PersonalFile,
    /// Data an application keeps: settings, profiles, perhaps files made in it.
    ApplicationData,
    /// An application, or part of one. Moving it will probably break it.
    ApplicationInstall,
}

impl Impact {
    /// Whether the Drawer may let this expire on its own. Only things that
    /// can be rebuilt or fetched again: anything else stays until a person
    /// removes it, so a move they chose never turns permanent behind them.
    pub fn may_expire(&self) -> bool {
        matches!(self, Impact::Regenerable | Impact::Redownloadable)
    }

    pub fn slug(&self) -> &'static str {
        match self {
            Impact::Regenerable => "regenerable",
            Impact::Redownloadable => "redownloadable",
            Impact::PersonalFile => "personal_file",
            Impact::ApplicationData => "application_data",
            Impact::ApplicationInstall => "application_install",
        }
    }

    pub fn from_slug(s: &str) -> Option<Impact> {
        [
            Impact::Regenerable,
            Impact::Redownloadable,
            Impact::PersonalFile,
            Impact::ApplicationData,
            Impact::ApplicationInstall,
        ]
        .into_iter()
        .find(|i| i.slug() == s)
    }
}

/// Something a person should know before moving an item. Acknowledged once
/// per kind, per reviewed batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CautionKind {
    /// Changed recently; it may still be in use.
    RecentlyChanged,
    /// Looks like something a person made (documents, saves, projects).
    MayBeYourWork,
    /// Something is using it, or its application is running.
    InUse,
    /// An application's data: settings, profiles, possibly the person's files.
    ApplicationData,
    /// Part of an application. Moving it will probably break it.
    BreaksApplication,
    /// Scuttle could not work out with any confidence what this is.
    Uncertain,
}

impl CautionKind {
    pub const ALL: [CautionKind; 6] = [
        CautionKind::RecentlyChanged,
        CautionKind::MayBeYourWork,
        CautionKind::InUse,
        CautionKind::ApplicationData,
        CautionKind::BreaksApplication,
        CautionKind::Uncertain,
    ];

    /// The sentence shown once for the whole batch.
    pub fn headline(&self) -> &'static str {
        match self {
            CautionKind::RecentlyChanged => {
                "Some of this changed recently. You may still be using it."
            }
            CautionKind::MayBeYourWork => "Some of this looks like something you made.",
            CautionKind::InUse => "Something is using some of this right now.",
            CautionKind::ApplicationData => {
                "Some of this is an application's own data. It can hold settings or files you \
                 made, and the application may miss it."
            }
            CautionKind::BreaksApplication => {
                "This is part of an application. Moving it will probably stop that application \
                 working until you put it back."
            }
            CautionKind::Uncertain => "Scuttle is not sure what some of this is.",
        }
    }
}

/// One caution about one item, with the specific reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caution {
    pub kind: CautionKind,
    pub detail: String,
}

/// Which requests may move an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Eligibility {
    /// Scuttle proposes it, and may include it in a sweep and in default
    /// selections. Nothing about it needs acknowledging.
    Suggested,
    /// A person may move it, in a batch or on its own, once they have seen
    /// its cautions.
    ByChoice,
    /// Only when a person chooses this one item specifically. Never in a
    /// batch, a sweep, a group action or a select-all.
    ExplicitOnly,
    /// Cannot be moved safely by any request.
    Blocked,
}

/// Everything the three questions answered for one item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assessment {
    pub impact: Impact,
    pub eligibility: Eligibility,
    pub cautions: Vec<Caution>,
    /// When blocked, why, in words a person can read.
    pub blocked: Option<String>,
}

impl Default for Assessment {
    /// The conservative default for anything not assessed yet: a person's
    /// own file, moved only by choice.
    fn default() -> Self {
        Assessment {
            impact: Impact::PersonalFile,
            eligibility: Eligibility::ByChoice,
            cautions: Vec::new(),
            blocked: None,
        }
    }
}

/// How recent a change has to be before it is worth a caution.
pub const RECENT_DAYS: i64 = 7;

/// Answer the three questions for a candidate, from its evidence.
pub fn assess(candidate: &CleanupCandidate, now_unix: i64) -> Assessment {
    let has =
        |pred: &dyn Fn(&EvidenceKind) -> bool| candidate.evidence.iter().any(|e| pred(&e.kind));

    let install = candidate.evidence.iter().find_map(|e| match &e.kind {
        EvidenceKind::PartOfInstalledApplication { app, how } => Some((app.clone(), how.clone())),
        _ => None,
    });

    let impact = if install.is_some() {
        Impact::ApplicationInstall
    } else {
        match candidate.category {
            Category::Caches | Category::DeveloperDebris => Impact::Regenerable,
            Category::Ghosts | Category::Oddments => Impact::ApplicationData,
            _ if has(&|k| matches!(k, EvidenceKind::InsideApplicationData)) => {
                Impact::ApplicationData
            }
            Category::Installers => Impact::Redownloadable,
            _ => Impact::PersonalFile,
        }
    };

    let mut cautions = Vec::new();
    if let Some((app, how)) = &install {
        cautions.push(Caution {
            kind: CautionKind::BreaksApplication,
            detail: format!("Part of {app} ({how})."),
        });
    }
    if impact == Impact::ApplicationData {
        cautions.push(Caution {
            kind: CautionKind::ApplicationData,
            detail: match &candidate.associated_app {
                Some(app) => format!("Data kept by {app}."),
                None => "Data an application keeps for itself.".to_string(),
            },
        });
    }
    for e in &candidate.evidence {
        match &e.kind {
            EvidenceKind::UserContentDetected { reason }
            | EvidenceKind::SaveDataDetected { reason } => cautions.push(Caution {
                kind: CautionKind::MayBeYourWork,
                detail: capitalise(reason),
            }),
            EvidenceKind::InUse { by } => cautions.push(Caution {
                kind: CautionKind::InUse,
                detail: format!("{by} appears to be using it."),
            }),
            _ => {}
        }
    }
    // Recency matters for anything that is not rebuilt on demand; a cache is
    // *supposed* to change all the time.
    if impact != Impact::Regenerable {
        let recent_evidence = candidate.evidence.iter().find_map(|e| match e.kind {
            EvidenceKind::RecentlyTouched { days } => Some(days as i64),
            _ => None,
        });
        let changed_days = candidate
            .fingerprint
            .modified_unix
            .or(candidate.modified_unix)
            .map(|m| ((now_unix - m).max(0)) / 86_400);
        let days = recent_evidence.or(changed_days.filter(|d| *d < RECENT_DAYS));
        if let Some(days) = days {
            cautions.push(Caution {
                kind: CautionKind::RecentlyChanged,
                detail: match days {
                    0 => "Changed today.".to_string(),
                    1 => "Changed yesterday.".to_string(),
                    d => format!("Changed {d} days ago."),
                },
            });
        }
    }
    let unsure = candidate.confidence == Confidence::Low
        || has(&|k| matches!(k, EvidenceKind::Unclassified { .. }));
    if unsure && impact != Impact::Regenerable {
        cautions.push(Caution {
            kind: CautionKind::Uncertain,
            detail: "The evidence is thin or points both ways.".to_string(),
        });
    }
    cautions.sort_by_key(|c| c.kind);
    cautions.dedup_by_key(|c| c.kind);

    let (eligibility, blocked) = if candidate.risk == Risk::Protected {
        let rule = candidate.evidence.iter().find_map(|e| match &e.kind {
            EvidenceKind::ProtectedByRule { rule } => Some(rule.clone()),
            _ => None,
        });
        (
            Eligibility::Blocked,
            Some(match rule {
                Some(rule) => format!("Protected: {rule}."),
                None => "Scuttle will not move this.".to_string(),
            }),
        )
    } else if impact == Impact::ApplicationInstall {
        (Eligibility::ExplicitOnly, None)
    } else if candidate.recommended_action == RecommendedAction::Quarantine
        && impact.may_expire()
        && cautions.is_empty()
    {
        (Eligibility::Suggested, None)
    } else {
        (Eligibility::ByChoice, None)
    };

    Assessment {
        impact,
        eligibility,
        cautions,
        blocked,
    }
}

/// Settle a freshly weighed or freshly loaded candidate: attach its
/// assessment, and make sure the older single rating never says more than
/// the assessment allows. A candidate Scuttle would not sweep is never shown
/// as one it recommends.
pub fn settle(candidate: &mut CleanupCandidate, now_unix: i64) {
    let assessment = assess(candidate, now_unix);
    match assessment.eligibility {
        Eligibility::Suggested => {}
        Eligibility::ByChoice => {
            if candidate.recommended_action == RecommendedAction::Quarantine {
                candidate.recommended_action = RecommendedAction::Review;
            }
        }
        Eligibility::ExplicitOnly | Eligibility::Blocked => {
            candidate.recommended_action = RecommendedAction::InspectOnly;
        }
    }
    candidate.assessment = assessment;
}

/// Re-assess with what the live filesystem says about applications: an item
/// that turns out to be part of one (or to hold one) is treated as an
/// application whatever the stored finding claims. This is how a finding
/// recorded before Scuttle knew better is still caught at the gate.
pub fn with_installation(
    mut assessment: Assessment,
    install: Option<&InstallRoot>,
    kind: TargetKind,
) -> Assessment {
    let Some(install) = install else {
        return assessment;
    };
    assessment.impact = Impact::ApplicationInstall;
    if assessment.eligibility != Eligibility::Blocked {
        assessment.eligibility = Eligibility::ExplicitOnly;
    }
    let detail = match kind {
        TargetKind::Directory => format!(
            "Holds or belongs to {} ({}).",
            install.name,
            install.kind.describe()
        ),
        TargetKind::File => format!("Part of {} ({}).", install.name, install.kind.describe()),
    };
    if !assessment
        .cautions
        .iter()
        .any(|c| c.kind == CautionKind::BreaksApplication)
    {
        assessment.cautions.push(Caution {
            kind: CautionKind::BreaksApplication,
            detail,
        });
        assessment.cautions.sort_by_key(|c| c.kind);
    }
    assessment
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => {
            let mut out = first.to_uppercase().collect::<String>();
            out.push_str(chars.as_str());
            if !out.ends_with('.') {
                out.push('.');
            }
            out
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::ev;
    use crate::model::{GroupMember, StateFingerprint};
    use std::path::PathBuf;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn candidate(category: Category, evidence: Vec<EvidenceKind>) -> CleanupCandidate {
        let evidence: Vec<_> = evidence.into_iter().map(ev).collect();
        let verdict = crate::evidence::weigh(&evidence, Risk::Low);
        CleanupCandidate {
            id: "x".into(),
            detector: "test".into(),
            category,
            target_kind: TargetKind::File,
            path: PathBuf::from("/tmp/x"),
            display_name: "x".into(),
            associated_app: None,
            size: 1,
            group_bytes: 1,
            confidence: verdict.confidence,
            risk: verdict.risk,
            recommended_action: verdict.action,
            evidence,
            remark: None,
            modified_unix: Some(NOW - 400 * DAY),
            accessed_unix: None,
            created_unix: None,
            group: Vec::<GroupMember>::new(),
            fingerprint: StateFingerprint {
                modified_unix: Some(NOW - 400 * DAY),
                ..Default::default()
            },
            assessment: Assessment::default(),
        }
    }

    #[test]
    fn a_cache_scuttle_is_sure_of_is_suggested() {
        let c = candidate(
            Category::Caches,
            vec![
                EvidenceKind::KnownCachePath { owner: "X".into() },
                EvidenceKind::RegeneratedOnDemand { owner: "X".into() },
            ],
        );
        let a = assess(&c, NOW);
        assert_eq!(a.impact, Impact::Regenerable);
        assert_eq!(a.eligibility, Eligibility::Suggested);
        assert!(a.cautions.is_empty());
    }

    #[test]
    fn a_recent_personal_file_is_movable_with_a_caution_not_blocked() {
        let mut c = candidate(
            Category::HeavyStrays,
            vec![
                EvidenceKind::LargeSize { bytes: 1 << 31 },
                EvidenceKind::RecentlyTouched { days: 1 },
            ],
        );
        c.fingerprint.modified_unix = Some(NOW - DAY);
        let a = assess(&c, NOW);
        assert_eq!(a.impact, Impact::PersonalFile);
        assert_eq!(a.eligibility, Eligibility::ByChoice);
        assert!(a
            .cautions
            .iter()
            .any(|x| x.kind == CautionKind::RecentlyChanged));
    }

    #[test]
    fn low_confidence_is_a_caution_not_a_lock() {
        let c = candidate(
            Category::HeavyStrays,
            vec![EvidenceKind::Unclassified {
                reason: "no idea".into(),
            }],
        );
        let a = assess(&c, NOW);
        assert_eq!(a.eligibility, Eligibility::ByChoice);
        assert!(a.cautions.iter().any(|x| x.kind == CautionKind::Uncertain));
    }

    #[test]
    fn an_installation_is_only_ever_an_explicit_choice() {
        let c = candidate(
            Category::Installers,
            vec![
                EvidenceKind::InstallerFormat { ext: "exe".into() },
                EvidenceKind::InstalledAppSupersedes { app: "Chat".into() },
                EvidenceKind::PartOfInstalledApplication {
                    app: "Chat".into(),
                    how: "an application with its own updater".into(),
                },
            ],
        );
        let a = assess(&c, NOW);
        assert_eq!(a.impact, Impact::ApplicationInstall);
        assert_eq!(a.eligibility, Eligibility::ExplicitOnly);
        assert!(a
            .cautions
            .iter()
            .any(|x| x.kind == CautionKind::BreaksApplication));
    }

    #[test]
    fn application_data_is_never_suggested() {
        let c = candidate(
            Category::Ghosts,
            vec![
                EvidenceKind::ApplicationNotInstalled { app: "Old".into() },
                EvidenceKind::MatchesApplicationId {
                    id: "com.old".into(),
                },
                EvidenceKind::UntouchedFor { days: 900 },
                EvidenceKind::GeneratedContent {
                    reason: "blobs".into(),
                },
            ],
        );
        let mut settled = c.clone();
        settle(&mut settled, NOW);
        assert_eq!(settled.assessment.impact, Impact::ApplicationData);
        assert_eq!(settled.assessment.eligibility, Eligibility::ByChoice);
        assert_ne!(settled.recommended_action, RecommendedAction::Quarantine);
    }

    #[test]
    fn protected_is_blocked_and_says_why() {
        let c = candidate(
            Category::Oddments,
            vec![EvidenceKind::ProtectedByRule {
                rule: "SSH keys".into(),
            }],
        );
        let a = assess(&c, NOW);
        assert_eq!(a.eligibility, Eligibility::Blocked);
        assert!(a.blocked.unwrap().contains("SSH keys"));
    }

    #[test]
    fn only_rebuildable_things_may_expire() {
        assert!(Impact::Regenerable.may_expire());
        assert!(Impact::Redownloadable.may_expire());
        assert!(!Impact::PersonalFile.may_expire());
        assert!(!Impact::ApplicationData.may_expire());
        assert!(!Impact::ApplicationInstall.may_expire());
    }
}
