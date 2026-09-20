//! Installers whose job is finished.
//!
//! The interesting signal is not age. It is that the thing the installer
//! installs is already installed, or that a newer copy of the same installer
//! is sitting right next to it.

use std::collections::HashMap;

use super::naming::{display_name, product_key};
use crate::evidence::EvidenceKind;
use crate::model::{human_bytes, Category, Risk};
use crate::platform::apps::Attribution;
use crate::scanning::{CandidateSink, Detector, FileEntry, Finding, ScanContext};

pub struct InstallerDetector {
    /// Everything that looked like an installer, grouped by product.
    by_product: HashMap<String, Vec<FileEntry>>,
}

impl InstallerDetector {
    pub fn new() -> Self {
        InstallerDetector {
            by_product: HashMap::new(),
        }
    }
}

impl Default for InstallerDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector for InstallerDetector {
    fn id(&self) -> &'static str {
        "installers"
    }

    fn category(&self) -> Category {
        Category::Installers
    }

    fn rummaging_note(&self) -> &'static str {
        "Checking what you already installed"
    }

    fn observe(&mut self, entry: &FileEntry, ctx: &ScanContext) {
        if entry.is_dir {
            return;
        }
        let Some(ext) = entry.extension() else { return };
        if !ctx.platform.installer_extensions().contains(&ext) {
            return;
        }
        // A 40 KB ".exe" is a helper tool, not an installer.
        if entry.size < 1024 * 512 {
            return;
        }
        let stem = entry
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.by_product
            .entry(product_key(&stem))
            .or_default()
            .push(entry.clone());
    }

    fn finish(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {
        for (product, mut entries) in std::mem::take(&mut self.by_product) {
            if ctx.cancelled() {
                return;
            }
            // Newest first, so "the newer one" is always entries[0].
            entries.sort_by_key(|e| std::cmp::Reverse(e.modified_unix.unwrap_or(0)));

            let installed_app = match ctx.apps.attribute(&product) {
                Attribution::Installed(app) => Some(app.name.clone()),
                _ => None,
            };

            let copies = entries.len();
            let newest_name = display_name(&entries[0].path);

            for (index, entry) in entries.iter().enumerate() {
                let mut finding = Finding::new(
                    "installers",
                    Category::Installers,
                    entry,
                    display_name(&entry.path),
                )
                // Installers are the archetypal low-risk finding: if
                // Scuttle is wrong, the cost is a download.
                .risk(Risk::Low)
                .with(EvidenceKind::InstallerFormat {
                    ext: entry.extension().unwrap_or_default().to_string(),
                });

                if let Some(app) = &installed_app {
                    finding = finding
                        .app(app.clone())
                        .with(EvidenceKind::InstalledAppSupersedes { app: app.clone() });
                }

                if index > 0 {
                    finding = finding.with(EvidenceKind::NewerInstallerPresent {
                        newer: newest_name.clone(),
                    });
                }

                if let Some(days) = entry.idle_days(ctx.now_unix) {
                    if days >= 30 {
                        finding = finding.with(EvidenceKind::UntouchedFor { days });
                    }
                }

                finding = finding.saying(remark(
                    installed_app.as_deref(),
                    copies,
                    index,
                    entry,
                    ctx.now_unix,
                ));

                sink.emit(finding);
            }
        }
    }
}

/// The one line Scuttle says about an installer. Derived from the same facts
/// as the evidence; never decorative.
fn remark(
    installed_app: Option<&str>,
    copies: usize,
    index: usize,
    entry: &FileEntry,
    now_unix: i64,
) -> String {
    if copies >= 5 && index == 0 {
        return format!("You downloaded this {copies} times.\n\nScuttle counted twice to be sure.");
    }
    if copies >= 2 && index > 0 {
        return "There is a newer copy of this next to it.".to_string();
    }
    if let Some(app) = installed_app {
        return match entry.age_days(now_unix) {
            Some(days) if days >= 30 => {
                format!("{app} is already installed.\n\nThis arrived {days} days ago.")
            }
            _ => format!("{app} is already installed."),
        };
    }
    match entry.idle_days(now_unix) {
        Some(days) if days >= 180 => format!(
            "{} of installer, untouched for {days} days.",
            human_bytes(entry.size)
        ),
        _ => "An installer. Probably done with this.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detectors::test_support::*;
    use crate::model::{Confidence, RecommendedAction};

    fn installer(name: &str, days_old: i64, mb: u64) -> FixtureFile {
        fixture_entry(name, days_old, mb * 1024 * 1024)
    }

    #[test]
    fn an_installer_for_an_installed_app_is_high_confidence_and_low_risk() {
        let h = Harness::with_apps(&[("Google Chrome", Some("com.google.Chrome"))]);
        let candidates = h.run(
            InstallerDetector::new(),
            vec![installer("Downloads/Google Chrome.dmg", 200, 220)],
        );
        let c = one(&candidates);
        assert_eq!(c.category, Category::Installers);
        assert_eq!(c.risk, Risk::Low);
        assert_eq!(c.confidence, Confidence::High);
        assert_eq!(c.recommended_action, RecommendedAction::Quarantine);
        assert!(c.remark.as_ref().unwrap().contains("already installed"));
    }

    #[test]
    fn an_installer_for_something_absent_is_not_recommended_for_cleanup() {
        // Scuttle has no idea whether you still need this.
        let h = Harness::with_apps(&[]);
        let candidates = h.run(
            InstallerDetector::new(),
            vec![installer("Downloads/ObscureTool.dmg", 40, 60)],
        );
        let c = one(&candidates);
        assert_ne!(c.recommended_action, RecommendedAction::Quarantine);
    }

    #[test]
    fn five_copies_of_the_same_thing_get_noticed() {
        let h = Harness::with_apps(&[]);
        let candidates = h.run(
            InstallerDetector::new(),
            vec![
                installer("Downloads/Chrome.dmg", 300, 200),
                installer("Downloads/Chrome (1).dmg", 250, 200),
                installer("Downloads/Chrome (2).dmg", 200, 200),
                installer("Downloads/Chrome (3).dmg", 150, 200),
                installer("Downloads/Chrome (4).dmg", 100, 200),
            ],
        );
        assert_eq!(candidates.len(), 5);
        let newest = candidates
            .iter()
            .find(|c| c.remark.as_deref().is_some_and(|r| r.contains("5 times")))
            .expect("Scuttle should count the copies");
        assert!(newest.remark.as_ref().unwrap().contains("counted twice"));

        let older: Vec<_> = candidates
            .iter()
            .filter(|c| {
                c.evidence
                    .iter()
                    .any(|e| matches!(e.kind, EvidenceKind::NewerInstallerPresent { .. }))
            })
            .collect();
        assert_eq!(
            older.len(),
            4,
            "every copy but the newest has a newer sibling"
        );
    }

    #[test]
    fn small_executables_are_not_treated_as_installers() {
        let h = Harness::with_apps(&[]);
        let candidates = h.run(
            InstallerDetector::new(),
            vec![fixture_entry("Downloads/helper.dmg", 400, 4096)],
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn non_installer_extensions_are_ignored() {
        let h = Harness::with_apps(&[]);
        let candidates = h.run(
            InstallerDetector::new(),
            vec![
                installer("Downloads/notes.txt", 400, 20),
                installer("Downloads/movie.mp4", 400, 900),
            ],
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn age_is_reported_only_once_it_means_something() {
        let h = Harness::with_apps(&[]);
        let fresh = h.run(
            InstallerDetector::new(),
            vec![installer("Downloads/Thing.dmg", 3, 100)],
        );
        assert!(!one(&fresh)
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::UntouchedFor { .. })));

        let stale = h.run(
            InstallerDetector::new(),
            vec![installer("Downloads/Thing.dmg", 400, 100)],
        );
        assert!(one(&stale)
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::UntouchedFor { .. })));
    }
}
