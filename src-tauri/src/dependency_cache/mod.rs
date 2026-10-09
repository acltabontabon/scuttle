//! Opt-in, bounded dependency-cache inventory. This is deliberately separate
//! from generic directory findings: a managed store cannot be moved safely by
//! the ordinary Drawer gate. No adapter in this version claims reliable usage,
//! complete project coverage, compatible locking or a recoverable eviction.
pub mod policy;
mod projects;
mod reader;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::safety::{paths, ProtectedPaths};
use crate::scanning::IgnoreSet;
use policy::{Decision, Facts, Origin, Reason};
use reader::Reader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheKind {
    Maven,
    Gradle,
    Npm,
    Pnpm,
    Yarn,
}

impl CacheKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Maven => "Maven",
            Self::Gradle => "Gradle",
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "Yarn",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheLocation {
    pub kind: CacheKind,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub enabled: bool,
    pub retention_days: u32,
    /// Additional locations, never environment-derived or executed config.
    pub locations: Vec<CacheLocation>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: false,
            retention_days: policy::DEFAULT_RETENTION_DAYS,
            locations: Vec::new(),
        }
    }
}

pub fn validate_preferences(
    preferences: &mut Preferences,
    protected: &ProtectedPaths,
) -> crate::Result<()> {
    preferences.retention_days = policy::retention_days(preferences.retention_days);
    if preferences.locations.len() > 10 {
        return Err(crate::ScuttleError::Refused(
            "Choose at most ten additional dependency cache locations.".into(),
        ));
    }
    for location in &mut preferences.locations {
        crate::commands::check_scan_root(&location.path, protected)?;
        if protected.is_too_shallow(&location.path) {
            return Err(crate::ScuttleError::Refused(
                "Choose the cache directory itself, rather than a home or structural folder."
                    .into(),
            ));
        }
        location.path = paths::normalize(&location.path);
    }
    Ok(())
}

pub fn locations(home: &Path, extra: &[CacheLocation]) -> Vec<CacheLocation> {
    let mut out = vec![
        CacheLocation {
            kind: CacheKind::Maven,
            path: home.join(".m2/repository"),
        },
        CacheLocation {
            kind: CacheKind::Gradle,
            path: home.join(".gradle"),
        },
    ];
    #[cfg(target_os = "windows")]
    let node = [
        (CacheKind::Npm, "AppData/Local/npm-cache/_cacache"),
        (CacheKind::Pnpm, "AppData/Local/pnpm/store"),
        (CacheKind::Yarn, "AppData/Local/Yarn/Cache"),
    ];
    #[cfg(not(target_os = "windows"))]
    let node = [
        (CacheKind::Npm, ".npm/_cacache"),
        (CacheKind::Pnpm, "Library/pnpm/store"),
        (CacheKind::Yarn, "Library/Caches/Yarn"),
        (CacheKind::Pnpm, ".local/share/pnpm/store"),
        (CacheKind::Yarn, ".cache/yarn"),
    ];
    out.extend(node.into_iter().map(|(kind, rel)| CacheLocation {
        kind,
        path: home.join(rel),
    }));
    out.extend_from_slice(extra);
    for location in &mut out {
        location.path = paths::normalize(&location.path);
    }
    out.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| paths::folded_components(&a.path).cmp(&paths::folded_components(&b.path)))
    });
    out.dedup_by(|a, b| {
        a.kind == b.kind && paths::folded_components(&a.path) == paths::folded_components(&b.path)
    });
    out
}

/// Apply even when preview is disabled. Neither old findings nor another
/// detector may bypass per-artifact protection by moving a store or a parent.
pub fn protect_managed(protected: &mut ProtectedPaths, locations: &[CacheLocation]) {
    for location in locations {
        protected.also_protect(
            "Managed dependency cache; inspect it in Settings",
            location.path.clone(),
        );
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheEntry {
    pub id: String,
    pub kind: CacheKind,
    pub repository: PathBuf,
    pub path: PathBuf,
    pub artifact: String,
    pub version: Option<String>,
    pub bytes: u64,
    pub complete: bool,
    pub metadata_fingerprint: String,
    pub newest_modified_unix: Option<i64>,
    pub last_used_unix: Option<i64>,
    pub origin: Origin,
    pub projects: Vec<PathBuf>,
    pub decision: Decision,
    pub reasons: Vec<Reason>,
    pub explanations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepositoryReport {
    pub kind: CacheKind,
    pub path: PathBuf,
    pub present: bool,
    pub entries: usize,
    pub bytes: u64,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheReport {
    pub policy_version: u32,
    pub evaluated_unix: i64,
    pub retention_days: u32,
    pub complete: bool,
    pub repositories: Vec<RepositoryReport>,
    pub projects: Vec<projects::Project>,
    pub entries: Vec<CacheEntry>,
    pub bytes: u64,
    pub kept: usize,
    pub insufficient_evidence: usize,
    pub eligible: usize,
    pub notes: Vec<String>,
}

/// Shared desktop/terminal entry point. Callers enforce preview opt-in; this
/// function establishes the same locations, project boundary and exclusions
/// without creating a database or writing a scan.
pub fn inspect_configured(
    platform: &dyn crate::platform::PlatformService,
    settings: &crate::storage::Settings,
    ignores: &IgnoreSet,
    now: i64,
    cancelled: &dyn Fn() -> bool,
) -> CacheReport {
    let home = platform.home_dir();
    let roots = if cancelled() {
        Vec::new()
    } else if settings.developer_roots.is_empty() {
        crate::scanning::roots::common(platform)
    } else {
        settings.developer_roots.clone()
    };
    let mut protected = ProtectedPaths::for_home(&home);
    crate::platform::protect_own_files(&mut protected, platform);
    inspect(
        &locations(&home, &settings.dependency_caches.locations),
        &roots,
        &protected,
        ignores,
        settings.dependency_caches.retention_days,
        now,
        cancelled,
    )
}

/// Metadata and selected dependency manifests only; never launches a tool,
/// connects to a repository or writes cache contents or lock files. OS-managed
/// access timestamps may change when manifests are read; they are never used.
pub fn inspect(
    locations: &[CacheLocation],
    project_roots: &[PathBuf],
    protected: &ProtectedPaths,
    ignores: &IgnoreSet,
    days: u32,
    now: i64,
    cancelled: &dyn Fn() -> bool,
) -> CacheReport {
    let mut reader = Reader::new(protected, cancelled);
    reader.available(); // Empty inventories must still report an early cancellation.
    let projects = projects::discover(project_roots, &mut reader);
    let references_by_project = projects::ReferenceIndex::new(&projects, &mut reader);
    let mut entries = Vec::new();
    let mut repositories = Vec::new();
    let mut seen = BTreeSet::new();
    let mut ordered_locations = locations.to_vec();
    ordered_locations.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.path.cmp(&b.path)));
    for location in &ordered_locations {
        let before = entries.len();
        let present = reader.directory(&location.path);
        if present {
            let nodes = match location.kind {
                CacheKind::Maven => maven(location, &mut reader),
                CacheKind::Gradle => gradle(location, &mut reader),
                CacheKind::Npm => blobs(
                    &location.path.join("content-v2"),
                    "npm content",
                    false,
                    &mut reader,
                ),
                CacheKind::Pnpm => pnpm(location, &mut reader),
                CacheKind::Yarn => yarn(location, &mut reader),
            };
            for mut node in nodes {
                if !seen.insert(paths::folded_components(&node.path)) {
                    continue;
                }
                if entries.len() >= reader::MAX_ROWS {
                    reader.incomplete("The entry limit was reached; remaining cache entries were left uninspected.");
                    break;
                }
                let profile = reader.measure(&node.path);
                let mut references = references_by_project.references(
                    location.kind,
                    &node.artifact,
                    node.version.as_deref(),
                );
                if location.kind == CacheKind::Npm {
                    if let Some((artifact, version, paths)) =
                        references_by_project.npm_identity(&node.path, &location.path)
                    {
                        node.artifact = artifact;
                        node.version = Some(version);
                        references = paths;
                    }
                }
                let ignored =
                    ignores.paths.iter().any(|p| {
                        paths::is_within(&node.path, p) || paths::is_within(p, &node.path)
                    }) || ignores.categories.contains(&crate::model::Category::Caches)
                        || ignores
                            .categories
                            .contains(&crate::model::Category::DeveloperDebris)
                        || ignores
                            .apps
                            .iter()
                            .any(|a| a.eq_ignore_ascii_case(location.kind.label()));
                let facts = Facts {
                    ignored,
                    referenced: !references.is_empty(),
                    origin: node.origin,
                    snapshot: node
                        .version
                        .as_ref()
                        .is_some_and(|v| v.to_ascii_uppercase().contains("SNAPSHOT")),
                    newest_modified_unix: profile.newest,
                    last_used_unix: None,
                    complete: profile.complete,
                    project_coverage_complete: false,
                    recovery_verified: false,
                    coordination_verified: false,
                };
                let (decision, reasons) = policy::evaluate(&facts, days, now);
                let id = blake3::hash(
                    format!(
                        "{:?}:{:?}",
                        location.kind,
                        paths::folded_components(&node.path)
                    )
                    .as_bytes(),
                )
                .to_hex()
                .to_string();
                let explanations = reasons
                    .iter()
                    .map(|r| r.description().to_string())
                    .collect();
                entries.push(CacheEntry {
                    id,
                    kind: location.kind,
                    repository: location.path.clone(),
                    path: node.path,
                    artifact: node.artifact,
                    version: node.version,
                    bytes: profile.bytes,
                    complete: profile.complete,
                    metadata_fingerprint: profile.fingerprint,
                    newest_modified_unix: profile.newest,
                    last_used_unix: None,
                    origin: node.origin,
                    projects: references,
                    decision,
                    reasons,
                    explanations,
                });
            }
        }
        repositories.push(RepositoryReport {
            kind: location.kind,
            path: location.path.clone(),
            present,
            entries: entries.len() - before,
            bytes: entries[before..].iter().map(|e| e.bytes).sum(),
            complete: reader.complete,
        });
    }
    // A partial project/repository inventory must not look like a complete
    // result simply because one individual entry happened to be measurable.
    if !reader.complete {
        for repository in &mut repositories {
            repository.complete = false;
        }
    }
    entries.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.path.cmp(&b.path)));
    let bytes = entries.iter().map(|e| e.bytes).sum();
    let kept = entries
        .iter()
        .filter(|e| e.decision == Decision::Kept)
        .count();
    let insufficient_evidence = entries
        .iter()
        .filter(|e| e.decision == Decision::InsufficientEvidence)
        .count();
    let eligible = entries
        .iter()
        .filter(|e| e.decision == Decision::Eligible)
        .count();
    let mut notes: Vec<_> = reader.notes.into_iter().collect();
    notes.push("Inspection only: nothing was moved or deleted. Last use, full project coverage, recovery and tool coordination are not established by this preview.".into());
    notes.push("Only recognized cache layouts are measured. Repository-wide metadata, configuration, credentials and project node_modules are excluded. Add custom locations when your tools use other paths.".into());
    CacheReport {
        policy_version: policy::POLICY_VERSION,
        evaluated_unix: now,
        retention_days: policy::retention_days(days),
        complete: reader.complete,
        repositories,
        projects,
        entries,
        bytes,
        kept,
        insufficient_evidence,
        eligible,
        notes,
    }
}

struct Node {
    path: PathBuf,
    artifact: String,
    version: Option<String>,
    origin: Origin,
}
fn node(path: PathBuf, artifact: String, version: Option<String>) -> Node {
    Node {
        path,
        artifact,
        version,
        origin: Origin::Unknown,
    }
}
fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn maven(location: &CacheLocation, reader: &mut Reader<'_>) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut pending = vec![(location.path.clone(), 0)];
    while let Some((path, depth)) = pending.pop() {
        if !reader.available() {
            break;
        }
        let children = reader.children(&path);
        let version = name(&path);
        let artifact = path.parent().map(name).unwrap_or_default();
        let prefix = format!("{artifact}-{version}");
        let is_version = depth >= 3
            && children.iter().any(|p| {
                let n = name(p);
                (n == format!("{prefix}.pom") || n == format!("{prefix}.jar")) && reader.file(p)
            });
        if is_version {
            let group = path
                .parent()
                .and_then(Path::parent)
                .and_then(|p| p.strip_prefix(&location.path).ok())
                .map(|p| {
                    p.components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(".")
                });
            let mut found = node(
                path.clone(),
                format!("{}:{artifact}", group.unwrap_or_default()),
                Some(version),
            );
            if let Some(records) = reader.text(&path.join("_remote.repositories")) {
                let records: Vec<_> = records
                    .lines()
                    .filter(|l| !l.trim_start().starts_with('#'))
                    .filter_map(|l| l.split_once('='))
                    .collect();
                if records
                    .iter()
                    .any(|(key, _)| key.trim().ends_with('>') && key.trim().starts_with(&prefix))
                {
                    found.origin = Origin::Local;
                } else if children
                    .iter()
                    .filter(|p| {
                        matches!(p.extension().and_then(|s| s.to_str()), Some("pom" | "jar"))
                    })
                    .all(|p| {
                        records.iter().any(|(key, _)| {
                            key.split_once('>')
                                .is_some_and(|(file, repo)| file == name(p) && !repo.is_empty())
                        })
                    })
                {
                    found.origin = Origin::Downloaded;
                }
            }
            nodes.push(found);
        } else if depth < 12 {
            for child in children.into_iter().rev().filter(|p| reader.directory(p)) {
                pending.push((child, depth + 1));
            }
        } else if children.iter().any(|p| reader.directory(p)) {
            reader.incomplete("Maven's directory depth limit was reached.");
        }
        if nodes.len() >= reader::MAX_ROWS {
            reader.incomplete("The Maven entry limit was reached.");
            break;
        }
    }
    nodes
}

fn gradle(location: &CacheLocation, reader: &mut Reader<'_>) -> Vec<Node> {
    let mut out = Vec::new();
    for group in reader.dirs(&location.path.join("caches/modules-2/files-2.1")) {
        for artifact in reader.dirs(&group) {
            for version in reader.dirs(&artifact) {
                // The layout identifies coordinates, not a recoverable origin:
                // a build can resolve these through a local/private repository.
                out.push(node(
                    version.clone(),
                    format!("{}:{}", name(&group), name(&artifact)),
                    Some(name(&version)),
                ));
                if out.len() >= reader::MAX_ROWS {
                    reader.incomplete("The Gradle entry limit was reached.");
                    return out;
                }
            }
        }
    }
    for distribution in reader.dirs(&location.path.join("wrapper/dists")) {
        if name(&distribution).starts_with("gradle-") {
            out.push(node(
                distribution.clone(),
                "Gradle distribution".into(),
                Some(name(&distribution)),
            ));
        }
    }
    for cache in reader.dirs(&location.path.join("caches")) {
        if name(&cache).starts_with("build-cache-") {
            for path in reader
                .children(&cache)
                .into_iter()
                .filter(|p| is_hex(&name(p)) && reader.file(p))
            {
                out.push(node(
                    path.clone(),
                    format!("Gradle build output {}", name(&path)),
                    None,
                ));
                if out.len() >= reader::MAX_ROWS {
                    reader.incomplete("The Gradle entry limit was reached.");
                    return out;
                }
            }
        }
    }
    out
}

fn is_hex(s: &str) -> bool {
    s.len() >= 2 && s.bytes().all(|c| c.is_ascii_hexdigit())
}
fn blobs(root: &Path, label: &str, executable_suffix: bool, reader: &mut Reader<'_>) -> Vec<Node> {
    let mut out = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0)];
    while let Some((path, depth)) = pending.pop() {
        for child in reader.children(&path) {
            let filename = name(&child);
            let digest = if executable_suffix {
                filename.strip_suffix("-exec").unwrap_or(&filename)
            } else {
                &filename
            };
            if !is_hex(digest) && !(depth == 0 && filename.starts_with("sha")) {
                continue;
            }
            if reader.file(&child) {
                out.push(node(
                    child.clone(),
                    format!("{label} {}", name(&child)),
                    None,
                ));
                if out.len() >= reader::MAX_ROWS {
                    reader.incomplete("The Node cache entry limit was reached.");
                    return out;
                }
            } else if reader.directory(&child) && depth < 4 {
                pending.push((child, depth + 1));
            }
        }
    }
    out
}
fn pnpm(location: &CacheLocation, reader: &mut Reader<'_>) -> Vec<Node> {
    let mut out = blobs(&location.path.join("files"), "pnpm content", true, reader);
    for version in reader.dirs(&location.path) {
        if name(&version)
            .strip_prefix('v')
            .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        {
            out.extend(blobs(&version.join("files"), "pnpm content", true, reader));
            if out.len() >= reader::MAX_ROWS {
                out.truncate(reader::MAX_ROWS);
                reader.incomplete("The pnpm entry limit was reached.");
                break;
            }
        }
    }
    out
}
fn yarn(location: &CacheLocation, reader: &mut Reader<'_>) -> Vec<Node> {
    let mut out = Vec::new();
    let mut roots = vec![location.path.clone()];
    roots.extend(
        reader
            .dirs(&location.path)
            .into_iter()
            .filter(|p| name(p).starts_with('v')),
    );
    for root in roots {
        for child in reader.children(&root) {
            let n = name(&child);
            if (n.starts_with("npm-") && reader.directory(&child))
                || (n.ends_with(".zip") && reader.file(&child))
            {
                let mut entry = node(child.clone(), n, None);
                if reader.directory(&child) {
                    let mut packages = Vec::new();
                    for path in reader.dirs(&child.join("node_modules")) {
                        if name(&path).starts_with('@') {
                            packages.extend(reader.dirs(&path));
                        } else {
                            packages.push(path);
                        }
                    }
                    if packages.len() == 1 {
                        if let Some(text) = reader.text(&packages[0].join("package.json")) {
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                if let (Some(package), Some(version)) = (
                                    json.get("name").and_then(|v| v.as_str()),
                                    json.get("version").and_then(|v| v.as_str()),
                                ) {
                                    entry.artifact = package.into();
                                    entry.version = Some(version.into());
                                }
                            }
                        }
                    }
                }
                out.push(entry);
                if out.len() >= reader::MAX_ROWS {
                    reader.incomplete("The Yarn entry limit was reached.");
                    return out;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
