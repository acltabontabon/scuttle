//! Ghosts: files belonging to software that is no longer here.
//!
//! "I uninstalled this six months ago, why is 8 GB still here?" is the
//! question this detector exists to answer. Answering it responsibly means
//! two things:
//!
//! * Attribution comes from the platform's own records — application bundles,
//!   the uninstall registry, Steam and Epic manifests — not from folder names
//!   that happen to contain a product name.
//! * Save data and mods are treated as irreplaceable. A game's shader cache
//!   and a game's saved campaign live side by side, and only one of them is
//!   disposable.
//!
//! When attribution fails entirely the finding goes to Oddments rather than
//! being dressed up as a confident conclusion.

use std::path::{Path, PathBuf};

use super::naming::display_name;
use crate::evidence::EvidenceKind;
use crate::model::{human_bytes, Category, Risk, TargetKind};
use crate::platform::apps::Attribution;
use crate::scanning::walk::FileEntry;
use crate::scanning::ContentProfile;
use crate::scanning::{CandidateSink, Detector, Finding, ScanContext};

/// Below this, a leftover folder is not worth anyone's attention.
const MIN_INTERESTING_BYTES: u64 = 20 * 1024 * 1024;
/// An unattributable folder has to be considerably larger before Scuttle will
/// mention it at all, because all it can say is "no idea".
const MIN_ODDMENT_BYTES: u64 = 200 * 1024 * 1024;
/// Recently touched data belongs to something that is still in use.
const MIN_IDLE_DAYS: u32 = 45;

pub struct GhostDetector {
    seen: Vec<PathBuf>,
}

impl GhostDetector {
    pub fn new() -> Self {
        GhostDetector { seen: Vec::new() }
    }

    /// Application data directories: one level of children, each attributed.
    fn examine_app_data(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {
        for root in ctx.platform.application_data_roots() {
            if ctx.cancelled() {
                return;
            }
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            for entry in entries.flatten() {
                if ctx.cancelled() {
                    return;
                }
                let path = entry.path();
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                // Links are never followed, here or anywhere else.
                if file_type.is_symlink() || !file_type.is_dir() {
                    continue;
                }
                if ctx.protected.is_protected(&path) || self.seen.contains(&path) {
                    continue;
                }
                // A cache with a rule is a cache, not an orphan. The cache
                // detector may be switched off for this scan; that is a reason
                // to stay quiet, not to reclassify it as something else.
                if ctx.cache_rule_owns(&path) {
                    continue;
                }
                self.seen.push(path.clone());

                let token = display_name(&path);
                match ctx.apps.attribute(&token) {
                    // Still installed: its data is its own business.
                    Attribution::Installed(_) => continue,
                    Attribution::Missing {
                        display,
                        identifier,
                    } => {
                        self.emit_ghost(ctx, sink, &path, &display, identifier.as_deref(), None);
                    }
                    Attribution::Unknown => {
                        self.emit_oddment(ctx, sink, &path, &token);
                    }
                }
            }
        }
    }

    /// Game libraries: folders sitting in an install root that the launcher
    /// has no manifest for.
    fn examine_game_libraries(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {
        for library in &ctx.libraries {
            for install_root in &library.install_roots {
                if ctx.cancelled() {
                    return;
                }
                let Ok(entries) = std::fs::read_dir(install_root) else {
                    continue;
                };
                for entry in entries.flatten() {
                    if ctx.cancelled() {
                        return;
                    }
                    let path = entry.path();
                    let Ok(file_type) = entry.file_type() else {
                        continue;
                    };
                    if file_type.is_symlink() || !file_type.is_dir() {
                        continue;
                    }
                    if ctx.protected.is_protected(&path) || self.seen.contains(&path) {
                        continue;
                    }
                    let folder = display_name(&path);
                    // The launcher still has this title. Not a ghost.
                    if library.has_install_dir(&folder) {
                        continue;
                    }
                    self.seen.push(path.clone());
                    self.emit_ghost(ctx, sink, &path, &folder, None, Some(library.kind.label()));
                }
            }
        }
    }

    /// Build the finding for something Scuttle can name but cannot find.
    fn emit_ghost(
        &self,
        ctx: &ScanContext,
        sink: &mut dyn CandidateSink,
        path: &Path,
        display: &str,
        identifier: Option<&str>,
        library: Option<&'static str>,
    ) {
        let profile = ctx.profile_of(path);
        if profile.bytes < MIN_INTERESTING_BYTES || profile.is_empty() {
            return;
        }

        let idle_days = idle_days(&profile, ctx.now_unix);
        if idle_days < MIN_IDLE_DAYS {
            return;
        }

        let entry = directory_entry(path, profile.bytes, profile.newest_unix);
        let mut finding = Finding::new("ghosts", Category::Ghosts, &entry, display)
            .app(display)
            .size(profile.bytes)
            // Generated application data is cheap to be wrong about; the
            // evidence below raises this when it should not be.
            .risk(Risk::Low)
            .classified();
        finding.target_kind = TargetKind::Directory;

        finding = match library {
            Some(library) => finding.with(EvidenceKind::GameNotInLibrary {
                app: display.to_string(),
                library: library.to_string(),
            }),
            None => finding.with(EvidenceKind::ApplicationNotInstalled {
                app: display.to_string(),
            }),
        };

        if let Some(id) = identifier {
            finding = finding.with(EvidenceKind::MatchesApplicationId { id: id.to_string() });
        }

        finding = finding.with(EvidenceKind::UntouchedFor { days: idle_days });

        if profile.looks_generated() {
            finding = finding.with(EvidenceKind::GeneratedContent {
                reason: profile.generated_reason(),
            });
        }

        // The careful part. Saved progress and mods are not leftovers.
        if profile.save_data > 0 {
            finding = finding.with(EvidenceKind::SaveDataDetected {
                reason: save_reason(&profile),
            });
        }
        if profile.authored > 0 {
            finding = finding.with(EvidenceKind::UserContentDetected {
                reason: format!("{} files in here look like your own work", profile.authored),
            });
        }

        match ctx.process_running(display) {
            Some(true) => {
                finding = finding.with(EvidenceKind::InUse {
                    by: display.to_string(),
                });
            }
            Some(false) => {
                finding = finding.with(EvidenceKind::NoProcessUsingIt);
            }
            // Could not tell. Claiming "nothing is using it" would be a lie.
            None => {}
        }

        sink.emit(finding.saying(ghost_remark(display, &profile, idle_days, library)));
    }

    /// Something big that Scuttle genuinely cannot identify.
    fn emit_oddment(
        &self,
        ctx: &ScanContext,
        sink: &mut dyn CandidateSink,
        path: &Path,
        token: &str,
    ) {
        let profile = ctx.profile_of(path);
        if profile.bytes < MIN_ODDMENT_BYTES || profile.is_empty() {
            return;
        }
        let idle_days = idle_days(&profile, ctx.now_unix);
        if idle_days < MIN_IDLE_DAYS {
            return;
        }

        let entry = directory_entry(path, profile.bytes, profile.newest_unix);
        let mut finding = Finding::new("ghosts", Category::Oddments, &entry, token)
            .size(profile.bytes)
            .risk(Risk::Moderate)
            .with(EvidenceKind::Unclassified {
                reason: "nothing installed claims this folder".into(),
            })
            .with(EvidenceKind::UntouchedFor { days: idle_days })
            .with(EvidenceKind::LargeSize {
                bytes: profile.bytes,
            });
        finding.target_kind = TargetKind::Directory;

        if profile.save_data > 0 || profile.authored > 0 {
            finding = finding.with(EvidenceKind::UserContentDetected {
                reason: "it holds files that look authored".into(),
            });
        }

        sink.emit(finding.saying(format!(
            "{}, untouched for {idle_days} days.\n\nScuttle has no idea what this is.\n\nYou decide.",
            human_bytes(profile.bytes)
        )));
    }
}

impl Default for GhostDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector for GhostDetector {
    fn id(&self) -> &'static str {
        "ghosts"
    }

    fn category(&self) -> Category {
        Category::Ghosts
    }

    fn rummaging_note(&self) -> &'static str {
        "Seeing what your games left behind"
    }

    fn probe(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {
        self.examine_game_libraries(ctx, sink);
        self.examine_app_data(ctx, sink);
    }
}

fn directory_entry(path: &Path, bytes: u64, newest_unix: i64) -> FileEntry {
    FileEntry::for_directory(path, bytes, newest_unix)
}

fn idle_days(profile: &ContentProfile, now_unix: i64) -> u32 {
    if profile.newest_unix <= 0 {
        return 0;
    }
    ((now_unix - profile.newest_unix).max(0) / 86_400) as u32
}

fn save_reason(profile: &ContentProfile) -> String {
    if profile.save_data == 1 {
        "one file in here looks like saved progress".to_string()
    } else {
        format!(
            "{} files in here look like saved progress",
            profile.save_data
        )
    }
}

fn ghost_remark(
    display: &str,
    profile: &ContentProfile,
    idle_days: u32,
    library: Option<&'static str>,
) -> String {
    let size = human_bytes(profile.bytes);
    if profile.save_data > 0 {
        return format!(
            "{display} is gone, but this is not only leftovers.\n\nThere is something in here that looks like saved progress. Scuttle will not suggest removing it."
        );
    }
    match library {
        Some(library) => format!(
            "{library} has no installation of {display}.\n\n{size} stayed behind, untouched for {idle_days} days."
        ),
        None => format!(
            "The app is gone.\n\nThis isn't.\n\n{size}, last touched {idle_days} days ago."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detectors::test_support::*;
    use crate::model::{Confidence, RecommendedAction};
    use crate::platform::games::{GameInstall, GameLibrary, LibraryKind};

    fn mb(n: u64) -> u64 {
        n * 1024 * 1024
    }

    /// A harness whose application-data root is `~/AppData`.
    fn app_data_harness(apps: &[(&str, Option<&str>)]) -> Harness {
        let h = Harness::with_apps(apps);
        std::fs::create_dir_all(h.path("AppData")).unwrap();
        h
    }

    #[test]
    fn leftovers_from_an_uninstalled_app_are_found() {
        let h = app_data_harness(&[("Figma", Some("com.figma.Desktop"))]);
        h.materialise(&[
            fixture_entry("AppData/com.sublimetext.4/Cache/a.cache", 300, mb(12)),
            fixture_entry("AppData/com.sublimetext.4/Cache/b.cache", 300, mb(12)),
            fixture_entry("AppData/com.sublimetext.4/logs/run.log", 300, mb(1)),
        ]);
        let candidates = h.run_bare(GhostDetector::new());
        let c = one(&candidates);
        assert_eq!(c.category, Category::Ghosts);
        assert_eq!(c.display_name, "com.sublimetext.4");
        assert_eq!(c.target_kind, TargetKind::Directory);
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::ApplicationNotInstalled { .. })));
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::MatchesApplicationId { .. })));
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::GeneratedContent { .. })));
        assert_eq!(c.confidence, Confidence::High);
        assert_eq!(c.recommended_action, RecommendedAction::Quarantine);
    }

    #[test]
    fn an_installed_apps_data_is_left_completely_alone() {
        let h = app_data_harness(&[("Figma", Some("com.figma.Desktop"))]);
        h.materialise(&[fixture_entry(
            "AppData/com.figma.Desktop/Cache/a.cache",
            400,
            mb(60),
        )]);
        let candidates = h.run_bare(GhostDetector::new());
        assert!(candidates.is_empty(), "got {candidates:#?}");
    }

    #[test]
    fn a_vendor_that_is_still_installed_shields_its_other_folders() {
        let h = app_data_harness(&[("Figma", Some("com.figma.Desktop"))]);
        h.materialise(&[fixture_entry(
            "AppData/com.figma.Agent/Cache/a.cache",
            400,
            mb(60),
        )]);
        let candidates = h.run_bare(GhostDetector::new());
        assert!(candidates.is_empty());
    }

    #[test]
    fn save_data_stops_scuttle_recommending_anything() {
        // The most important test in this file.
        let h = app_data_harness(&[]);
        h.materialise(&[
            fixture_entry("AppData/com.oldstudio.rpg/shadercache/a.bin", 500, mb(40)),
            fixture_entry("AppData/com.oldstudio.rpg/saves/slot1.sav", 500, mb(1)),
        ]);
        let candidates = h.run_bare(GhostDetector::new());
        let c = one(&candidates);
        assert_eq!(c.risk, Risk::High);
        assert_eq!(c.recommended_action, RecommendedAction::InspectOnly);
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::SaveDataDetected { .. })));
        assert!(c.remark.as_ref().unwrap().contains("saved progress"));
    }

    #[test]
    fn authored_files_inside_leftovers_raise_the_risk() {
        let h = app_data_harness(&[]);
        h.materialise(&[
            fixture_entry("AppData/com.oldtool.editor/cache/a.cache", 400, mb(40)),
            fixture_entry(
                "AppData/com.oldtool.editor/projects/thesis.docx",
                400,
                mb(2),
            ),
        ]);
        let c = only(h.run_bare(GhostDetector::new()));
        assert_eq!(c.risk, Risk::High);
        assert_eq!(c.recommended_action, RecommendedAction::InspectOnly);
    }

    #[test]
    fn recently_used_leftovers_are_not_leftovers() {
        let h = app_data_harness(&[]);
        h.materialise(&[fixture_entry(
            "AppData/com.something.tool/cache/a.cache",
            3,
            mb(80),
        )]);
        assert!(h.run_bare(GhostDetector::new()).is_empty());
    }

    #[test]
    fn small_leftovers_are_beneath_notice() {
        let h = app_data_harness(&[]);
        h.materialise(&[fixture_entry(
            "AppData/com.tiny.tool/cache/a.cache",
            400,
            1024,
        )]);
        assert!(h.run_bare(GhostDetector::new()).is_empty());
    }

    #[test]
    fn an_unattributable_folder_becomes_an_oddment_and_admits_it() {
        let h = app_data_harness(&[]);
        h.materialise(&[fixture_entry("AppData/tmp/blob.bin", 400, mb(260))]);
        let c = only(h.run_bare(GhostDetector::new()));
        assert_eq!(c.category, Category::Oddments);
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::Unclassified { .. })));
        assert!(c.remark.as_ref().unwrap().contains("no idea"));
        assert_ne!(c.recommended_action, RecommendedAction::Quarantine);
    }

    #[test]
    fn a_small_unidentifiable_folder_is_not_mentioned_at_all() {
        let h = app_data_harness(&[]);
        h.materialise(&[fixture_entry("AppData/tmp/blob.bin", 400, mb(30))]);
        assert!(
            h.run_bare(GhostDetector::new()).is_empty(),
            "saying 'no idea' about something small is just noise"
        );
    }

    #[test]
    fn a_known_cache_is_never_reclassified_as_a_ghost() {
        // The Go build cache has a rule, but that rule only fires when
        // developer debris is switched on. With it off, the ghost detector
        // must stay quiet rather than announce that "go-build is not
        // installed" — a wrong answer delivered more confidently.
        let mut h = app_data_harness(&[]);
        h.materialise(&[fixture_entry("AppData/go-build/aa/blob.bin", 400, mb(80))]);
        h.caches = vec![crate::platform::caches::CacheRule {
            owner: "Go",
            label: "Go build cache",
            path: h.path("AppData/go-build"),
            safety: crate::platform::caches::CacheSafety::Regenerates,
            owner_process: None,
            developer_only: true,
            settle_secs: 0,
        }];

        let candidates = h.run_bare(GhostDetector::new());
        assert!(
            candidates.is_empty(),
            "a path a cache rule owns must not become a ghost: {candidates:#?}"
        );
    }

    #[test]
    fn a_game_the_launcher_forgot_is_found() {
        let mut h = Harness::new();
        let common = h.path("Steam/steamapps/common");
        std::fs::create_dir_all(&common).unwrap();
        h.libraries = vec![GameLibrary {
            kind: LibraryKind::Steam,
            root: h.path("Steam"),
            install_roots: vec![common.clone()],
            generated_roots: vec![],
            installed: vec![GameInstall {
                name: "Hades".into(),
                id: "1145360".into(),
                install_dir: Some(common.join("Hades")),
            }],
        }];
        h.materialise(&[
            fixture_entry("Steam/steamapps/common/Hades/game.pak", 400, mb(50)),
            fixture_entry(
                "Steam/steamapps/common/Cyberpunk 2077/shadercache/a.bin",
                400,
                mb(80),
            ),
        ]);

        let candidates = h.run_bare(GhostDetector::new());
        let c = one(&candidates);
        assert_eq!(c.display_name, "Cyberpunk 2077");
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(&e.kind, EvidenceKind::GameNotInLibrary { library, .. } if library == "Steam")));
        assert!(c
            .remark
            .as_ref()
            .unwrap()
            .contains("Steam has no installation"));
    }

    #[test]
    fn a_running_process_protects_its_own_leftovers() {
        let mut h = app_data_harness(&[]);
        h.processes = vec!["com.oldtool.helper".into()];
        h.materialise(&[fixture_entry(
            "AppData/com.oldtool.helper/cache/a.cache",
            400,
            mb(80),
        )]);
        let c = only(h.run_bare(GhostDetector::new()));
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::InUse { .. })));
        assert_eq!(c.recommended_action, RecommendedAction::InspectOnly);
    }

    #[test]
    fn scuttle_does_not_claim_nothing_is_running_when_it_cannot_tell() {
        let h = app_data_harness(&[]); // no process list at all
        h.materialise(&[fixture_entry(
            "AppData/com.oldtool.thing/cache/a.cache",
            400,
            mb(80),
        )]);
        let c = only(h.run_bare(GhostDetector::new()));
        assert!(
            !c.evidence
                .iter()
                .any(|e| matches!(e.kind, EvidenceKind::NoProcessUsingIt)),
            "an empty process list means 'could not tell', not 'nothing is running'"
        );
    }
}
