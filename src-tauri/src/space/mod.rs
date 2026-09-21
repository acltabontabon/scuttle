//! Where the space went, in a sentence a person would say.
//!
//! This is deliberately *not* a disk analyser. It does not draw a treemap of
//! every directory. It answers one question — "what is actually taking up the
//! room, and how much of that is worth a second look" — and it is careful to
//! phrase estimates as estimates.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::{human_bytes, Category, RecommendedAction};
use crate::platform::PlatformService;
use crate::safety::ProtectedPaths;
use crate::scanning::walk;
use crate::storage::Store;
use crate::Result;

/// How many entries each area measurement will look at before settling for an
/// estimate. Keeps the whole overview to a few seconds.
const MEASURE_BUDGET: usize = 120_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Area {
    pub label: String,
    pub bytes: u64,
    /// False when the measurement was truncated or hit unreadable corners, so
    /// the interface can say "at least" rather than a flat number.
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryTally {
    pub category: Category,
    pub label: String,
    pub bytes: u64,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceOverview {
    pub volume_total: u64,
    pub volume_free: u64,
    pub volume_used: u64,
    /// The big areas, largest first.
    pub areas: Vec<Area>,
    /// What the last rummage turned up, per pile.
    pub worth_checking: Vec<CategoryTally>,
    /// Bytes Scuttle believes could plausibly be reclaimed. Always phrased as
    /// "around", never as "you can safely delete".
    pub reclaimable_estimate: u64,
    /// Whether a rummage has happened at all yet.
    pub has_findings: bool,
    /// One honest sentence summarising the above.
    pub summary: String,
}

/// Read total and free bytes for the volume holding `path`.
pub fn volume_usage(path: &std::path::Path) -> Option<(u64, u64)> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    // The mount point with the longest matching prefix is the right volume.
    disks
        .list()
        .iter()
        .filter(|disk| path.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(|disk| (disk.total_space(), disk.available_space()))
}

/// Areas worth naming, resolved for this machine.
fn areas_to_measure(platform: &dyn PlatformService) -> Vec<(String, PathBuf)> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let mut areas: Vec<(String, PathBuf)> = Vec::new();

    for library in platform.game_libraries() {
        for root in library.install_roots {
            if root.is_dir() {
                areas.push((format!("{} games", library.kind.label()), root));
            }
        }
    }

    for (label, relative) in [
        ("Photos and video", "Pictures"),
        ("Music", "Music"),
        ("Movies", "Movies"),
        ("Downloads", "Downloads"),
        ("Desktop", "Desktop"),
        ("Documents", "Documents"),
    ] {
        let path = home.join(relative);
        if path.is_dir() {
            areas.push((label.to_string(), path));
        }
    }

    for root in platform.application_data_roots() {
        if let Some(name) = root.file_name() {
            areas.push((format!("App data ({})", name.to_string_lossy()), root));
        }
    }

    areas
}

/// Build the overview. Bounded, cancellable and honest about its own limits.
pub fn overview(
    platform: &dyn PlatformService,
    protected: &ProtectedPaths,
    store: &Store,
    cancel: &dyn Fn() -> bool,
) -> Result<SpaceOverview> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let (volume_total, volume_free) = volume_usage(&home).unwrap_or((0, 0));

    let mut areas: Vec<Area> = Vec::new();
    for (label, path) in areas_to_measure(platform) {
        if cancel() {
            break;
        }
        let (bytes, _, complete) = walk::measure_tree(&path, protected, MEASURE_BUDGET, cancel);
        if bytes == 0 {
            continue;
        }
        // Several areas can resolve to the same label (two Steam libraries);
        // fold them together rather than listing duplicates.
        match areas.iter_mut().find(|a| a.label == label) {
            Some(existing) => {
                existing.bytes += bytes;
                existing.complete &= complete;
            }
            None => areas.push(Area {
                label,
                bytes,
                complete,
            }),
        }
    }
    areas.sort_by_key(|a| std::cmp::Reverse(a.bytes));
    areas.truncate(6);

    // What the last rummage actually found.
    let mut worth_checking: Vec<CategoryTally> = Vec::new();
    let mut reclaimable_estimate = 0u64;
    let mut has_findings = false;

    if let Some(scan) = store.latest_scan()? {
        let candidates = store.candidates_for_scan(&scan.id)?;
        has_findings = !candidates.is_empty();
        for category in Category::ALL {
            let matching: Vec<_> = candidates
                .iter()
                .filter(|c| c.category == category)
                .collect();
            if matching.is_empty() {
                continue;
            }
            worth_checking.push(CategoryTally {
                category,
                label: category.title().to_string(),
                bytes: matching.iter().map(|c| c.size).sum(),
                count: matching.len() as u64,
            });
        }
        worth_checking.sort_by_key(|t| std::cmp::Reverse(t.bytes));
        reclaimable_estimate = candidates
            .iter()
            .filter(|c| c.recommended_action != RecommendedAction::InspectOnly)
            .map(|c| c.size)
            .sum();
    }

    let volume_used = volume_total.saturating_sub(volume_free);
    let summary = summarise(volume_used, reclaimable_estimate, has_findings);

    Ok(SpaceOverview {
        volume_total,
        volume_free,
        volume_used,
        areas,
        worth_checking,
        reclaimable_estimate,
        has_findings,
        summary,
    })
}

/// The sentence. Note the hedging: "around", "may be", never "you can safely
/// delete", which is a promise no scanner is in a position to make.
fn summarise(used: u64, reclaimable: u64, has_findings: bool) -> String {
    if used == 0 {
        return "Scuttle could not read the size of this volume.".to_string();
    }
    if !has_findings {
        return format!(
            "{} in use. Nothing rummaged through yet.",
            human_bytes(used)
        );
    }
    if reclaimable == 0 {
        return format!(
            "{} in use.\n\nNothing Scuttle found looks worth reclaiming.",
            human_bytes(used)
        );
    }
    format!(
        "{} in use.\n\nAround {} may be reclaimable, depending on what you still want.",
        human_bytes(used),
        human_bytes(reclaimable)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{ev, EvidenceKind};
    use crate::model::*;
    use crate::scanning::{ScanOptions, ScanSummary};

    fn candidate(
        id: &str,
        category: Category,
        size: u64,
        action: RecommendedAction,
    ) -> CleanupCandidate {
        CleanupCandidate {
            id: id.into(),
            detector: "test".into(),
            category,
            target_kind: TargetKind::File,
            path: PathBuf::from(format!("/x/{id}")),
            display_name: id.into(),
            associated_app: None,
            size,
            group_bytes: size,
            confidence: Confidence::High,
            risk: Risk::Low,
            recommended_action: action,
            evidence: vec![ev(EvidenceKind::LargeSize { bytes: size })],
            remark: None,
            modified_unix: None,
            accessed_unix: None,
            created_unix: None,
            group: vec![],
            fingerprint: StateFingerprint::default(),
            assessment: Default::default(),
        }
    }

    fn store_with(candidates: &[CleanupCandidate]) -> Store {
        let store = Store::in_memory().unwrap();
        store
            .begin_scan(
                "s1",
                crate::storage::ScanKind::Full,
                &ScanOptions::default(),
                0,
            )
            .unwrap();
        store.save_candidates("s1", candidates).unwrap();
        store
            .finish_scan(
                &ScanSummary {
                    scan_id: "s1".into(),
                    files_seen: 1,
                    bytes_seen: 1,
                    candidates_found: candidates.len() as u64,
                    reclaimable_bytes: 0,
                    duration_ms: 1,
                    walk_ms: 1,
                    probe_ms: 0,
                    finish_ms: 0,
                    cancelled: false,
                    hiccups: Default::default(),
                },
                1,
            )
            .unwrap();
        store
    }

    #[test]
    fn uncertainty_is_phrased_as_uncertainty() {
        let text = summarise(731 * 1024_u64.pow(3), 43 * 1024_u64.pow(3), true);
        assert!(text.contains("Around"), "{text}");
        assert!(text.contains("may be reclaimable"), "{text}");
        assert!(
            !text.to_lowercase().contains("safely delete"),
            "Scuttle must never promise this"
        );
    }

    #[test]
    fn a_machine_with_nothing_to_reclaim_says_so_plainly() {
        let text = summarise(500 * 1024_u64.pow(3), 0, true);
        assert!(text.contains("Nothing Scuttle found"));
    }

    #[test]
    fn before_the_first_rummage_no_claims_are_made() {
        let text = summarise(500 * 1024_u64.pow(3), 0, false);
        assert!(text.contains("Nothing rummaged through yet"));
    }

    #[test]
    fn tallies_group_by_pile_and_sort_by_size() {
        let store = store_with(&[
            candidate("a", Category::Ghosts, 300, RecommendedAction::Quarantine),
            candidate(
                "b",
                Category::Installers,
                900,
                RecommendedAction::Quarantine,
            ),
            candidate("c", Category::Ghosts, 100, RecommendedAction::Quarantine),
        ]);
        let platform = crate::platform::current();
        let protected = ProtectedPaths::for_home("/nonexistent-home-for-tests");
        let overview = overview(platform.as_ref(), &protected, &store, &|| true).unwrap();

        assert_eq!(overview.worth_checking[0].category, Category::Installers);
        assert_eq!(overview.worth_checking[0].bytes, 900);
        assert_eq!(overview.worth_checking[1].category, Category::Ghosts);
        assert_eq!(overview.worth_checking[1].bytes, 400);
        assert_eq!(overview.worth_checking[1].count, 2);
    }

    #[test]
    fn inspect_only_findings_are_excluded_from_the_estimate() {
        let store = store_with(&[
            candidate(
                "a",
                Category::Installers,
                500,
                RecommendedAction::Quarantine,
            ),
            candidate(
                "b",
                Category::HeavyStrays,
                9000,
                RecommendedAction::InspectOnly,
            ),
        ]);
        let platform = crate::platform::current();
        let protected = ProtectedPaths::for_home("/nonexistent-home-for-tests");
        let overview = overview(platform.as_ref(), &protected, &store, &|| true).unwrap();
        assert_eq!(
            overview.reclaimable_estimate, 500,
            "a large file Scuttle refuses to recommend is not a saving"
        );
        // ...but it is still shown, because it is worth knowing about.
        assert!(overview
            .worth_checking
            .iter()
            .any(|t| t.category == Category::HeavyStrays));
    }

    #[test]
    fn the_real_volume_reports_a_plausible_size() {
        let home = dirs::home_dir().unwrap();
        let (total, free) = volume_usage(&home).expect("the home volume should be readable");
        assert!(total > 0);
        assert!(free <= total);
    }
}
