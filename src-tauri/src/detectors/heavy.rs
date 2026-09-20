//! Heavy strays.
//!
//! This detector exists for discovery, not cleanup. Nothing it finds is ever
//! recommended for removal — a large file is a fact about storage, not an
//! accusation. Its base risk is [`Risk::High`] on purpose, which routes every
//! finding to "inspect only" through the normal arithmetic.

use std::collections::HashMap;
use std::path::PathBuf;

use super::naming::display_name;
use crate::evidence::EvidenceKind;
use crate::model::{human_bytes, Category, Risk, TargetKind};
use crate::safety::paths;
use crate::scanning::{CandidateSink, Detector, FileEntry, Finding, ScanContext};

/// How far below a scan root directory totals are accumulated. Deeper than
/// this and the answer stops being useful ("your home folder is large" is not
/// news).
const DIRECTORY_LEVELS: usize = 3;

pub struct HeavyStrayDetector {
    files: Vec<FileEntry>,
    /// Running totals for directories near the top of each scan root.
    directory_totals: HashMap<PathBuf, DirectoryTotal>,
}

struct DirectoryTotal {
    bytes: u64,
    files: u64,
    /// Most recent modification seen anywhere inside.
    newest_unix: i64,
}

impl HeavyStrayDetector {
    pub fn new() -> Self {
        HeavyStrayDetector {
            files: Vec::new(),
            directory_totals: HashMap::new(),
        }
    }
}

impl Default for HeavyStrayDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector for HeavyStrayDetector {
    fn id(&self) -> &'static str {
        "heavy"
    }

    fn category(&self) -> Category {
        Category::HeavyStrays
    }

    fn rummaging_note(&self) -> &'static str {
        "Weighing the big things"
    }

    fn observe(&mut self, entry: &FileEntry, ctx: &ScanContext) {
        if entry.is_dir {
            return;
        }
        if entry.size >= ctx.options.heavy_threshold {
            self.files.push(entry.clone());
        }

        // Accumulate into the handful of directories above this file.
        let modified = entry.modified_unix.unwrap_or(0);
        for level in 1..=DIRECTORY_LEVELS {
            if entry.depth <= level {
                break;
            }
            let Some(ancestor) = entry.path.ancestors().nth(entry.depth - level) else {
                break;
            };
            let total = self
                .directory_totals
                .entry(ancestor.to_path_buf())
                .or_insert(DirectoryTotal {
                    bytes: 0,
                    files: 0,
                    newest_unix: 0,
                });
            total.bytes += entry.size;
            total.files += 1;
            total.newest_unix = total.newest_unix.max(modified);
        }
    }

    fn finish(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {
        for entry in std::mem::take(&mut self.files) {
            if ctx.cancelled() {
                return;
            }
            let mut finding = Finding::new(
                "heavy",
                Category::HeavyStrays,
                &entry,
                display_name(&entry.path),
            )
            .risk(Risk::High)
            .with(EvidenceKind::LargeSize { bytes: entry.size });

            let idle = entry.idle_days(ctx.now_unix);
            if let Some(days) = idle {
                if days >= 60 {
                    finding = finding.with(EvidenceKind::UntouchedFor { days });
                }
            }
            sink.emit(finding.saying(file_remark(entry.size, idle)));
        }

        // Only the most specific qualifying directory in each chain is worth
        // reporting: "Application Support is 40 GB" is less useful than naming
        // the folder inside it that accounts for the bulk.
        let heavy: Vec<(PathBuf, u64, u64, i64)> = self
            .directory_totals
            .iter()
            .filter(|(_, t)| t.bytes >= ctx.options.heavy_threshold)
            .map(|(p, t)| (p.clone(), t.bytes, t.files, t.newest_unix))
            .collect();

        for (path, bytes, files, newest) in &heavy {
            if ctx.cancelled() {
                return;
            }
            let has_qualifying_descendant = heavy
                .iter()
                .any(|(other, _, _, _)| other != path && paths::is_strictly_within(other, path));
            if has_qualifying_descendant {
                continue;
            }

            let entry = FileEntry::for_directory(path, *bytes, *newest);
            let mut finding =
                Finding::new("heavy", Category::HeavyStrays, &entry, display_name(path))
                    .risk(Risk::High)
                    .size(*bytes)
                    .with(EvidenceKind::LargeSize { bytes: *bytes });
            finding.target_kind = TargetKind::Directory;

            let idle = entry.idle_days(ctx.now_unix);
            if let Some(days) = idle {
                if days >= 60 {
                    finding = finding.with(EvidenceKind::UntouchedFor { days });
                }
            }
            sink.emit(finding.saying(directory_remark(*bytes, *files, idle)));
        }
    }
}

fn file_remark(size: u64, idle_days: Option<u32>) -> String {
    match idle_days {
        Some(days) if days >= 300 => format!(
            "{}, last touched {days} days ago.\n\nScuttle isn't calling this junk. But that's a lot of storage.",
            human_bytes(size)
        ),
        Some(days) if days >= 60 => format!(
            "{}, sitting still for {days} days.",
            human_bytes(size)
        ),
        _ => format!("{}. Still in use, by the look of it.", human_bytes(size)),
    }
}

fn directory_remark(bytes: u64, files: u64, idle_days: Option<u32>) -> String {
    let head = format!("{} across {files} files.", human_bytes(bytes));
    match idle_days {
        Some(days) if days >= 300 => {
            format!("{head}\n\nNothing in here has changed in {days} days.")
        }
        _ => format!("{head}\n\nThis folder is enormous. Scuttle has questions."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detectors::test_support::*;
    use crate::model::RecommendedAction;

    fn mb(n: u64) -> u64 {
        n * 1024 * 1024
    }

    fn harness() -> Harness {
        let mut h = Harness::new();
        h.options.heavy_threshold = mb(4);
        h
    }

    #[test]
    fn a_large_file_is_surfaced_but_never_recommended_for_removal() {
        let h = harness();
        let candidates = h.run(
            HeavyStrayDetector::new(),
            vec![fixture_entry(
                "Downloads/android-studio-backup.zip",
                340,
                mb(9),
            )],
        );
        let c = one(&candidates);
        assert_eq!(c.category, Category::HeavyStrays);
        assert_eq!(c.risk, Risk::High);
        assert_eq!(
            c.recommended_action,
            RecommendedAction::InspectOnly,
            "a big file is a fact, not an accusation"
        );
        assert!(c
            .remark
            .as_ref()
            .unwrap()
            .contains("isn't calling this junk"));
    }

    #[test]
    fn files_below_the_threshold_are_left_alone() {
        let h = harness();
        let candidates = h.run(
            HeavyStrayDetector::new(),
            vec![fixture_entry("Downloads/modest.bin", 400, mb(1))],
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn a_heavy_directory_is_reported_at_its_most_specific_level() {
        let h = harness();
        let candidates = h.run(
            HeavyStrayDetector::new(),
            vec![
                fixture_entry("Media/Projects/BigOne/a.bin", 400, mb(3)),
                fixture_entry("Media/Projects/BigOne/b.bin", 400, mb(3)),
            ],
        );
        // Media, Media/Projects and Media/Projects/BigOne all exceed the
        // threshold; only the innermost is worth saying out loud.
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].display_name, "BigOne");
        assert_eq!(candidates[0].target_kind, TargetKind::Directory);
        assert_eq!(candidates[0].size, mb(6));
    }

    #[test]
    fn directory_totals_ignore_files_that_are_individually_heavy_elsewhere() {
        let h = harness();
        let candidates = h.run(
            HeavyStrayDetector::new(),
            vec![
                fixture_entry("Downloads/huge.zip", 400, mb(9)),
                fixture_entry("Elsewhere/Deep/small.bin", 10, mb(1)),
            ],
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].display_name, "huge.zip");
    }

    #[test]
    fn age_changes_what_scuttle_says_but_not_what_it_recommends() {
        let h = harness();
        let fresh = h.run(
            HeavyStrayDetector::new(),
            vec![fixture_entry("Downloads/recent.zip", 2, mb(9))],
        );
        let c = one(&fresh);
        assert!(c.remark.as_ref().unwrap().contains("Still in use"));
        assert_eq!(c.recommended_action, RecommendedAction::InspectOnly);
    }
}
