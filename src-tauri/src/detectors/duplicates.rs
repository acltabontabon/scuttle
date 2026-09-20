//! Byte-identical copies.
//!
//! Hashing is expensive, so the work is staged: group by size first (free,
//! from metadata already gathered), then fingerprint the head and tail of each
//! same-size group, and only fully hash what still looks identical. Large
//! files are streamed rather than read into memory.
//!
//! Scuttle never assumes which copy is the disposable one. A group is
//! presented as a group, with the newest member marked as the obvious keeper
//! and the decision left to the reader.

use rayon::prelude::*;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use super::naming::display_name;
use crate::evidence::EvidenceKind;
use crate::model::{human_bytes, Category, GroupMember, Risk};
use crate::scanning::{CandidateSink, Detector, FileEntry, Finding, ScanContext};

/// Bytes read from each end of a file for the cheap fingerprint.
const PROBE_BYTES: usize = 64 * 1024;
/// Streaming read size for the full hash.
const STREAM_CHUNK: usize = 256 * 1024;

pub struct DuplicateDetector {
    by_size: HashMap<u64, Vec<FileEntry>>,
}

impl DuplicateDetector {
    pub fn new() -> Self {
        DuplicateDetector {
            by_size: HashMap::new(),
        }
    }
}

impl Default for DuplicateDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector for DuplicateDetector {
    fn id(&self) -> &'static str {
        "duplicates"
    }

    fn category(&self) -> Category {
        Category::Copies
    }

    fn rummaging_note(&self) -> &'static str {
        "Looking for copies"
    }

    fn observe(&mut self, entry: &FileEntry, ctx: &ScanContext) {
        if entry.is_dir || entry.size < ctx.options.duplicate_min_size {
            return;
        }
        // Files an application maintains for itself are not the user's copies.
        // Without this, any machine with two versions of an IDE installed
        // reports a pile of "duplicates" inside their plugin directories:
        // true, unactionable, and pure noise.
        if ctx.is_application_managed(&entry.path) {
            return;
        }
        self.by_size
            .entry(entry.size)
            .or_default()
            .push(entry.clone());
    }

    fn finish(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {
        // Only same-size groups can contain duplicates, and that much is free
        // from metadata already gathered.
        let groups: Vec<(u64, Vec<FileEntry>)> = std::mem::take(&mut self.by_size)
            .into_iter()
            .filter(|(_, entries)| entries.len() >= 2)
            .collect();
        if groups.is_empty() || ctx.cancelled() {
            return;
        }

        // Reading file contents is the expensive part of this detector, and
        // it is embarrassingly parallel. The sink is not `Send`, so the work
        // happens across threads and the emission happens after, in order.
        let identical_sets: Vec<(u64, Vec<FileEntry>)> = groups
            .into_par_iter()
            .filter(|_| !ctx.cancelled())
            .flat_map(|(size, entries)| resolve_group(size, entries, || ctx.cancelled()))
            .collect();

        for (size, identical) in identical_sets {
            if ctx.cancelled() {
                return;
            }
            emit_group(identical, size, ctx, sink);
        }
    }
}

/// Narrow one same-size group down to the sets that are genuinely identical.
///
/// Three stages, cheapest first: the size grouping is already done, then a
/// head-and-tail probe rejects most non-matches for two seeks, and only what
/// survives is read in full.
fn resolve_group(
    size: u64,
    entries: Vec<FileEntry>,
    cancelled: impl Fn() -> bool,
) -> Vec<(u64, Vec<FileEntry>)> {
    let mut by_probe: HashMap<[u8; 32], Vec<FileEntry>> = HashMap::new();
    for entry in entries {
        if cancelled() {
            return Vec::new();
        }
        if let Some(probe) = probe_fingerprint(&entry.path, size) {
            by_probe.entry(probe).or_default().push(entry);
        }
    }

    let mut out = Vec::new();
    for group in by_probe.into_values() {
        if group.len() < 2 || cancelled() {
            continue;
        }
        let mut by_hash: HashMap<[u8; 32], Vec<FileEntry>> = HashMap::new();
        for entry in group {
            if let Some(hash) = full_hash(&entry.path) {
                by_hash.entry(hash).or_default().push(entry);
            }
        }
        for mut identical in by_hash.into_values() {
            if identical.len() < 2 {
                continue;
            }
            identical.sort_by_key(|e| std::cmp::Reverse(e.modified_unix.unwrap_or(0)));
            out.push((size, identical));
        }
    }
    out
}

/// Turn a set of identical files into one finding.
fn emit_group(
    identical: Vec<FileEntry>,
    size: u64,
    ctx: &ScanContext,
    sink: &mut dyn CandidateSink,
) {
    let copies = identical.len();
    // The newest is the obvious keeper; the oldest is what Scuttle proposes
    // acting on. Both are suggestions the reader can overrule.
    let newest = &identical[0];
    let oldest = identical.last().expect("group is non-empty");

    let group: Vec<GroupMember> = identical
        .iter()
        .map(|e| GroupMember {
            path: e.path.clone(),
            size: e.size,
            modified_unix: e.modified_unix,
            suggested_keep: e.path == newest.path,
        })
        .collect();

    let mut finding = Finding::new(
        "duplicates",
        Category::Copies,
        oldest,
        display_name(&oldest.path),
    )
    .risk(Risk::Moderate)
    // What could be freed while still keeping a copy, which is what
    // `CleanupCandidate::size` is documented to mean for a group. This used to
    // report one copy's size: correct for a pair by coincidence, and an
    // understatement for every larger set — five identical 2 GB files were
    // reported as 2 GB when 8 GB was redundant.
    .size(size * (copies as u64 - 1))
    .with(EvidenceKind::ExactDuplicate {
        copies: (copies - 1) as u32,
    });

    if let Some(days) = oldest.idle_days(ctx.now_unix) {
        if days >= 60 {
            finding = finding.with(EvidenceKind::UntouchedFor { days });
        }
    }

    finding.group = group;
    sink.emit(finding.saying(remark(copies, size * (copies as u64 - 1), &identical)));
}

/// What Scuttle says about a set of copies. Derived from where they live:
/// copies scattered across different folders read differently from copies
/// sitting side by side.
fn remark(copies: usize, reclaimable: u64, identical: &[FileEntry]) -> String {
    let distinct_folders = {
        let mut folders: Vec<_> = identical
            .iter()
            .filter_map(|e| e.path.parent().map(std::path::Path::to_path_buf))
            .collect();
        folders.sort();
        folders.dedup();
        folders.len()
    };

    let opening = if copies == 2 {
        "There are two of these.".to_string()
    } else {
        format!("There are {copies} of these.")
    };

    if distinct_folders == 1 {
        format!(
            "{opening}\n\nAll in the same folder. {} of it is repetition.",
            human_bytes(reclaimable)
        )
    } else {
        format!(
            "{opening}\n\nSpread across {distinct_folders} folders. {} of it is repetition.",
            human_bytes(reclaimable)
        )
    }
}

/// Hash the first and last chunk of a file plus its size.
///
/// Files that differ almost always differ near one end; this rejects most
/// non-matches for two seeks instead of a full read.
fn probe_fingerprint(path: &PathBuf, size: u64) -> Option<[u8; 32]> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size.to_le_bytes());

    let probe = PROBE_BYTES.min(size as usize);
    let mut buffer = vec![0u8; probe];

    file.read_exact(&mut buffer).ok()?;
    hasher.update(&buffer);

    if size > probe as u64 * 2 {
        file.seek(SeekFrom::End(-(probe as i64))).ok()?;
        file.read_exact(&mut buffer).ok()?;
        hasher.update(&buffer);
    }

    Some(*hasher.finalize().as_bytes())
}

/// Stream the whole file. Never loads more than [`STREAM_CHUNK`] at a time, so
/// a 40 GB disk image costs no more memory than a 40 KB text file.
fn full_hash(path: &PathBuf) -> Option<[u8; 32]> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; STREAM_CHUNK];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            // A file that vanishes or becomes unreadable mid-hash is simply
            // not a duplicate as far as this scan is concerned.
            Err(_) => return None,
        };
    }
    Some(*hasher.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detectors::test_support::*;
    use crate::model::RecommendedAction;

    /// Distinct payloads of the same length, so the size grouping matches but
    /// the contents do not.
    fn payload(marker: u8, len: usize) -> Vec<u8> {
        let mut bytes = vec![marker; len];
        bytes[0] = marker;
        bytes[len - 1] = marker.wrapping_add(1);
        bytes
    }

    fn harness() -> Harness {
        let mut h = Harness::new();
        h.options.duplicate_min_size = 64;
        h
    }

    #[test]
    fn identical_files_become_one_group() {
        let h = harness();
        let bytes = payload(7, 4096);
        let candidates = h.run(
            DuplicateDetector::new(),
            vec![
                fixture_file("Downloads/report.pdf", 300, bytes.clone()),
                fixture_file("Downloads/archive/report.pdf", 100, bytes.clone()),
                fixture_file("Work/report-final.pdf", 50, bytes),
            ],
        );
        let c = one(&candidates);
        assert_eq!(c.category, Category::Copies);
        assert_eq!(c.group.len(), 3);
        // Three quantities, and only one of them belongs in `size`:
        //   12288  the whole group's footprint  -> `group_bytes`
        //    8192  the two redundant copies     -> `size`
        //    4096  a single copy                -> neither
        // This asserted 4096 for a long time, which is right for a pair by
        // coincidence and an understatement for every larger set.
        assert_eq!(c.size, 8192, "size is every redundant copy, keeping one");
        assert_eq!(
            c.group_bytes, 12288,
            "the whole group is reported separately"
        );
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::ExactDuplicate { copies: 2 })));
    }

    #[test]
    fn the_newest_copy_is_the_suggested_keeper_and_the_oldest_is_proposed() {
        let h = harness();
        let bytes = payload(3, 2048);
        let candidates = h.run(
            DuplicateDetector::new(),
            vec![
                fixture_file("Downloads/old.bin", 400, bytes.clone()),
                fixture_file("Downloads/new.bin", 2, bytes),
            ],
        );
        let c = one(&candidates);
        let keeper = c.group.iter().find(|m| m.suggested_keep).unwrap();
        assert!(keeper.path.ends_with("new.bin"));
        assert!(
            c.path.ends_with("old.bin"),
            "the proposal should be the oldest copy, not the newest"
        );
    }

    #[test]
    fn same_size_but_different_contents_is_not_a_duplicate() {
        // This is the case the cheap size grouping gets wrong and the hash
        // has to rescue.
        let h = harness();
        let candidates = h.run(
            DuplicateDetector::new(),
            vec![
                fixture_file("Downloads/a.bin", 300, payload(1, 8192)),
                fixture_file("Downloads/b.bin", 300, payload(2, 8192)),
            ],
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn files_differing_only_in_the_middle_are_still_distinguished() {
        // The head/tail probe cannot see this difference; the full hash must.
        let h = harness();
        let mut a = vec![9u8; 300_000];
        let mut b = a.clone();
        a[150_000] = 1;
        b[150_000] = 2;
        let candidates = h.run(
            DuplicateDetector::new(),
            vec![
                fixture_file("Downloads/a.bin", 300, a),
                fixture_file("Downloads/b.bin", 300, b),
            ],
        );
        assert!(
            candidates.is_empty(),
            "the full hash must catch a middle difference"
        );
    }

    #[test]
    fn files_below_the_minimum_size_are_not_hashed() {
        let mut h = Harness::new();
        h.options.duplicate_min_size = 1024 * 1024;
        let bytes = payload(4, 256);
        let candidates = h.run(
            DuplicateDetector::new(),
            vec![
                fixture_file("Downloads/tiny-a.txt", 300, bytes.clone()),
                fixture_file("Downloads/tiny-b.txt", 300, bytes),
            ],
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn copies_an_application_maintains_for_itself_are_not_reported() {
        // Two versions of an IDE shipping the same JAR is true, unactionable
        // and useless — the user cannot remove either without breaking the
        // thing that put them there.
        let mut h = Harness::new();
        h.options.duplicate_min_size = 64;
        std::fs::create_dir_all(h.path("Library/Application Support")).unwrap();
        let bytes = payload(9, 4096);
        let candidates = h.run(
            DuplicateDetector::new(),
            vec![
                fixture_file(
                    "Library/Application Support/IDE2024/plugins/lib/thing.jar",
                    300,
                    bytes.clone(),
                ),
                fixture_file(
                    "Library/Application Support/IDE2025/plugins/lib/thing.jar",
                    300,
                    bytes,
                ),
            ],
        );
        assert!(candidates.is_empty(), "got {candidates:#?}");
    }

    #[test]
    fn the_users_own_copies_are_still_reported() {
        let h = harness();
        let bytes = payload(10, 4096);
        let candidates = h.run(
            DuplicateDetector::new(),
            vec![
                fixture_file("Downloads/report.pdf", 300, bytes.clone()),
                fixture_file("Work/report.pdf", 300, bytes),
            ],
        );
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn a_lone_file_is_never_a_copy() {
        let h = harness();
        let candidates = h.run(
            DuplicateDetector::new(),
            vec![fixture_file("Downloads/unique.bin", 300, payload(5, 4096))],
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn copies_in_user_territory_are_surfaced_not_recommended() {
        let h = harness();
        let bytes = payload(6, 4096);
        let candidates = h.run(
            DuplicateDetector::new(),
            vec![
                fixture_file("Pictures/holiday.jpg", 500, bytes.clone()),
                fixture_file("Pictures/backup/holiday.jpg", 500, bytes),
            ],
        );
        let c = one(&candidates);
        assert_eq!(c.risk, Risk::High);
        assert_eq!(c.recommended_action, RecommendedAction::InspectOnly);
    }

    #[test]
    fn the_remark_notices_whether_copies_are_scattered() {
        let h = harness();
        let bytes = payload(8, 4096);
        let together = h.run(
            DuplicateDetector::new(),
            vec![
                fixture_file("Downloads/x/a.bin", 300, bytes.clone()),
                fixture_file("Downloads/x/b.bin", 300, bytes.clone()),
            ],
        );
        assert!(one(&together)
            .remark
            .as_ref()
            .unwrap()
            .contains("same folder"));

        let h2 = harness();
        let scattered = h2.run(
            DuplicateDetector::new(),
            vec![
                fixture_file("Downloads/a.bin", 300, bytes.clone()),
                fixture_file("Other/b.bin", 300, bytes),
            ],
        );
        assert!(one(&scattered)
            .remark
            .as_ref()
            .unwrap()
            .contains("2 folders"));
    }

    #[test]
    fn hashing_a_missing_file_yields_nothing_rather_than_panicking() {
        assert!(full_hash(&PathBuf::from("/definitely/not/here")).is_none());
        assert!(probe_fingerprint(&PathBuf::from("/definitely/not/here"), 10).is_none());
    }
}
