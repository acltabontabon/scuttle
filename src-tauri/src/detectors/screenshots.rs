//! Forgotten screenshots.
//!
//! The useful discovery here is rarely a single old capture. It is the
//! eighteen near-identical attempts at the same thing, taken in one
//! increasingly frustrated afternoon.
//!
//! Identification uses several independent signals — where the file lives,
//! what it is called, and what it looks like — because any one of them alone
//! produces false positives on ordinary photographs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use super::naming::display_name;
use crate::evidence::EvidenceKind;
use crate::model::{human_bytes, Category, GroupMember, Risk};
use crate::safety::paths;
use crate::scanning::{CandidateSink, Detector, FileEntry, Finding, ScanContext};

/// Filename fragments used by screenshot tools, lowercased. Localised macOS
/// and Windows names are included because people do not all run English
/// systems.
const NAME_PATTERNS: [&str; 12] = [
    "screenshot",
    "screen shot",
    "screen_shot",
    "screencapture",
    "cleanshot",
    "bildschirmfoto",
    "captura de pantalla",
    "capture d",
    "snimok",
    "snip",
    "shottr",
    "screen recording",
];

const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "gif", "bmp", "tiff", "webp"];

/// Decoding images is the expensive part of this detector, so it is bounded.
const MAX_DECODES: usize = 4000;
/// Images larger than this are not screenshots worth comparing visually.
const MAX_DECODE_BYTES: u64 = 40 * 1024 * 1024;
/// Hamming distance at or below which two captures count as near-identical.
const NEAR_DUPLICATE_DISTANCE: u32 = 6;
/// A burst has to be at least this big before it is worth remarking on.
const BURST_SIZE: usize = 3;
/// A lone screenshot is only interesting once it has been ignored this long.
const STALE_DAYS: u32 = 180;

pub struct ScreenshotDetector {
    candidates: Vec<Shot>,
    locations: Option<Vec<PathBuf>>,
}

#[derive(Clone)]
struct Shot {
    entry: FileEntry,
    /// The pattern that matched the name, if any.
    matched_pattern: Option<String>,
    in_screenshot_location: bool,
}

impl ScreenshotDetector {
    pub fn new() -> Self {
        ScreenshotDetector {
            candidates: Vec::new(),
            locations: None,
        }
    }

    fn locations<'a>(&'a mut self, ctx: &ScanContext) -> &'a [PathBuf] {
        self.locations
            .get_or_insert_with(|| ctx.platform.screenshot_locations())
    }
}

impl Default for ScreenshotDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector for ScreenshotDetector {
    fn id(&self) -> &'static str {
        "screenshots"
    }

    fn category(&self) -> Category {
        Category::Screenshots
    }

    fn rummaging_note(&self) -> &'static str {
        "Checking screenshots"
    }

    fn observe(&mut self, entry: &FileEntry, ctx: &ScanContext) {
        if entry.is_dir {
            return;
        }
        let Some(ext) = entry.extension() else { return };
        if !IMAGE_EXTENSIONS.contains(&ext) {
            return;
        }

        let name = entry.name_lower();
        let matched_pattern = NAME_PATTERNS
            .iter()
            .find(|pattern| name.contains(*pattern))
            .map(|p| (*p).to_string());

        let in_screenshot_location = {
            let locations = self.locations(ctx);
            locations
                .iter()
                .any(|dir| paths::is_within(&entry.path, dir))
        };

        // One signal alone is not enough: a holiday photo in Pictures is not a
        // screenshot, and a file called `screenshot-of-the-menu.png` in a
        // design project probably is one but is also someone's working asset.
        if matched_pattern.is_none() && !in_screenshot_location {
            return;
        }

        self.candidates.push(Shot {
            entry: entry.clone(),
            matched_pattern,
            in_screenshot_location,
        });
    }

    fn finish(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {
        let shots = std::mem::take(&mut self.candidates);
        if shots.is_empty() || ctx.cancelled() {
            return;
        }

        // Visual fingerprints, computed in parallel and bounded.
        let decodable: Vec<&Shot> = shots
            .iter()
            .filter(|s| s.entry.size <= MAX_DECODE_BYTES)
            .take(MAX_DECODES)
            .collect();

        let fingerprints: HashMap<PathBuf, Fingerprint> = decodable
            .par_iter()
            .filter_map(|shot| fingerprint(&shot.entry.path).map(|f| (shot.entry.path.clone(), f)))
            .collect();

        if ctx.cancelled() {
            return;
        }

        let groups = group_by_similarity(&shots, &fingerprints);
        let mut grouped: Vec<PathBuf> = Vec::new();

        for group in &groups {
            if group.len() < BURST_SIZE {
                continue;
            }
            for shot in group {
                grouped.push(shot.entry.path.clone());
            }
            emit_burst(group, &fingerprints, ctx, sink);
        }

        // Screenshots that are not part of a burst are only interesting once
        // they have been sitting unlooked-at for a long time.
        for shot in &shots {
            if ctx.cancelled() {
                return;
            }
            if grouped.contains(&shot.entry.path) {
                continue;
            }
            let Some(days) = shot.entry.idle_days(ctx.now_unix) else {
                continue;
            };
            if days < STALE_DAYS {
                continue;
            }
            let mut finding = Finding::new(
                "screenshots",
                Category::Screenshots,
                &shot.entry,
                display_name(&shot.entry.path),
            )
            .risk(Risk::Moderate)
            .classified()
            .with(EvidenceKind::UntouchedFor { days });

            finding = add_identification(finding, shot, &fingerprints);
            sink.emit(finding.saying(format!(
                "A screenshot from {days} days ago.\n\nWhatever it was for, it is over now."
            )));
        }
    }
}

/// Attach the evidence that says "this is a screenshot".
fn add_identification(
    mut finding: Finding,
    shot: &Shot,
    fingerprints: &HashMap<PathBuf, Fingerprint>,
) -> Finding {
    if let Some(pattern) = &shot.matched_pattern {
        finding = finding.with(EvidenceKind::ScreenshotNamePattern {
            pattern: pattern.clone(),
        });
    }
    if shot.in_screenshot_location {
        if let Some(folder) = shot.entry.path.parent().and_then(|p| p.file_name()) {
            finding = finding.with(EvidenceKind::InKnownScreenshotFolder {
                folder: folder.to_string_lossy().into_owned(),
            });
        }
    }
    if let Some(fp) = fingerprints.get(&shot.entry.path) {
        if looks_like_a_display(fp.width, fp.height) {
            finding = finding.with(EvidenceKind::ScreenshotDimensions {
                width: fp.width,
                height: fp.height,
            });
        }
    }
    finding
}

fn emit_burst(
    group: &[&Shot],
    fingerprints: &HashMap<PathBuf, Fingerprint>,
    ctx: &ScanContext,
    sink: &mut dyn CandidateSink,
) {
    let mut sorted: Vec<&Shot> = group.to_vec();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.entry.modified_unix.unwrap_or(0)));
    let newest = sorted[0];
    let oldest = sorted[sorted.len() - 1];

    let members: Vec<GroupMember> = sorted
        .iter()
        .map(|s| GroupMember {
            path: s.entry.path.clone(),
            size: s.entry.size,
            modified_unix: s.entry.modified_unix,
            suggested_keep: s.entry.path == newest.entry.path,
        })
        .collect();

    let total: u64 = sorted.iter().map(|s| s.entry.size).sum();
    let reclaimable = total.saturating_sub(newest.entry.size);

    let mut finding = Finding::new(
        "screenshots",
        Category::Screenshots,
        &oldest.entry,
        display_name(&oldest.entry.path),
    )
    .risk(Risk::Moderate)
    .classified()
    .size(reclaimable)
    .with(EvidenceKind::NearDuplicateImage {
        group_size: sorted.len() as u32,
    });

    finding = add_identification(finding, oldest, fingerprints);

    if let Some(days) = oldest.entry.idle_days(ctx.now_unix) {
        if days >= 90 {
            finding = finding.with(EvidenceKind::UntouchedFor { days });
        }
    }

    finding.group = members;
    sink.emit(finding.saying(burst_remark(sorted.len(), reclaimable, &sorted)));
}

fn burst_remark(count: usize, reclaimable: u64, shots: &[&Shot]) -> String {
    let same_day = shots
        .first()
        .zip(shots.last())
        .and_then(|(a, b)| Some((a.entry.modified_unix?, b.entry.modified_unix?)))
        .map(|(newest, oldest)| (newest - oldest).abs() < 86_400)
        .unwrap_or(false);

    if count >= 12 && same_day {
        format!("{count} screenshots.\n\nAll the same thing.\n\nRough afternoon?")
    } else if same_day {
        format!(
            "{count} versions of this, all from one sitting.\n\n{} of near-identical pixels.",
            human_bytes(reclaimable)
        )
    } else {
        format!(
            "{count} screenshots that look nearly identical.\n\n{} of repetition.",
            human_bytes(reclaimable)
        )
    }
}

/// Union-find over visual similarity. O(n^2) in the number of screenshots,
/// which is fine at the scale this detector is bounded to.
fn group_by_similarity<'a>(
    shots: &'a [Shot],
    fingerprints: &HashMap<PathBuf, Fingerprint>,
) -> Vec<Vec<&'a Shot>> {
    let with_hash: Vec<(&Shot, u64)> = shots
        .iter()
        .filter_map(|s| fingerprints.get(&s.entry.path).map(|f| (s, f.hash)))
        .collect();

    let mut parent: Vec<usize> = (0..with_hash.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }

    for (i, (_, left)) in with_hash.iter().enumerate() {
        for (j, (_, right)) in with_hash.iter().enumerate().skip(i + 1) {
            if (left ^ right).count_ones() <= NEAR_DUPLICATE_DISTANCE {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }

    let mut buckets: HashMap<usize, Vec<&Shot>> = HashMap::new();
    for (i, (shot, _)) in with_hash.iter().enumerate() {
        let root = find(&mut parent, i);
        buckets.entry(root).or_default().push(shot);
    }
    buckets.into_values().filter(|g| g.len() > 1).collect()
}

#[derive(Debug, Clone, Copy)]
struct Fingerprint {
    /// Difference hash: each bit compares a pixel with its right-hand
    /// neighbour, which makes it robust to scaling and small edits.
    hash: u64,
    width: u32,
    height: u32,
}

fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let image = image::open(path).ok()?;
    let (width, height) = (
        image::GenericImageView::dimensions(&image).0,
        image::GenericImageView::dimensions(&image).1,
    );
    let small = image
        .resize_exact(9, 8, image::imageops::FilterType::Triangle)
        .to_luma8();

    let mut hash = 0u64;
    for y in 0..8u32 {
        for x in 0..8u32 {
            let left = small.get_pixel(x, y)[0];
            let right = small.get_pixel(x + 1, y)[0];
            hash = (hash << 1) | u64::from(left < right);
        }
    }
    Some(Fingerprint {
        hash,
        width,
        height,
    })
}

/// Common display resolutions, including the doubled sizes Retina captures
/// produce. Used as corroboration, never on its own.
fn looks_like_a_display(width: u32, height: u32) -> bool {
    const SIZES: [(u32, u32); 14] = [
        (1280, 800),
        (1440, 900),
        (1512, 982),
        (1680, 1050),
        (1920, 1080),
        (1920, 1200),
        (2056, 1329),
        (2560, 1440),
        (2560, 1600),
        (2880, 1800),
        (3024, 1964),
        (3456, 2234),
        (3840, 2160),
        (5120, 2880),
    ];
    SIZES
        .iter()
        .any(|(w, h)| (width == *w && height == *h) || (width == w * 2 && height == h * 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detectors::test_support::*;
    use crate::model::RecommendedAction;

    /// A tiny PNG with a controllable pattern, so visual similarity is
    /// something the test can actually steer.
    fn png(seed: u8, noise: u8) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        let image = image::ImageBuffer::from_fn(32, 32, |x, y| {
            let base = seed
                .wrapping_add((x as u8).wrapping_mul(7))
                .wrapping_add((y as u8).wrapping_mul(3));
            let v = if x < 4 && y < 4 {
                base.wrapping_add(noise)
            } else {
                base
            };
            image::Rgb([v, v, v])
        });
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();
        buffer.into_inner()
    }

    #[test]
    fn a_burst_of_near_identical_captures_becomes_one_finding() {
        let h = Harness::new();
        let files: Vec<FixtureFile> = (0..18)
            .map(|i| {
                fixture_file(
                    &format!("Desktop/Screenshot 2024-03-0{}.png", i % 9),
                    200,
                    png(40, i as u8 % 3),
                )
            })
            .collect();
        let candidates = h.run(ScreenshotDetector::new(), files);
        let c = one(&candidates);
        assert_eq!(c.category, Category::Screenshots);
        assert!(c.group.len() >= BURST_SIZE);
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::NearDuplicateImage { .. })));
        assert!(c.group.iter().filter(|m| m.suggested_keep).count() == 1);
    }

    #[test]
    fn visually_different_captures_are_not_grouped_together() {
        let h = Harness::new();
        let candidates = h.run(
            ScreenshotDetector::new(),
            vec![
                fixture_file("Desktop/Screenshot one.png", 400, png(10, 0)),
                fixture_file("Desktop/Screenshot two.png", 400, png(200, 0)),
            ],
        );
        assert!(
            candidates.iter().all(|c| c.group.is_empty()),
            "two unrelated captures must not be called a burst"
        );
    }

    #[test]
    fn an_ordinary_photo_outside_a_screenshot_folder_is_ignored() {
        let h = Harness::new();
        let candidates = h.run(
            ScreenshotDetector::new(),
            vec![fixture_file("Work/assets/hero-image.png", 600, png(77, 0))],
        );
        assert!(candidates.is_empty(), "one weak signal must not be enough");
    }

    #[test]
    fn a_recent_lone_screenshot_is_left_alone() {
        let h = Harness::new();
        let candidates = h.run(
            ScreenshotDetector::new(),
            vec![fixture_file("Desktop/Screenshot today.png", 2, png(30, 0))],
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn an_old_lone_screenshot_is_surfaced_with_its_age() {
        let h = Harness::new();
        let candidates = h.run(
            ScreenshotDetector::new(),
            vec![fixture_file("Desktop/Screenshot old.png", 400, png(30, 0))],
        );
        let c = one(&candidates);
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::UntouchedFor { days } if days >= STALE_DAYS)));
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::ScreenshotNamePattern { .. })));
    }

    #[test]
    fn screenshots_are_never_recommended_for_automatic_removal() {
        let h = Harness::new();
        let files: Vec<FixtureFile> = (0..20)
            .map(|i| {
                fixture_file(
                    &format!("Desktop/Screenshot {i}.png"),
                    400,
                    png(50, i as u8 % 2),
                )
            })
            .collect();
        let candidates = h.run(ScreenshotDetector::new(), files);
        for c in &candidates {
            assert_ne!(
                c.recommended_action,
                RecommendedAction::Quarantine,
                "screenshots must never be proposed for automatic cleanup"
            );
        }
    }

    #[test]
    fn a_burst_reports_the_saving_of_the_redundant_copies_only() {
        let h = Harness::new();
        let bytes = png(60, 0);
        let each = bytes.len() as u64;
        let files: Vec<FixtureFile> = (0..4)
            .map(|i| fixture_file(&format!("Desktop/Screenshot {i}.png"), 300, bytes.clone()))
            .collect();
        let candidates = h.run(ScreenshotDetector::new(), files);
        let c = one(&candidates);
        assert_eq!(c.size, each * 3, "keeping one copy is the assumption");
    }

    #[test]
    fn living_in_a_capture_folder_counts_as_corroboration() {
        let h = Harness::new();
        let candidates = h.run(
            ScreenshotDetector::new(),
            vec![fixture_file(
                "Pictures/Screenshots/img-0042.png",
                400,
                png(120, 0),
            )],
        );
        let c = one(&candidates);
        assert!(
            c.evidence
                .iter()
                .any(|e| matches!(e.kind, EvidenceKind::InKnownScreenshotFolder { .. })),
            "a file with no telltale name is still a capture if it lives where captures land"
        );
    }

    #[test]
    fn display_sized_images_are_recognised_including_retina() {
        assert!(looks_like_a_display(1920, 1080));
        assert!(looks_like_a_display(3024, 1964));
        assert!(looks_like_a_display(2880, 1800));
        assert!(
            looks_like_a_display(3840, 2400),
            "a doubled 1920x1200 capture"
        );
        assert!(!looks_like_a_display(4032, 3024), "a phone camera photo");
    }

    #[test]
    fn a_corrupt_image_does_not_stop_the_detector() {
        let h = Harness::new();
        let candidates = h.run(
            ScreenshotDetector::new(),
            vec![
                fixture_file(
                    "Desktop/Screenshot broken.png",
                    400,
                    b"not really a png".to_vec(),
                ),
                fixture_file("Desktop/Screenshot fine.png", 400, png(90, 0)),
            ],
        );
        // The unreadable one still has a name and an age, so it is still a
        // finding; it simply has no visual evidence.
        assert!(candidates
            .iter()
            .any(|c| c.display_name == "Screenshot fine.png"));
    }

    #[test]
    fn the_burst_remark_notices_a_single_sitting() {
        let h = Harness::new();
        let files: Vec<FixtureFile> = (0..14)
            .map(|i| {
                fixture_file(
                    &format!("Desktop/Screenshot {i}.png"),
                    300,
                    png(70, i as u8 % 2),
                )
            })
            .collect();
        let candidates = h.run(ScreenshotDetector::new(), files);
        let remark = one(&candidates).remark.clone().unwrap();
        assert!(remark.contains("Rough afternoon"), "got: {remark}");
    }
}
