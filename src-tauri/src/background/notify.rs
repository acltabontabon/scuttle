//! Deciding whether there is anything worth saying, and how to say it.
//!
//! The rule is that silence is the default. A notification costs the user
//! attention they did not offer, so it has to earn itself: something must be
//! genuinely new, there must be enough of it to be worth interrupting for,
//! and it must not have happened recently.
//!
//! Nothing here ever implies that a file is safe to remove, and nothing here
//! ever contains a filename or a path — a summary that appears on a lock
//! screen must not say what is in someone's Downloads folder.

use crate::model::{Category, CleanupCandidate};
use crate::safety::paths;
use crate::storage::BackgroundState;

/// Below this, Scuttle says nothing. A single new file is not worth a
/// notification; it will still be there next time the window is opened.
pub const WORTH_SAYING: usize = 3;

/// At most one summary in this window.
pub const COOLDOWN: i64 = 20 * 60 * 60;

/// A stable, path-free identity for something Scuttle has mentioned.
///
/// Hashed rather than stored plainly because this table would otherwise be an
/// index of the user's filesystem sitting next to their findings, which is
/// exactly what `docs/privacy.md` says Scuttle does not keep.
pub fn notice_key(candidate: &CleanupCandidate) -> String {
    let normalised = paths::normalize(&candidate.path);
    let mut hasher = blake3::Hasher::new();
    hasher.update(candidate.category.slug().as_bytes());
    hasher.update(b"\0");
    hasher.update(normalised.to_string_lossy().as_bytes());
    hasher.finalize().to_hex()[..32].to_string()
}

/// What a completed check found that is worth mentioning.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    /// Every key seen, so they can all be remembered once something is said.
    pub keys: Vec<String>,
    /// Counts by category, largest first. Never paths, never names.
    pub tally: Vec<(Category, usize)>,
    pub total: usize,
}

/// Group new findings into a single summary.
///
/// `fresh` is whatever survived the "have we mentioned this before" check;
/// ignored findings never reach here, because the scan itself filters them.
pub fn summarise(fresh: &[&CleanupCandidate]) -> Notice {
    let mut tally: Vec<(Category, usize)> = Vec::new();
    for candidate in fresh {
        match tally.iter_mut().find(|(c, _)| *c == candidate.category) {
            Some((_, count)) => *count += 1,
            None => tally.push((candidate.category, 1)),
        }
    }
    // Largest first, then by category order, so the same set always reads the
    // same way.
    tally.sort_by(|a, b| {
        b.1.cmp(&a.1).then_with(|| {
            Category::ALL
                .iter()
                .position(|c| *c == a.0)
                .cmp(&Category::ALL.iter().position(|c| *c == b.0))
        })
    });
    Notice {
        keys: fresh.iter().map(|c| notice_key(c)).collect(),
        total: fresh.len(),
        tally,
    }
}

/// Whether to say it.
pub fn worth_interrupting(notice: &Notice, state: &BackgroundState, now: i64) -> bool {
    if notice.total < WORTH_SAYING {
        return false;
    }
    now - state.last_notified_unix >= COOLDOWN
}

/// The summary, as a person would say it.
///
/// Deliberately vague about quantity in the first sentence and precise in the
/// second: "a few things turned up" is the honest register for something
/// nobody asked for, and the count belongs with the category it applies to.
pub fn phrase(notice: &Notice) -> (String, String) {
    let title = "A few things turned up.".to_string();

    let leading = notice
        .tally
        .first()
        .map(|(category, count)| format!("{count} {}", plural(*category, *count)))
        .unwrap_or_else(|| "Nothing in particular".into());

    let body = match notice.tally.len() {
        0 | 1 => format!("{leading} worth a look."),
        2 => {
            let (category, count) = notice.tally[1];
            format!(
                "{leading} and {count} {} worth a look.",
                plural(category, count)
            )
        }
        _ => {
            let rest: usize = notice.tally[1..].iter().map(|(_, n)| n).sum();
            format!("{leading}, and {rest} other things worth a look.")
        }
    };

    (title, body)
}

/// Category names as they read mid-sentence, rather than as headings.
fn plural(category: Category, count: usize) -> &'static str {
    let one = count == 1;
    match category {
        Category::Ghosts if one => "leftover from something uninstalled",
        Category::Ghosts => "leftovers from uninstalled software",
        Category::Screenshots if one => "screenshot",
        Category::Screenshots => "screenshots",
        Category::Installers if one => "old installer",
        Category::Installers => "old installers",
        Category::HeavyStrays if one => "large file",
        Category::HeavyStrays => "large files",
        Category::Copies if one => "copy",
        Category::Copies => "copies",
        Category::Caches if one => "cache",
        Category::Caches => "caches",
        Category::DeveloperDebris if one => "build folder",
        Category::DeveloperDebris => "build folders",
        Category::Oddments if one => "oddment",
        Category::Oddments => "oddments",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Confidence, RecommendedAction, Risk, StateFingerprint, TargetKind};
    use std::path::PathBuf;

    fn candidate(category: Category, path: &str) -> CleanupCandidate {
        CleanupCandidate {
            id: path.to_string(),
            detector: "test".into(),
            category,
            target_kind: TargetKind::File,
            path: PathBuf::from(path),
            display_name: "x".into(),
            associated_app: None,
            size: 1,
            group_bytes: 1,
            confidence: Confidence::High,
            risk: Risk::Low,
            recommended_action: RecommendedAction::Quarantine,
            remark: None,
            modified_unix: None,
            accessed_unix: None,
            created_unix: None,
            fingerprint: StateFingerprint::default(),
            evidence: Vec::new(),
            group: Vec::new(),
        }
    }

    fn state(last_notified: i64) -> BackgroundState {
        BackgroundState {
            last_notified_unix: last_notified,
            ..Default::default()
        }
    }

    #[test]
    fn nothing_new_is_not_worth_saying() {
        let notice = summarise(&[]);
        assert!(!worth_interrupting(&notice, &state(0), 1_700_000_000));
    }

    #[test]
    fn one_stray_file_does_not_earn_an_interruption() {
        let one = candidate(Category::Installers, "/a.dmg");
        let notice = summarise(&[&one]);
        assert!(!worth_interrupting(&notice, &state(0), 1_700_000_000));
    }

    #[test]
    fn a_second_summary_does_not_follow_the_first_the_same_day() {
        let items: Vec<CleanupCandidate> = (0..5)
            .map(|i| candidate(Category::Installers, &format!("/a{i}.dmg")))
            .collect();
        let refs: Vec<&CleanupCandidate> = items.iter().collect();
        let notice = summarise(&refs);
        let now = 1_700_000_000;

        assert!(worth_interrupting(&notice, &state(now - COOLDOWN), now));
        assert!(!worth_interrupting(
            &notice,
            &state(now - COOLDOWN + 1),
            now
        ));
    }

    #[test]
    fn a_summary_never_contains_a_name_or_a_path() {
        // The reason this matters: these strings can appear on a lock screen.
        let items = [
            candidate(Category::Installers, "/Users/someone/Downloads/Payroll.dmg"),
            candidate(Category::Installers, "/Users/someone/Downloads/Taxes.dmg"),
            candidate(Category::Ghosts, "/Users/someone/Library/SecretApp"),
        ];
        let refs: Vec<&CleanupCandidate> = items.iter().collect();
        let (title, body) = phrase(&summarise(&refs));
        let both = format!("{title} {body}");
        for leak in [
            "Payroll",
            "Taxes",
            "SecretApp",
            "someone",
            "/Users",
            "Downloads",
        ] {
            assert!(!both.contains(leak), "{both} leaked {leak}");
        }
    }

    #[test]
    fn the_summary_never_suggests_the_files_should_go() {
        let items: Vec<CleanupCandidate> = (0..4)
            .map(|i| candidate(Category::Installers, &format!("/a{i}.dmg")))
            .collect();
        let refs: Vec<&CleanupCandidate> = items.iter().collect();
        let (title, body) = phrase(&summarise(&refs));
        let both = format!("{title} {body}").to_lowercase();
        for claim in [
            "safe to", "delete", "remove", "clean up", "free up", "junk", "should",
        ] {
            assert!(!both.contains(claim), "{both} implied {claim}");
        }
    }

    #[test]
    fn the_summary_reads_the_way_the_rest_of_scuttle_speaks() {
        // Same rules as src/features/rummage/phrasing.test.ts.
        let items: Vec<CleanupCandidate> = (0..7)
            .map(|i| candidate(Category::Installers, &format!("/a{i}.dmg")))
            .collect();
        let refs: Vec<&CleanupCandidate> = items.iter().collect();
        let (title, body) = phrase(&summarise(&refs));
        let both = format!("{title} {body}");
        assert!(!both.contains('!'));
        assert!(!both
            .chars()
            .collect::<Vec<_>>()
            .windows(4)
            .any(|w| w.iter().all(|c| c.is_ascii_uppercase())));
        for word in [
            "problem", "critical", "warning", "urgent", "boost", "optimise", "optimize",
        ] {
            assert!(!both.to_lowercase().contains(word), "{both}");
        }
    }

    #[test]
    fn counts_are_grouped_by_category_largest_first() {
        let items = [
            candidate(Category::Ghosts, "/g1"),
            candidate(Category::Installers, "/a.dmg"),
            candidate(Category::Installers, "/b.dmg"),
            candidate(Category::Installers, "/c.dmg"),
        ];
        let refs: Vec<&CleanupCandidate> = items.iter().collect();
        let notice = summarise(&refs);
        assert_eq!(notice.total, 4);
        assert_eq!(notice.tally[0], (Category::Installers, 3));
        assert_eq!(notice.tally[1], (Category::Ghosts, 1));

        let (_, body) = phrase(&notice);
        assert_eq!(
            body,
            "3 old installers and 1 leftover from something uninstalled worth a look."
        );
    }

    #[test]
    fn more_than_two_categories_collapse_rather_than_listing_everything() {
        let items = [
            candidate(Category::Installers, "/a.dmg"),
            candidate(Category::Installers, "/b.dmg"),
            candidate(Category::Ghosts, "/g"),
            candidate(Category::Caches, "/c"),
        ];
        let refs: Vec<&CleanupCandidate> = items.iter().collect();
        let (_, body) = phrase(&summarise(&refs));
        assert_eq!(body, "2 old installers, and 2 other things worth a look.");
    }

    #[test]
    fn the_same_file_in_the_same_place_always_hashes_the_same_way() {
        let a = candidate(Category::Installers, "/Users/x/Downloads/Thing.dmg");
        let b = candidate(Category::Installers, "/Users/x/Downloads/Thing.dmg");
        assert_eq!(notice_key(&a), notice_key(&b));
    }

    #[test]
    fn the_key_is_not_the_path() {
        let c = candidate(Category::Installers, "/Users/x/Downloads/Thing.dmg");
        let key = notice_key(&c);
        assert!(!key.contains("Thing"));
        assert!(!key.contains('/'));
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn the_same_file_found_as_two_different_things_is_two_notices() {
        // Category is part of the key because the same path turning up as a
        // ghost and later as a heavy stray is genuinely different news.
        let ghost = candidate(Category::Ghosts, "/same/path");
        let heavy = candidate(Category::HeavyStrays, "/same/path");
        assert_ne!(notice_key(&ghost), notice_key(&heavy));
    }
}
