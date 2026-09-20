//! Caches with a known owner.
//!
//! Every finding here comes from a rule in [`crate::platform::caches`] that
//! names the owning application and states what happens when the data is
//! gone. There is deliberately no "anything in a folder called cache" rule:
//! that is how cleanup software deletes people's work.

use crate::evidence::EvidenceKind;
use crate::model::{human_bytes, Category, Risk, TargetKind};
use crate::platform::caches::CacheSafety;
use crate::scanning::walk::FileEntry;
use crate::scanning::{CandidateSink, Detector, Finding, ScanContext};

/// Not worth mentioning a cache smaller than this.
const MIN_INTERESTING_BYTES: u64 = 50 * 1024 * 1024;

pub struct CacheDetector;

impl CacheDetector {
    pub fn new() -> Self {
        CacheDetector
    }
}

impl Default for CacheDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector for CacheDetector {
    fn id(&self) -> &'static str {
        "caches"
    }

    fn category(&self) -> Category {
        Category::Caches
    }

    fn rummaging_note(&self) -> &'static str {
        "Checking caches Scuttle recognises"
    }

    fn probe(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {
        for rule in ctx.platform.cache_rules() {
            if ctx.cancelled() {
                return;
            }
            if rule.developer_only && !ctx.options.include_developer_debris {
                continue;
            }
            if !rule.path.is_dir() {
                continue;
            }

            let profile = ctx.profile_of(&rule.path);
            if profile.bytes < MIN_INTERESTING_BYTES {
                continue;
            }

            let entry = FileEntry::for_directory(&rule.path, profile.bytes, profile.newest_unix);

            // The rule's own safety statement decides the starting risk. A
            // cache that costs the user something is never low risk however
            // confidently we recognise it.
            let base_risk = match rule.safety {
                CacheSafety::Regenerates => Risk::Low,
                CacheSafety::RegeneratesWhenClosed => Risk::Low,
                CacheSafety::CostsSomething => Risk::High,
            };

            let mut finding = Finding::new("caches", Category::Caches, &entry, rule.label)
                .app(rule.owner)
                .size(profile.bytes)
                .risk(base_risk)
                .classified()
                .with(EvidenceKind::KnownCachePath {
                    owner: rule.owner.to_string(),
                });
            finding.target_kind = TargetKind::Directory;

            if !matches!(rule.safety, CacheSafety::CostsSomething) {
                finding = finding.with(EvidenceKind::RegeneratedOnDemand {
                    owner: rule.owner.to_string(),
                });
            }

            // Clearing a cache out from under a running application is how
            // "cleaners" corrupt profiles. If the owner is up, say so.
            if let Some(process) = rule.owner_process {
                match ctx.process_running(process) {
                    Some(true) => {
                        finding = finding.with(EvidenceKind::InUse {
                            by: rule.owner.to_string(),
                        })
                    }
                    Some(false) => finding = finding.with(EvidenceKind::NoProcessUsingIt),
                    None => {}
                }
            }

            let idle_days = if profile.newest_unix > 0 {
                ((ctx.now_unix - profile.newest_unix).max(0) / 86_400) as u32
            } else {
                0
            };
            if idle_days >= 30 {
                finding = finding.with(EvidenceKind::UntouchedFor { days: idle_days });
            }

            sink.emit(finding.saying(remark(rule.owner, rule.safety, profile.bytes, idle_days)));
        }
    }
}

fn remark(owner: &str, safety: CacheSafety, bytes: u64, idle_days: u32) -> String {
    let size = human_bytes(bytes);
    match safety {
        CacheSafety::CostsSomething => {
            format!("{size} of {owner} cache.\n\n{}", safety.explanation())
        }
        _ if idle_days >= 120 => format!(
            "{size} of {owner} cache, untouched for {idle_days} days.\n\n{}",
            safety.explanation()
        ),
        _ => format!("{size} of {owner} cache.\n\n{}", safety.explanation()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detectors::test_support::*;
    use crate::model::RecommendedAction;
    use crate::platform::caches::CacheRule;

    fn mb(n: u64) -> u64 {
        n * 1024 * 1024
    }

    fn rule(
        h: &Harness,
        rel: &'static str,
        safety: CacheSafety,
        process: Option<&'static str>,
        dev: bool,
    ) -> CacheRule {
        CacheRule {
            owner: "Test Owner",
            label: "Test cache",
            path: h.path(rel),
            safety,
            owner_process: process,
            developer_only: dev,
            settle_secs: 0,
        }
    }

    #[test]
    fn a_regenerating_cache_of_a_closed_app_is_quarantinable() {
        let mut h = Harness::new();
        h.processes = vec!["something-else".into()];
        h.materialise(&[fixture_entry("Caches/owner/blob.cache", 200, mb(90))]);
        h.caches = vec![rule(
            &h,
            "Caches/owner",
            CacheSafety::Regenerates,
            Some("testowner"),
            false,
        )];

        let c = only(h.run_bare(CacheDetector::new()));
        assert_eq!(c.category, Category::Caches);
        assert_eq!(c.risk, Risk::Low);
        assert_eq!(c.recommended_action, RecommendedAction::Quarantine);
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::RegeneratedOnDemand { .. })));
    }

    #[test]
    fn a_cache_whose_owner_is_running_is_not_recommended() {
        let mut h = Harness::new();
        h.processes = vec!["testowner".into()];
        h.materialise(&[fixture_entry("Caches/owner/blob.cache", 200, mb(90))]);
        h.caches = vec![rule(
            &h,
            "Caches/owner",
            CacheSafety::Regenerates,
            Some("testowner"),
            false,
        )];

        let c = only(h.run_bare(CacheDetector::new()));
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::InUse { .. })));
        assert_eq!(c.recommended_action, RecommendedAction::InspectOnly);
    }

    #[test]
    fn a_cache_that_costs_something_is_surfaced_not_recommended() {
        let mut h = Harness::new();
        h.processes = vec!["nothing".into()];
        h.materialise(&[fixture_entry("Caches/music/offline.dat", 300, mb(400))]);
        h.caches = vec![rule(
            &h,
            "Caches/music",
            CacheSafety::CostsSomething,
            None,
            false,
        )];

        let c = only(h.run_bare(CacheDetector::new()));
        assert_eq!(c.risk, Risk::High);
        assert_eq!(c.recommended_action, RecommendedAction::InspectOnly);
        assert!(!c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::RegeneratedOnDemand { .. })));
        assert!(c.remark.as_ref().unwrap().contains("loses something"));
    }

    #[test]
    fn developer_caches_stay_hidden_until_asked_for() {
        let mut h = Harness::new();
        h.materialise(&[fixture_entry("Caches/build/artifact.bin", 200, mb(300))]);
        h.caches = vec![rule(
            &h,
            "Caches/build",
            CacheSafety::Regenerates,
            None,
            true,
        )];
        assert!(h.run_bare(CacheDetector::new()).is_empty());

        h.options.include_developer_debris = true;
        assert_eq!(h.run_bare(CacheDetector::new()).len(), 1);
    }

    #[test]
    fn small_caches_are_not_worth_mentioning() {
        let mut h = Harness::new();
        h.materialise(&[fixture_entry("Caches/owner/blob.cache", 200, mb(2))]);
        h.caches = vec![rule(
            &h,
            "Caches/owner",
            CacheSafety::Regenerates,
            None,
            false,
        )];
        assert!(h.run_bare(CacheDetector::new()).is_empty());
    }

    #[test]
    fn a_rule_pointing_at_nothing_produces_nothing() {
        let mut h = Harness::new();
        h.caches = vec![rule(
            &h,
            "Caches/never-existed",
            CacheSafety::Regenerates,
            None,
            false,
        )];
        assert!(h.run_bare(CacheDetector::new()).is_empty());
    }

    #[test]
    fn the_owner_is_always_named_in_the_finding() {
        let mut h = Harness::new();
        h.materialise(&[fixture_entry("Caches/owner/blob.cache", 200, mb(90))]);
        h.caches = vec![rule(
            &h,
            "Caches/owner",
            CacheSafety::Regenerates,
            None,
            false,
        )];
        let c = only(h.run_bare(CacheDetector::new()));
        assert_eq!(c.associated_app.as_deref(), Some("Test Owner"));
        assert!(c.remark.as_ref().unwrap().contains("Test Owner"));
    }
}
