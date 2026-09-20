//! Developer debris. Opt-in, and conservative even then.
//!
//! A directory being called `target`, `dist` or `build` proves nothing — there
//! are photographers with a folder called `build` and writers with a folder
//! called `dist`. Scuttle requires a project manifest beside the directory
//! before it will believe the name, and it refuses to recommend anything for a
//! project that has been worked on recently.

use std::collections::HashMap;
use std::path::PathBuf;

use super::naming::display_name;
use crate::evidence::EvidenceKind;
use crate::model::{human_bytes, Category, Risk, TargetKind};
use crate::scanning::walk::FileEntry;
use crate::scanning::{CandidateSink, Detector, Finding, ScanContext};

/// `(directory name, the manifests that make it believable)`.
const BUILD_DIRECTORIES: [(&str, &[&str]); 11] = [
    ("node_modules", &["package.json"]),
    ("target", &["Cargo.toml"]),
    (
        "build",
        &[
            "build.gradle",
            "build.gradle.kts",
            "CMakeLists.txt",
            "pom.xml",
        ],
    ),
    ("dist", &["package.json", "pyproject.toml", "setup.py"]),
    (
        ".next",
        &["package.json", "next.config.js", "next.config.mjs"],
    ),
    (
        ".nuxt",
        &["package.json", "nuxt.config.ts", "nuxt.config.js"],
    ),
    (".parcel-cache", &["package.json"]),
    ("obj", &["*.csproj", "*.sln"]),
    ("Pods", &["Podfile"]),
    (".venv", &["pyproject.toml", "requirements.txt", "setup.py"]),
    ("vendor", &["composer.json", "go.mod", "Gemfile"]),
];

/// A project touched this recently is work in progress.
const ACTIVE_PROJECT_DAYS: u32 = 60;
const MIN_INTERESTING_BYTES: u64 = 100 * 1024 * 1024;

pub struct DeveloperDebrisDetector {
    /// Candidate directory -> the manifest that vouched for it.
    found: HashMap<PathBuf, &'static str>,
}

impl DeveloperDebrisDetector {
    pub fn new() -> Self {
        DeveloperDebrisDetector {
            found: HashMap::new(),
        }
    }
}

impl Default for DeveloperDebrisDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector for DeveloperDebrisDetector {
    fn id(&self) -> &'static str {
        "developer_debris"
    }

    fn category(&self) -> Category {
        Category::DeveloperDebris
    }

    fn rummaging_note(&self) -> &'static str {
        "Looking at what your build tools left"
    }

    fn enabled(&self, ctx: &ScanContext) -> bool {
        ctx.options.include_developer_debris
    }

    fn observe(&mut self, entry: &FileEntry, _ctx: &ScanContext) {
        if !entry.is_dir {
            return;
        }
        let name = entry.name_lower();
        let Some((_, manifests)) = BUILD_DIRECTORIES
            .iter()
            .find(|(candidate, _)| *candidate == name)
        else {
            return;
        };
        let Some(parent) = entry.path.parent() else {
            return;
        };

        // The name alone is never enough.
        if let Some(manifest) = find_manifest(parent, manifests) {
            self.found.insert(entry.path.clone(), manifest);
        }
    }

    fn finish(&mut self, ctx: &ScanContext, sink: &mut dyn CandidateSink) {
        for (path, manifest) in std::mem::take(&mut self.found) {
            if ctx.cancelled() {
                return;
            }
            let Some(project) = path.parent().map(std::path::Path::to_path_buf) else {
                continue;
            };
            let profile = ctx.profile_of(&path);
            if profile.bytes < MIN_INTERESTING_BYTES {
                continue;
            }

            let project_idle = project_idle_days(&project, ctx.now_unix);

            let entry = FileEntry::for_directory(&path, profile.bytes, profile.newest_unix);

            let mut finding = Finding::new(
                "developer_debris",
                Category::DeveloperDebris,
                &entry,
                display_name(&path),
            )
            .size(profile.bytes)
            .risk(Risk::Moderate)
            .classified()
            .with(EvidenceKind::BuildOutputOfProject {
                manifest: manifest.to_string(),
            });
            finding.target_kind = TargetKind::Directory;

            match project_idle {
                Some(days) if days < ACTIVE_PROJECT_DAYS => {
                    // Someone is working here. Surface it, do not suggest it.
                    finding = finding.with(EvidenceKind::ProjectRecentlyActive { days });
                }
                Some(days) => {
                    finding = finding.with(EvidenceKind::UntouchedFor { days });
                }
                None => {}
            }

            if is_inside_repository(&project) {
                finding = finding.with(EvidenceKind::InsideRepository);
            }

            sink.emit(finding.saying(remark(&display_name(&project), profile.bytes, project_idle)));
        }
    }
}

/// Is one of these manifests sitting next to the build directory?
fn find_manifest(project: &std::path::Path, manifests: &[&'static str]) -> Option<&'static str> {
    for manifest in manifests {
        if let Some(suffix) = manifest.strip_prefix('*') {
            // Glob forms such as `*.csproj`.
            let Ok(entries) = std::fs::read_dir(project) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().ends_with(suffix) {
                    return Some(manifest);
                }
            }
        } else if project.join(manifest).is_file() {
            return Some(manifest);
        }
    }
    None
}

/// The most recent modification among the project's own files, ignoring the
/// build directory itself.
fn project_idle_days(project: &std::path::Path, now_unix: i64) -> Option<u32> {
    let mut newest = 0i64;
    for entry in std::fs::read_dir(project).ok()?.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            continue;
        }
        if let Ok(modified) = meta.modified() {
            if let Ok(d) = modified.duration_since(std::time::UNIX_EPOCH) {
                newest = newest.max(d.as_secs() as i64);
            }
        }
    }
    if newest == 0 {
        return None;
    }
    Some(((now_unix - newest).max(0) / 86_400) as u32)
}

fn is_inside_repository(project: &std::path::Path) -> bool {
    project.ancestors().take(6).any(|a| a.join(".git").exists())
}

fn remark(project: &str, bytes: u64, project_idle: Option<u32>) -> String {
    let size = human_bytes(bytes);
    match project_idle {
        Some(days) if days < ACTIVE_PROJECT_DAYS => format!(
            "{size}.\n\nYou were working on {project} {days} days ago, so Scuttle is only pointing at it."
        ),
        Some(days) => format!(
            "{size} of build output.\n\n{project} has not been touched in {days} days. Your tools can rebuild this."
        ),
        None => format!("{size} of build output for {project}."),
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
        h.options.include_developer_debris = true;
        h
    }

    #[test]
    fn a_name_alone_is_never_enough() {
        // A photographer's `build` folder with no manifest anywhere near it.
        let h = harness();
        let candidates = h.run(
            DeveloperDebrisDetector::new(),
            vec![fixture_entry("Photos/build/shoot.raw", 400, mb(300))],
        );
        assert!(
            candidates.is_empty(),
            "a folder called build is not evidence of a build"
        );
    }

    #[test]
    fn a_stale_project_with_a_manifest_is_surfaced() {
        let h = harness();
        let candidates = h.run(
            DeveloperDebrisDetector::new(),
            vec![
                fixture_entry("code/thing/Cargo.toml", 500, 200),
                fixture_entry("code/thing/target/debug/binary", 500, mb(250)),
            ],
        );
        let c = one(&candidates);
        assert_eq!(c.category, Category::DeveloperDebris);
        assert_eq!(c.display_name, "target");
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(&e.kind, EvidenceKind::BuildOutputOfProject { manifest } if manifest == "Cargo.toml")));
        assert!(c.remark.as_ref().unwrap().contains("rebuild this"));
    }

    #[test]
    fn an_active_project_is_pointed_at_not_recommended() {
        let h = harness();
        let candidates = h.run(
            DeveloperDebrisDetector::new(),
            vec![
                fixture_entry("code/live/package.json", 1, 200),
                fixture_entry("code/live/node_modules/dep/index.js", 1, mb(150)),
            ],
        );
        let c = one(&candidates);
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::ProjectRecentlyActive { .. })));
        assert_eq!(c.recommended_action, RecommendedAction::InspectOnly);
        assert!(c.remark.as_ref().unwrap().contains("only pointing at it"));
    }

    #[test]
    fn the_detector_is_off_unless_asked_for() {
        let mut h = Harness::new();
        h.options.include_developer_debris = false;
        let candidates = h.run(
            DeveloperDebrisDetector::new(),
            vec![
                fixture_entry("code/thing/Cargo.toml", 500, 200),
                fixture_entry("code/thing/target/debug/binary", 500, mb(250)),
            ],
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn small_build_output_is_not_worth_mentioning() {
        let h = harness();
        let candidates = h.run(
            DeveloperDebrisDetector::new(),
            vec![
                fixture_entry("code/thing/Cargo.toml", 500, 200),
                fixture_entry("code/thing/target/debug/binary", 500, mb(2)),
            ],
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn glob_manifests_are_matched() {
        let h = harness();
        let candidates = h.run(
            DeveloperDebrisDetector::new(),
            vec![
                fixture_entry("code/app/App.csproj", 500, 200),
                fixture_entry("code/app/obj/Debug/app.dll", 500, mb(150)),
            ],
        );
        let c = one(&candidates);
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(&e.kind, EvidenceKind::BuildOutputOfProject { manifest } if manifest == "*.csproj")));
    }

    #[test]
    fn being_inside_a_repository_is_recorded_as_a_reason_for_caution() {
        let h = harness();
        std::fs::create_dir_all(h.path("code/repo/.git")).unwrap();
        let candidates = h.run(
            DeveloperDebrisDetector::new(),
            vec![
                fixture_entry("code/repo/Cargo.toml", 500, 200),
                fixture_entry("code/repo/target/debug/binary", 500, mb(250)),
            ],
        );
        let c = one(&candidates);
        assert!(c
            .evidence
            .iter()
            .any(|e| matches!(e.kind, EvidenceKind::InsideRepository)));
    }

    #[test]
    fn a_manifest_for_a_different_ecosystem_does_not_vouch_for_the_folder() {
        let h = harness();
        let candidates = h.run(
            DeveloperDebrisDetector::new(),
            vec![
                fixture_entry("code/thing/package.json", 500, 200),
                fixture_entry("code/thing/target/debug/binary", 500, mb(250)),
            ],
        );
        assert!(
            candidates.is_empty(),
            "package.json does not explain a Cargo target directory"
        );
    }
}
