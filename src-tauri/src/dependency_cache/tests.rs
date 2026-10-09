use super::*;
use filetime::FileTime;
use std::fs;

const NOW: i64 = 2_000_000_000;
struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        #[cfg(target_os = "macos")]
        let temp = tempfile::tempdir_in("/private/tmp").unwrap();
        #[cfg(not(target_os = "macos"))]
        let temp = tempfile::tempdir().unwrap();
        // /tmp is a symlink on macOS; the reader checks every ancestor.
        let home = fs::canonicalize(temp.path()).unwrap().join("home/tester");
        fs::create_dir_all(&home).unwrap();
        Self { _temp: temp, home }
    }
    fn file(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.home.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }
    fn age(&self, rel: &str, days: i64) {
        for entry in walkdir::WalkDir::new(self.home.join(rel)).contents_first(true) {
            let entry = entry.unwrap();
            filetime::set_file_mtime(
                entry.path(),
                FileTime::from_unix_time(NOW - days * 86_400, 0),
            )
            .unwrap();
        }
    }
    fn maven(&self, version: &str, origin: &str) {
        let base = format!(".m2/repository/org/demo/lib/{version}");
        self.file(&format!("{base}/lib-{version}.jar"), "jar");
        self.file(&format!("{base}/lib-{version}.pom"), "<project/>");
        if origin != "unknown" {
            let repo = if origin == "local" { "" } else { "central" };
            self.file(
                &format!("{base}/_remote.repositories"),
                &format!("lib-{version}.jar>{repo}=\nlib-{version}.pom>{repo}=\n"),
            );
        }
        self.age(&base, 150);
    }
    fn locations(&self) -> Vec<CacheLocation> {
        vec![
            CacheLocation {
                kind: CacheKind::Maven,
                path: self.home.join(".m2/repository"),
            },
            CacheLocation {
                kind: CacheKind::Gradle,
                path: self.home.join(".gradle"),
            },
            CacheLocation {
                kind: CacheKind::Npm,
                path: self.home.join("npm"),
            },
            CacheLocation {
                kind: CacheKind::Pnpm,
                path: self.home.join("pnpm"),
            },
            CacheLocation {
                kind: CacheKind::Yarn,
                path: self.home.join("yarn"),
            },
        ]
    }
    fn inspect(&self, ignores: IgnoreSet) -> CacheReport {
        inspect(
            &self.locations(),
            &[self.home.join("Projects")],
            &ProtectedPaths::for_home(&self.home),
            &ignores,
            90,
            NOW,
            &|| false,
        )
    }
}

#[test]
fn inventories_all_ecosystems_without_treating_age_as_permission() {
    let f = Fixture::new();
    f.maven("1.0", "downloaded");
    f.file(
        ".gradle/caches/modules-2/files-2.1/org.demo/lib/1.0/abcd/lib.jar",
        "gradle",
    );
    f.file(".gradle/caches/build-cache-1/abcdef123456", "generated");
    f.file(
        ".gradle/wrapper/dists/gradle-8.0-bin/abcdef/gradle.zip",
        "distribution",
    );
    f.file("npm/content-v2/sha512/aa/bb/cccccccc", "npm");
    f.file("pnpm/v3/files/aa/bbbbbbbb", "pnpm");
    f.file(
        "yarn/v6/npm-lib-1.0-abcdef/node_modules/lib/index.js",
        "yarn",
    );
    // These are never inventoried or offered as cache entries.
    f.file(".m2/settings.xml", "secret settings");
    f.file(".gradle/gradle.properties", "secret properties");
    f.file("npm/index-v5/aa/bb/cccc", "index metadata");
    f.file(
        "Projects/app/node_modules/lib/index.js",
        "active dependency",
    );
    for root in [".gradle", "npm", "pnpm", "yarn"] {
        f.age(root, 150);
    }
    let before = fs::read(f.home.join(".m2/settings.xml")).unwrap();
    let report = f.inspect(IgnoreSet::default());
    assert!(report.complete, "{:?}", report.notes);
    assert_eq!(report.entries.len(), 7);
    assert_eq!(report.eligible, 0);
    assert!(report.entries.iter().all(|e| e.last_used_unix.is_none()));
    assert!(report
        .entries
        .iter()
        .all(|e| e.decision == Decision::InsufficientEvidence));
    assert_eq!(fs::read(f.home.join(".m2/settings.xml")).unwrap(), before);
    for entry in &report.entries {
        assert!(entry.path.exists());
        assert!(!entry.metadata_fingerprint.is_empty());
    }
    // Wall-clock duration is not part of the decision or report.
    assert_eq!(
        serde_json::to_value(report).unwrap(),
        serde_json::to_value(f.inspect(IgnoreSet::default())).unwrap()
    );
}

#[test]
fn protects_shared_versions_local_artifacts_snapshots_recent_changes_and_keeps() {
    let f = Fixture::new();
    for (version, origin) in [
        ("1.0", "downloaded"),
        ("2.0", "local"),
        ("3.0-SNAPSHOT", "downloaded"),
        ("4.0", "downloaded"),
        ("5.0", "unknown"),
    ] {
        f.maven(version, origin);
    }
    f.age(".m2/repository/org/demo/lib/4.0", 2);
    f.file("Projects/older/pom.xml", "<project><dependencies><dependency><groupId>org.demo</groupId><artifactId>lib</artifactId><version>1.0</version></dependency></dependencies></project>");
    f.file("Projects/newer/pom.xml", "<project><dependencies><dependency><groupId>org.demo</groupId><artifactId>lib</artifactId><version>4.0</version></dependency></dependencies></project>");
    let ignores = IgnoreSet {
        paths: vec![f.home.join(".m2/repository/org/demo/lib/5.0")],
        ..Default::default()
    };
    let report = f.inspect(ignores);
    assert_eq!(report.kept, 5);
    for (version, reason) in [
        ("1.0", Reason::ProjectReference),
        ("2.0", Reason::LocalArtifact),
        ("3.0-SNAPSHOT", Reason::Snapshot),
        ("4.0", Reason::RecentlyChanged),
        ("5.0", Reason::UserKept),
    ] {
        assert!(report
            .entries
            .iter()
            .find(|e| e.version.as_deref() == Some(version))
            .unwrap()
            .reasons
            .contains(&reason));
    }
    assert_eq!(report.eligible, 0);
}

#[test]
fn gradle_locks_and_wrappers_protect_dependencies_and_distributions() {
    let f = Fixture::new();
    f.file(
        ".gradle/caches/modules-2/files-2.1/org.demo/lib/1.0/hash/lib.jar",
        "jar",
    );
    f.file(
        ".gradle/wrapper/dists/gradle-8.0-bin/hash/gradle.zip",
        "zip",
    );
    f.age(".gradle", 200);
    f.file("Projects/gradle/build.gradle.kts", "// Do not execute me");
    f.file(
        "Projects/gradle/gradle.lockfile",
        "org.demo:lib:1.0=compileClasspath\n",
    );
    f.file(
        "Projects/gradle/gradle/wrapper/gradle-wrapper.properties",
        "distributionUrl=https\\://services.gradle.org/distributions/gradle-8.0-bin.zip\n",
    );
    let report = f.inspect(IgnoreSet::default());
    assert_eq!(report.kept, 2);
    assert!(report
        .entries
        .iter()
        .all(|e| e.reasons.contains(&Reason::ProjectReference)));
    assert!(report.projects.iter().all(|p| !p.coverage_complete));
}

#[test]
fn cancellation_and_unknown_layouts_never_produce_eligible_entries() {
    let f = Fixture::new();
    f.maven("1.0", "unknown");
    let report = inspect(
        &f.locations(),
        &[],
        &ProtectedPaths::for_home(&f.home),
        &IgnoreSet::default(),
        90,
        NOW,
        &|| true,
    );
    assert!(!report.complete);
    assert!(report.entries.is_empty());
    assert_eq!(report.eligible, 0);
    let custom = f.home.join("custom");
    fs::create_dir_all(&custom).unwrap();
    f.file("custom/dont-delete.jar", "mine");
    let report = inspect(
        &[CacheLocation {
            kind: CacheKind::Maven,
            path: custom,
        }],
        &[],
        &ProtectedPaths::for_home(&f.home),
        &IgnoreSet::default(),
        90,
        NOW,
        &|| false,
    );
    assert!(report.entries.is_empty());
}

#[cfg(unix)]
#[test]
fn linked_roots_and_descendants_are_never_read_or_measured() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.file(".ssh/secret", "secret");
    fs::create_dir_all(f.home.join("npm/content-v2/sha512/aa/bb")).unwrap();
    symlink(
        f.home.join(".ssh/secret"),
        f.home.join("npm/content-v2/sha512/aa/bb/abcdef"),
    )
    .unwrap();
    symlink(f.home.join(".ssh"), f.home.join(".m2")).unwrap();
    let report = f.inspect(IgnoreSet::default());
    assert!(!report.complete);
    assert!(report.entries.is_empty());
    assert_eq!(
        fs::read_to_string(f.home.join(".ssh/secret")).unwrap(),
        "secret"
    );
}

#[test]
fn generic_move_gate_protects_cache_contents_and_their_parents_when_preview_is_off() {
    let f = Fixture::new();
    f.maven("1.0", "downloaded");
    let mut protected = ProtectedPaths::for_home(&f.home);
    protect_managed(&mut protected, &locations(&f.home, &[]));
    assert!(protected.is_protected(&f.home.join(".m2/repository/org/demo/lib/1.0/lib-1.0.jar")));
    assert!(protected
        .first_protected_descendant(&f.home.join(".m2"), &|| false)
        .is_some());
    assert!(!protected.is_protected(&f.home.join(".m2-neighbor/lib.jar")));
    let custom = CacheLocation {
        kind: CacheKind::Npm,
        path: f.home.join("custom/npm/_cacache"),
    };
    protect_managed(&mut protected, std::slice::from_ref(&custom));
    assert!(protected.is_protected(&custom.path.join("content-v2/blob")));
}

#[test]
fn old_preferences_remain_off_and_retention_is_separate_from_build_output() {
    let settings: crate::storage::Settings =
        serde_json::from_str(r#"{"developer_stale_days":14}"#).unwrap();
    assert!(!settings.dependency_caches.enabled);
    assert_eq!(settings.dependency_caches.retention_days, 90);
    assert!(settings.dependency_caches.locations.is_empty());
}

#[test]
fn node_lockfile_integrities_and_yarn_package_metadata_protect_project_references() {
    use base64::Engine;
    let f = Fixture::new();
    let hex = "ab".repeat(64);
    let integrity = base64::engine::general_purpose::STANDARD.encode([0xab; 64]);
    f.file(&format!("npm/content-v2/sha512/ab/ab/{}", &hex[4..]), "npm");
    f.file(
        "yarn/v6/npm-lib-1.0.0-abcdef/node_modules/@scope/lib/package.json",
        r#"{"name":"@scope/lib","version":"1.0.0"}"#,
    );
    f.file(
        "Projects/node/package.json",
        r#"{"dependencies":{"@scope/lib":"1.0.0"}}"#,
    );
    f.file("Projects/node/package-lock.json", &format!(r#"{{"lockfileVersion":3,"packages":{{"node_modules/@scope/lib":{{"version":"1.0.0","integrity":"sha512-{integrity}"}}}}}}"#));
    for root in ["npm", "yarn"] {
        f.age(root, 150);
    }
    let report = f.inspect(IgnoreSet::default());
    assert_eq!(report.kept, 2);
    assert!(report.entries.iter().all(|e| e.artifact == "@scope/lib"
        && e.version.as_deref() == Some("1.0.0")
        && e.reasons.contains(&Reason::ProjectReference)));
    assert_eq!(report.eligible, 0);
}

#[test]
fn application_preview_obeys_opt_in_operation_gate_and_persisted_preferences() {
    use crate::commands::{AppState, Operation, RummageRequest};
    use crate::platform::testing::FixedPlatform;
    let f = Fixture::new();
    f.maven("1.0", "downloaded");
    let platform = std::sync::Arc::new(FixedPlatform::new(&f.home));
    let state = AppState::new(platform.clone()).unwrap();
    assert!(matches!(
        state.inspect_dependency_caches(),
        Err(crate::ScuttleError::Refused(_))
    ));
    assert_eq!(state.current_operation(), None);
    let mut settings = state.store().settings().unwrap();
    settings.dependency_caches.enabled = true;
    settings.dependency_caches.retention_days = 180;
    settings.dependency_caches.locations = vec![CacheLocation {
        kind: CacheKind::Npm,
        path: f.home.join("custom/npm"),
    }];
    state.store().save_settings(&settings).unwrap();
    let hold = state.begin_operation(Operation::Move).unwrap();
    assert!(matches!(
        state.inspect_dependency_caches(),
        Err(crate::ScuttleError::Busy(_))
    ));
    drop(hold);
    let report = state.inspect_dependency_caches().unwrap();
    assert_eq!(report.retention_days, 180);
    assert_eq!(report.entries.len(), 1);
    assert_eq!(state.current_operation(), None);
    let dry = state
        .dry_run(RummageRequest {
            roots: Some(vec![f.home.clone()]),
            include_developer_debris: Some(false),
        })
        .unwrap();
    assert!(dry.dependency_caches.is_some());
    assert!(!dry.would_quarantine.iter().any(|e| e.path.contains(".m2")));
    drop(state);
    let reopened = AppState::new(platform).unwrap();
    assert!(
        reopened
            .store()
            .settings()
            .unwrap()
            .dependency_caches
            .enabled
    );
    assert!(reopened
        .protected_paths()
        .is_protected(&f.home.join("custom/npm/content-v2/blob")));
}

#[test]
fn oversized_manifests_and_directories_are_marked_partial() {
    let f = Fixture::new();
    f.maven("1.0", "unknown");
    f.file("Projects/node/package.json", &"x".repeat(1_048_577));
    let report = f.inspect(IgnoreSet::default());
    assert!(!report.complete);
    assert_eq!(report.eligible, 0);
    assert!(report.notes.iter().any(|n| n.contains("read budget")));
    let huge = f.home.join("huge");
    fs::create_dir_all(&huge).unwrap();
    for i in 0..5_001 {
        fs::write(huge.join(format!("{i:06x}")), b"x").unwrap();
    }
    let protected = ProtectedPaths::for_home(&f.home);
    let mut reader = reader::Reader::new(&protected, &|| false);
    assert!(reader.children(&huge).is_empty());
    assert!(!reader.complete);
}

#[test]
fn custom_locations_refuse_credentials_relative_paths_and_structural_folders() {
    let f = Fixture::new();
    let protected = ProtectedPaths::for_home(&f.home);
    for path in [
        f.home.clone(),
        f.home.join("Documents"),
        f.home.join(".ssh"),
        PathBuf::from("relative/cache"),
    ] {
        let mut preferences = Preferences {
            locations: vec![CacheLocation {
                kind: CacheKind::Maven,
                path,
            }],
            ..Default::default()
        };
        assert!(validate_preferences(&mut preferences, &protected).is_err());
    }
    let mut preferences = Preferences {
        retention_days: 1,
        locations: vec![CacheLocation {
            kind: CacheKind::Gradle,
            path: f.home.join("custom/./gradle"),
        }],
        ..Default::default()
    };
    validate_preferences(&mut preferences, &protected).unwrap();
    assert_eq!(preferences.retention_days, 90);
    assert_eq!(preferences.locations[0].path, f.home.join("custom/gradle"));
}

#[test]
fn new_moves_are_refused_but_preexisting_drawer_cache_items_can_be_restored() {
    use crate::commands::AppState;
    use crate::model::CleanupCandidate;
    use crate::quarantine::Quarantine;
    use crate::safety::assess::CautionKind;
    use crate::safety::{ActionContext, Bidding};
    use crate::storage::Store;
    use std::sync::Arc;
    let f = Fixture::new();
    let platform = Arc::new(crate::platform::testing::FixedPlatform::new(&f.home));
    let state = AppState::new(platform).unwrap();
    let path = f.file(
        ".npm/_cacache/content-v2/sha512/aa/bb/abcdef",
        "old cache blob",
    );
    // Also covers the Windows build, where this is a custom npm location.
    let mut settings = state.store().settings().unwrap();
    settings.dependency_caches.locations.push(CacheLocation {
        kind: CacheKind::Npm,
        path: f.home.join(".npm/_cacache"),
    });
    state.store().save_settings(&settings).unwrap();
    let fingerprint = crate::safety::observe(&path).unwrap();
    let mut candidate: CleanupCandidate = serde_json::from_value(serde_json::json!({
        "id": "legacy", "detector": "caches", "category": "caches", "target_kind": "file",
        "path": path, "display_name": "Old npm blob", "associated_app": "npm",
        "size": fingerprint.size, "confidence": "high", "risk": "low", "recommended_action": "quarantine",
        "evidence": [], "remark": null, "modified_unix": fingerprint.modified_unix,
        "accessed_unix": null, "created_unix": null, "group": [], "fingerprint": fingerprint,
    })).unwrap();
    assert!(matches!(
        state.hold_for_user(&candidate, &CautionKind::ALL),
        Err(crate::ScuttleError::Refused(_))
    ));
    assert!(path.exists());
    // A generic parent-folder finding cannot evade the store protection.
    candidate.path = f.home.join(".npm");
    candidate.category = crate::model::Category::HeavyStrays;
    candidate.target_kind = crate::model::TargetKind::Directory;
    candidate.fingerprint = crate::safety::observe(&candidate.path).unwrap();
    assert!(matches!(
        state.hold_for_user(&candidate, &CautionKind::ALL),
        Err(crate::ScuttleError::Refused(_))
    ));
    candidate.path = path.clone();
    candidate.category = crate::model::Category::Caches;
    candidate.target_kind = crate::model::TargetKind::File;
    candidate.fingerprint = crate::safety::observe(&path).unwrap();
    // Simulate a record held under the previous release's protection table.
    let protected = state.base_protected_paths();
    let roots = vec![f.home.clone()];
    let ctx = ActionContext {
        protected: &protected,
        allowed_roots: &roots,
        bidding: Bidding::UserSpecific,
        acknowledged: &CautionKind::ALL,
        installs: &crate::platform::installations::NO_INSTALL_AREAS,
        developer_platform: None,
    };
    let legacy_store =
        Arc::new(Store::open(&state.platform().data_dir().join("scuttle.db")).unwrap());
    let legacy = Quarantine::new(state.platform().quarantine_root(), legacy_store, 14)
        .with_protected(protected.clone());
    let record = legacy.hold(&candidate, &ctx, NOW).unwrap();
    assert!(!path.exists());
    let restored = state
        .quarantine()
        .unwrap()
        .restore(&record.id, NOW + 1)
        .unwrap();
    assert_eq!(restored.path, path);
    assert_eq!(fs::read_to_string(path).unwrap(), "old cache blob");
}

#[test]
fn a_keep_decision_for_a_child_file_preserves_the_whole_version() {
    let f = Fixture::new();
    f.maven("1.0", "downloaded");
    let report = f.inspect(IgnoreSet {
        paths: vec![f.home.join(".m2/repository/org/demo/lib/1.0/lib-1.0.jar")],
        ..Default::default()
    });
    assert_eq!(report.kept, 1);
    assert!(report.entries[0].reasons.contains(&Reason::UserKept));
    // A similar neighboring coordinate must not inherit the decision.
    let report = f.inspect(IgnoreSet {
        paths: vec![f.home.join(".m2/repository/org/demo/library/1.0/lib.jar")],
        ..Default::default()
    });
    assert_eq!(report.kept, 0);
}

#[test]
fn pnpm_executable_blobs_are_counted_without_accepting_them_as_npm_blobs() {
    let f = Fixture::new();
    f.file("pnpm/v10/files/aa/bbbbbb-exec", "executable content");
    f.file("pnpm/v3/files/bb/cccccc", "regular content");
    f.file("pnpm/v3/files/bb/cccccc-index.json", "metadata");
    f.file(
        "npm/content-v2/sha512/aa/bb/bbbbbb-exec",
        "not an npm content key",
    );
    let report = f.inspect(IgnoreSet::default());
    assert!(report.complete);
    assert_eq!(report.entries.len(), 2);
    assert!(report.entries.iter().all(|e| e.kind == CacheKind::Pnpm));
    assert_eq!(
        report.bytes,
        ("executable content".len() + "regular content".len()) as u64
    );
}

#[test]
fn a_disappearing_measurement_propagates_partial_status_to_the_reader() {
    let f = Fixture::new();
    let path = f.file("cache/blob", "content");
    let protected = ProtectedPaths::for_home(&f.home);
    let mut reader = reader::Reader::new(&protected, &|| false);
    assert!(reader.file(&path));
    fs::remove_file(&path).unwrap();
    let measured = reader.measure(&path);
    assert!(!measured.complete);
    assert!(
        !reader.complete,
        "aggregate report must label this lower-bound measurement partial"
    );
}

#[test]
fn cancellation_before_a_queued_worker_runs_is_not_lost_or_inherited() {
    use crate::commands::{AppState, Operation};
    let f = Fixture::new();
    f.maven("1.0", "downloaded");
    let state = AppState::new(std::sync::Arc::new(
        crate::platform::testing::FixedPlatform::new(&f.home),
    ))
    .unwrap();
    let mut settings = state.store().settings().unwrap();
    settings.dependency_caches.enabled = true;
    state.store().save_settings(&settings).unwrap();
    let queued = state.prepare_dependency_cache_inspection().unwrap();
    assert_eq!(state.current_operation(), Some(Operation::Scan));
    state.cancel_dependency_cache_preview();
    let stopped = queued.run().unwrap();
    assert!(!stopped.complete);
    assert!(stopped.entries.is_empty());
    assert_eq!(state.current_operation(), None);
    let retried = state.inspect_dependency_caches().unwrap();
    assert_eq!(retried.entries.len(), 1);
    assert!(retried.complete);
    let abandoned = state.prepare_dependency_cache_inspection().unwrap();
    drop(abandoned);
    assert_eq!(state.current_operation(), None);
    state.cancel_dependency_cache_preview(); // No active task: cannot cancel the next task.
    assert_eq!(state.inspect_dependency_caches().unwrap().entries.len(), 1);
}

#[test]
fn a_manifest_removed_after_reading_marks_the_inventory_partial() {
    let f = Fixture::new();
    let path = f.file("cache/package.json", "{\"name\":\"example\"}");
    let calls = std::cell::Cell::new(0);
    let checkpoint = || {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            fs::remove_file(&path).unwrap();
        }
        false
    };
    let protected = ProtectedPaths::for_home(&f.home);
    let mut reader = reader::Reader::new(&protected, &checkpoint);
    assert!(reader.text(&path).is_none());
    assert!(!reader.complete);
    assert!(reader
        .notes
        .iter()
        .any(|note| note.contains("disappeared after reading")));
}

#[test]
fn unknown_modification_times_make_aggregate_metadata_partial() {
    let f = Fixture::new();
    let path = f.file("cache/blob", "content");
    filetime::set_file_mtime(&path, FileTime::from_unix_time(-1, 0)).unwrap();
    let protected = ProtectedPaths::for_home(&f.home);
    let mut reader = reader::Reader::new(&protected, &|| false);
    let measured = reader.measure(&path);
    assert!(!measured.complete);
    assert!(!reader.complete);
    assert_eq!(measured.newest, None);
    assert_eq!(measured.bytes, 7);
}
