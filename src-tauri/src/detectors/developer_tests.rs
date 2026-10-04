use super::*;
use crate::{
    detectors::{
        developer::{self, RepositoryState},
        test_support::Harness,
    },
    model::CleanupCandidate,
    platform::{installations::NO_INSTALL_AREAS, testing::FixedPlatform},
    safety::{
        self,
        assess::{CautionKind, Eligibility, Impact},
        ActionContext, Bidding,
    },
    scanning::{self, SilentObserver},
};
use std::{fs, path::Path, process::Command, sync::Arc};

fn file(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn age(path: &Path, days: i64) {
    let time = filetime::FileTime::from_unix_time(chrono::Utc::now().timestamp() - days * 86400, 0);
    filetime::set_file_mtime(path, time)
        .unwrap_or_else(|e| panic!("could not age {}: {e}", path.display()));
}
fn age_tree(path: &Path, days: i64) {
    for e in walkdir::WalkDir::new(path)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        age(e.unwrap().path(), days);
    }
}
fn fixture() -> Harness {
    let mut h = Harness::new();
    h.options.include_developer_debris = true;
    h.processes = vec!["finder".into()];
    file(
        &h.path("Code/repo/Cargo.toml"),
        b"[package]\nname='demo'\nversion='0.1.0'\n",
    );
    file(&h.path("Code/repo/src/lib.rs"), b"pub fn hello() {}\n");
    file(&h.path("Code/repo/target/CACHEDIR.TAG"), b"Signature: 8a477f597d28d172789f06886806bc55\n# This file is a cache directory tag created by cargo.\n");
    fs::create_dir_all(h.path("Code/repo/target/debug/.fingerprint")).unwrap();
    let output = h.path("Code/repo/target/debug/deps/binary");
    file(&output, b"");
    fs::File::options()
        .write(true)
        .open(output)
        .unwrap()
        .set_len(110 * 1024 * 1024)
        .unwrap();
    age_tree(&h.path("Code/repo"), 28);
    h
}
fn scan(h: &Harness) -> CleanupCandidate {
    let results = h.run_bare(DeveloperDebrisDetector::new());
    assert_eq!(results.len(), 1, "{results:#?}");
    results.into_iter().next().unwrap()
}
fn gate(
    h: &Harness,
    c: &CleanupCandidate,
    platform: &FixedPlatform,
) -> crate::Result<safety::AuthorizedTarget> {
    safety::authorize(
        c,
        &ActionContext {
            protected: &h.context().protected,
            allowed_roots: std::slice::from_ref(&h.home),
            bidding: Bidding::User,
            acknowledged: &CautionKind::ALL,
            installs: &NO_INSTALL_AREAS,
            developer_platform: Some(platform),
        },
    )
}
fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Scuttle tests",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=",
            "-c",
            "maintenance.auto=false",
            "-c",
            "gc.auto=0",
        ])
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", root.join("nonexistent-test-config"))
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("Git is required to create repository test fixtures");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn repository(h: &Harness) {
    let root = h.path("Code/repo");
    git(&root, &["init", "--template=", "-q"]);
    file(&root.join(".gitignore"), b"/target/\n");
    git(&root, &["add", "Cargo.toml", "src", ".gitignore"]);
    git(&root, &["commit", "-qm", "fixture"]);
    age_tree(&root, 28);
}

#[test]
fn stale_verified_output_is_suggested_and_can_be_rebuilt() {
    let h = fixture();
    let c = scan(&h);
    assert_eq!(c.assessment.eligibility, Eligibility::Suggested);
    assert_eq!(c.assessment.impact, Impact::Regenerable);
    assert!(c.remark.unwrap().contains("rebuild"));
}
#[test]
fn fourteen_day_boundary_is_inclusive_and_configurable() {
    let mut h = fixture();
    for (days, suggested) in [(13, false), (14, true), (15, true)] {
        age_tree(&h.path("Code/repo"), days);
        assert_eq!(
            scan(&h).assessment.eligibility == Eligibility::Suggested,
            suggested
        );
    }
    h.options.developer_stale_days = 30;
    assert_ne!(scan(&h).assessment.eligibility, Eligibility::Suggested);
}
#[test]
fn nested_source_and_new_builds_prevent_suggestions() {
    let h = fixture();
    age(&h.path("Code/repo/src/lib.rs"), 1);
    assert_ne!(scan(&h).assessment.eligibility, Eligibility::Suggested);
    age_tree(&h.path("Code/repo"), 28);
    age(&h.path("Code/repo/target/debug/deps/binary"), 1);
    assert_ne!(scan(&h).assessment.eligibility, Eligibility::Suggested);
}
#[test]
fn running_or_unknown_tools_prevent_suggestions() {
    let mut h = fixture();
    for processes in [vec!["rustc".into()], vec![]] {
        h.processes = processes;
        assert_ne!(scan(&h).assessment.eligibility, Eligibility::Suggested);
    }
}
#[test]
fn invalid_markers_and_authored_files_prevent_expiry() {
    let h = fixture();
    file(
        &h.path("Code/repo/target/CACHEDIR.TAG"),
        b"a name is not evidence",
    );
    age_tree(&h.path("Code/repo"), 28);
    let c = scan(&h);
    assert!(!c.assessment.impact.may_expire());
    let h = fixture();
    file(&h.path("Code/repo/target/notes.md"), b"my research");
    age_tree(&h.path("Code/repo"), 28);
    assert!(!scan(&h).assessment.impact.may_expire());
}
#[test]
fn a_name_without_a_manifest_is_not_a_finding_and_opt_in_is_required() {
    let mut h = fixture();
    h.options.include_developer_debris = false;
    assert!(h.run_bare(DeveloperDebrisDetector::new()).is_empty());
    h.options.include_developer_debris = true;
    fs::remove_file(h.path("Code/repo/Cargo.toml")).unwrap();
    assert!(h.run_bare(DeveloperDebrisDetector::new()).is_empty());
}
#[test]
fn small_outputs_are_not_surfaced() {
    let h = fixture();
    fs::write(h.path("Code/repo/target/debug/deps/binary"), b"small").unwrap();
    assert!(h.run_bare(DeveloperDebrisDetector::new()).is_empty());
}

#[test]
fn legacy_framework_configs_remain_manual_and_keep_decisions_are_respected() {
    for (folder, manifest) in [(".next", "next.config.mjs"), (".nuxt", "nuxt.config.ts")] {
        let mut h = fixture();
        fs::rename(
            h.path("Code/repo/target"),
            h.path(&format!("Code/repo/{folder}")),
        )
        .unwrap();
        fs::remove_file(h.path("Code/repo/Cargo.toml")).unwrap();
        file(
            &h.path(&format!("Code/repo/{manifest}")),
            b"export default {};",
        );
        age_tree(&h.path("Code/repo"), 28);
        let c = scan(&h);
        assert_ne!(c.assessment.eligibility, Eligibility::Suggested);
        assert_eq!(c.assessment.impact, Impact::PersonalFile);
        h.ignores.paths.push(c.path);
        assert!(h.run_bare(DeveloperDebrisDetector::new()).is_empty());
        h.ignores.paths.clear();
        h.ignores.categories.push(Category::DeveloperDebris);
        assert!(h.run_bare(DeveloperDebrisDetector::new()).is_empty());
    }
}
#[test]
fn ignored_git_output_is_suggested_but_tracked_output_is_blocked() {
    let h = fixture();
    repository(&h);
    assert_eq!(scan(&h).assessment.eligibility, Eligibility::Suggested);
    file(&h.path("Code/repo/target/readme"), b"tracked");
    git(&h.path("Code/repo"), &["add", "-f", "target/readme"]);
    age_tree(&h.path("Code/repo"), 28);
    assert_eq!(scan(&h).assessment.eligibility, Eligibility::Blocked);
}
#[test]
fn staged_deletion_does_not_erase_head_tracking() {
    let h = fixture();
    repository(&h);
    file(&h.path("Code/repo/target/readme"), b"tracked");
    let root = h.path("Code/repo");
    git(&root, &["add", "-f", "target/readme"]);
    git(&root, &["commit", "-qm", "track output"]);
    git(&root, &["rm", "--cached", "target/readme"]);
    age_tree(&root, 28);
    assert_eq!(scan(&h).assessment.eligibility, Eligibility::Blocked);
}
#[test]
fn unignored_and_unreadable_repositories_stay_manual() {
    let h = fixture();
    repository(&h);
    fs::write(h.path("Code/repo/.gitignore"), b"").unwrap();
    age_tree(&h.path("Code/repo"), 28);
    assert_eq!(
        scan(&h).developer_artifact().unwrap().repository,
        RepositoryState::NotIgnored
    );
    fs::write(h.path("Code/repo/.git/index"), b"broken index").unwrap();
    assert_eq!(
        scan(&h).developer_artifact().unwrap().repository,
        RepositoryState::Unknown
    );
}
#[test]
fn worktrees_and_submodule_git_files_are_understood() {
    let h = fixture();
    repository(&h);
    let worktree = h.path("Code/worktree");
    git(
        &h.path("Code/repo"),
        &["worktree", "add", "--detach", worktree.to_str().unwrap()],
    );
    assert_eq!(
        developer::repository_boundary(&worktree),
        Ok(Some(worktree.clone()))
    );
    assert_eq!(
        developer::repository_state(&worktree, &worktree.join("target")),
        RepositoryState::Ignored
    );
    let child = h.path("Code/repo/child");
    fs::create_dir_all(&child).unwrap();
    git(&child, &["init", "--template=", "-q"]);
    file(&child.join(".gitignore"), b"/target/\n");
    git(&child, &["add", ".gitignore"]);
    git(&child, &["commit", "-qm", "child"]);
    git(
        &h.path("Code/repo"),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "./child",
            "child",
        ],
    );
    git(&h.path("Code/repo"), &["submodule", "absorbgitdirs"]);
    assert!(child.join(".git").is_file());
    assert_eq!(
        developer::repository_state(&child, &child.join("target")),
        RepositoryState::Ignored
    );
}
#[test]
fn monorepo_sibling_changes_prevent_suggestions() {
    let h = fixture();
    repository(&h);
    fs::create_dir_all(h.path("Code/repo/crates/child")).unwrap();
    fs::rename(
        h.path("Code/repo/Cargo.toml"),
        h.path("Code/repo/crates/child/Cargo.toml"),
    )
    .unwrap();
    fs::rename(
        h.path("Code/repo/target"),
        h.path("Code/repo/crates/child/target"),
    )
    .unwrap();
    file(&h.path("Code/repo/.gitignore"), b"target/\n");
    age_tree(&h.path("Code/repo"), 28);
    assert_eq!(scan(&h).assessment.eligibility, Eligibility::Suggested);
    age(&h.path("Code/repo/src/lib.rs"), 1);
    assert_ne!(scan(&h).assessment.eligibility, Eligibility::Suggested);
}
#[test]
fn dependencies_and_legacy_findings_do_not_expire() {
    let h = fixture();
    fs::rename(h.path("Code/repo/target"), h.path("Code/repo/Pods")).unwrap();
    file(&h.path("Code/repo/Podfile"), b"pod 'Example'");
    age_tree(&h.path("Code/repo"), 28);
    let mut c = scan(&h);
    assert_eq!(
        c.developer_artifact().unwrap().artifact_kind,
        developer::ArtifactKind::Dependency
    );
    assert!(!c.assessment.impact.may_expire());
    c.evidence.clear();
    c.recommended_action = crate::model::RecommendedAction::Quarantine;
    safety::assess::settle(&mut c, chrono::Utc::now().timestamp());
    assert!(!c.assessment.impact.may_expire());
    assert_ne!(c.assessment.eligibility, Eligibility::Suggested);
}
#[test]
fn background_findings_are_preliminary_even_with_valid_markers() {
    let h = fixture();
    let c = h.run_bare(DeveloperDebrisDetector::preliminary()).remove(0);
    let state = c.developer_artifact().unwrap();
    assert!(state.preliminary);
    assert!(state.marker_digest.is_none());
    assert_eq!(state.repository, RepositoryState::Unknown);
    assert!(!c.assessment.impact.may_expire());
}
#[test]
fn cancellation_and_protected_content_make_measurement_partial() {
    let h = fixture();
    let ctx = h.context();
    assert!(!developer::tree(&h.path("Code/repo"), &ctx.protected, true, &|| true).complete);
    file(&h.path("Code/repo/target/.git/secret"), b"repository data");
    let c = scan(&h);
    assert!(!c.developer_artifact().unwrap().output.complete);
    assert!(c.size_human().starts_with("at least "));
    assert_ne!(c.assessment.eligibility, Eligibility::Suggested);
}
#[cfg(unix)]
#[test]
fn symlinks_and_symlinked_markers_are_not_followed() {
    let h = fixture();
    let marker = h.path("Code/repo/target/CACHEDIR.TAG");
    fs::rename(&marker, h.path("tag")).unwrap();
    std::os::unix::fs::symlink(h.path("tag"), marker).unwrap();
    let c = scan(&h);
    assert!(c.developer_artifact().unwrap().marker_digest.is_none());
    assert!(!c.developer_artifact().unwrap().output.complete);
}
#[test]
fn changes_after_scan_refuse_the_move_including_same_size_deep_edits() {
    let h = fixture();
    let c = scan(&h);
    let platform = FixedPlatform::new(&h.home).with_process("finder");
    assert!(gate(&h, &c, &platform).is_ok());
    age(&h.path("Code/repo/target/debug/deps/binary"), 1);
    assert_eq!(gate(&h, &c, &platform).unwrap_err().code(), "stale");
    let h = fixture();
    let c = scan(&h);
    age(&h.path("Code/repo/src/lib.rs"), 1);
    assert_eq!(gate(&h, &c, &platform).unwrap_err().code(), "stale");
}
#[test]
fn newly_tracked_output_and_new_running_tools_refuse_moves() {
    let h = fixture();
    repository(&h);
    let c = scan(&h);
    let platform = FixedPlatform::new(&h.home).with_process("finder");
    git(&h.path("Code/repo"), &["add", "-f", "target/CACHEDIR.TAG"]);
    assert_eq!(gate(&h, &c, &platform).unwrap_err().code(), "refused");
    let h = fixture();
    let c = scan(&h);
    let platform = FixedPlatform::new(&h.home).with_process("cargo");
    assert_eq!(gate(&h, &c, &platform).unwrap_err().code(), "stale");
}
#[test]
fn verified_output_can_be_quarantined_and_restored() {
    let h = fixture();
    let c = scan(&h);
    let platform = FixedPlatform::new(&h.home).with_process("finder");
    let ctx = h.context();
    let store = Arc::new(crate::storage::Store::in_memory().unwrap());
    let drawer = crate::quarantine::Quarantine::new(h.path(".scuttle/drawer"), store, 14);
    let roots = vec![h.home.clone()];
    let action = ActionContext {
        protected: &ctx.protected,
        allowed_roots: &roots,
        bidding: Bidding::Scuttle,
        acknowledged: &[],
        installs: &NO_INSTALL_AREAS,
        developer_platform: Some(&platform),
    };
    let now = chrono::Utc::now().timestamp();
    let held = drawer.hold(&c, &action, now).unwrap();
    assert!(!held.keep);
    assert!(!c.path.exists());
    let restored = drawer.restore(&held.id, now + 1).unwrap();
    assert_eq!(restored.path, c.path);
    assert!(c.path.join("debug/deps/binary").exists());
}
#[test]
fn next_and_dotnet_require_tool_specific_markers() {
    for dotnet in [false, true] {
        let h = fixture();
        let folder = if dotnet { "obj" } else { ".next" };
        fs::rename(
            h.path("Code/repo/target"),
            h.path(&format!("Code/repo/{folder}")),
        )
        .unwrap();
        if dotnet {
            file(
                &h.path("Code/repo/App.csproj"),
                b"<Project Sdk=\"Microsoft.NET.Sdk\" />",
            );
            let json = serde_json::json!({"version":3, "targets":{}, "libraries":{}, "project":{"restore":{"projectPath":h.path("Code/repo/App.csproj")}}});
            file(
                &h.path("Code/repo/obj/project.assets.json"),
                json.to_string().as_bytes(),
            );
        } else {
            file(
                &h.path("Code/repo/package.json"),
                br#"{"dependencies":{"next":"16"}}"#,
            );
            file(&h.path("Code/repo/.next/BUILD_ID"), b"generated-id");
            file(&h.path("Code/repo/.next/build-manifest.json"), b"{}");
        }
        age_tree(&h.path("Code/repo"), 28);
        assert_eq!(scan(&h).assessment.eligibility, Eligibility::Suggested);
    }
}
#[test]
fn developer_only_roots_do_not_feed_other_detectors() {
    let mut h = fixture();
    h.options.roots = vec![h.path("Downloads")];
    fs::create_dir_all(h.path("Downloads")).unwrap();
    h.options.developer_roots = vec![h.path("Code"), h.path("Code/repo")];
    h.options.heavy_threshold = 1;
    let ctx = h.context();
    let results = scanning::run(
        "only-dev",
        &ctx,
        crate::detectors::default_set(&ctx.options),
        &SilentObserver,
    );
    assert_eq!(results.candidates.len(), 1);
    assert_eq!(
        results.candidates[0].category,
        crate::model::Category::DeveloperDebris
    );
}

#[cfg(unix)]
#[test]
fn linked_git_metadata_is_unknown_and_never_authorizes_expiry() {
    let h = fixture();
    repository(&h);
    fs::rename(h.path("Code/repo/.git/index"), h.path("index-copy")).unwrap();
    std::os::unix::fs::symlink(h.path("index-copy"), h.path("Code/repo/.git/index")).unwrap();
    let c = scan(&h);
    assert_eq!(
        c.developer_artifact().unwrap().repository,
        RepositoryState::Unknown
    );
    assert!(!c.assessment.impact.may_expire());
}
#[test]
fn deep_output_and_unknown_timestamps_are_not_suggested() {
    let h = fixture();
    let mut deep = h.path("Code/repo/target");
    for _ in 0..65 {
        deep.push("d");
    }
    fs::create_dir_all(&deep).unwrap();
    let c = scan(&h);
    assert!(!c.developer_artifact().unwrap().output.complete);
    let h = fixture();
    let c = scan(&h);
    let mut state = c.developer_artifact().unwrap().clone();
    state.source.newest_unix = None;
    assert!(!state.verified(chrono::Utc::now().timestamp()));
}
#[test]
fn moving_one_verified_folder_does_not_invalidate_sibling_output() {
    let h = fixture();
    file(
        &h.path("Code/repo/package.json"),
        br#"{"dependencies":{"next":"16"}}"#,
    );
    file(&h.path("Code/repo/.next/BUILD_ID"), b"build-id");
    file(&h.path("Code/repo/.next/build-manifest.json"), b"{}");
    file(&h.path("Code/repo/.next/cache"), b"");
    fs::File::options()
        .write(true)
        .open(h.path("Code/repo/.next/cache"))
        .unwrap()
        .set_len(110 * 1024 * 1024)
        .unwrap();
    age_tree(&h.path("Code/repo"), 28);
    let candidates = h.run_bare(DeveloperDebrisDetector::new());
    assert_eq!(candidates.len(), 2);
    assert!(candidates
        .iter()
        .all(|c| c.assessment.eligibility == Eligibility::Suggested));
    let platform = FixedPlatform::new(&h.home).with_process("finder");
    gate(&h, &candidates[0], &platform).unwrap();
    fs::rename(&candidates[0].path, h.path("held-output")).unwrap();
    gate(&h, &candidates[1], &platform).unwrap();
}
#[test]
fn persisted_developer_roots_and_evidence_survive_restart() {
    let h = fixture();
    let platform: Arc<dyn crate::platform::PlatformService> =
        Arc::new(FixedPlatform::new(&h.home).with_process("finder"));
    let state = crate::commands::AppState::new(Arc::clone(&platform)).unwrap();
    let mut settings = state.store().settings().unwrap();
    settings.include_developer_debris = true;
    settings.developer_roots = vec![h.path("Code")];
    fs::create_dir_all(h.path("Downloads")).unwrap();
    settings.scan_roots = vec![h.path("Downloads")];
    state.store().save_settings(&settings).unwrap();
    let options = scanning::roots::resolve(platform.as_ref(), &settings, None, None);
    let id = state.start_scan(&options).unwrap();
    state.run_scan_with(&id, options, &SilentObserver).unwrap();
    drop(state);
    let state = crate::commands::AppState::new(Arc::clone(&platform)).unwrap();
    let c = state
        .store()
        .candidates_for_scan(&id)
        .unwrap()
        .into_iter()
        .find(|c| c.category == Category::DeveloperDebris)
        .unwrap();
    assert_eq!(c.assessment.eligibility, Eligibility::Suggested);
    assert!(state
        .action_scope()
        .ctx(Bidding::User, &CautionKind::ALL)
        .allowed_roots
        .contains(&h.path("Code")));
    assert_eq!(state.store().settings().unwrap().developer_stale_days, 14);
}
