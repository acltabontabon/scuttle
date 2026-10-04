//! Read-only verification shared by developer detection and the move gate.
//! A tree snapshot is metadata, not a promise that every byte is disposable.
use crate::{
    quarantine::fsx::{self, EntryKind},
    safety::{paths, ProtectedPaths},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_ENTRIES: usize = 200_000;
const MAX_DEPTH: usize = 64;
const BUDGET: Duration = Duration::from_secs(5);
const MARKER_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Cargo,
    Next,
    Dotnet,
    Dependency,
    Ambiguous,
}
impl ArtifactKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cargo => "Cargo build output",
            Self::Next => "Next.js build output",
            Self::Dotnet => ".NET intermediate output",
            Self::Dependency => "Dependencies",
            Self::Ambiguous => "Possible build output",
        }
    }
    fn processes(self) -> &'static [&'static str] {
        match self {
            Self::Cargo => &["cargo", "rustc", "rustdoc"],
            Self::Next => &["node", "next"],
            Self::Dotnet => &["dotnet", "msbuild", "vbcscompiler"],
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeState {
    pub digest: String,
    pub bytes: u64,
    pub newest_unix: Option<i64>,
    pub complete: bool,
    #[serde(default)]
    pub user_content: bool,
}
impl TreeState {
    pub fn age(&self, now: i64) -> Option<u32> {
        self.newest_unix.map(|t| ((now - t).max(0) / 86400) as u32)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryState {
    NotRepository,
    Ignored,
    NotIgnored,
    Tracked,
    Unknown,
}

/// Persisted in evidence so legacy findings cannot inherit new permissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeveloperArtifact {
    pub version: u32,
    pub project: PathBuf,
    pub boundary: PathBuf,
    pub artifact_kind: ArtifactKind,
    pub manifest: String,
    pub marker_digest: Option<String>,
    pub source: TreeState,
    pub output: TreeState,
    pub repository: RepositoryState,
    pub processes_clear: bool,
    pub stale_days: u32,
    pub preliminary: bool,
}
impl DeveloperArtifact {
    pub fn verified(&self, now: i64) -> bool {
        self.version == 1
            && !self.preliminary
            && self.marker_digest.is_some()
            && matches!(
                self.artifact_kind,
                ArtifactKind::Cargo | ArtifactKind::Next | ArtifactKind::Dotnet
            )
            && matches!(
                self.repository,
                RepositoryState::NotRepository | RepositoryState::Ignored
            )
            && self.processes_clear
            && self.source.complete
            && self.output.complete
            && !self.output.user_content
            && self.source.age(now).is_some_and(|d| d >= self.stale_days)
            && self.output.age(now).is_some_and(|d| d >= self.stale_days)
    }
    pub fn caution(&self, now: i64) -> String {
        if self.preliminary {
            return "Preliminary finding. Run a full rummage to verify this build folder.".into();
        }
        if self.repository == RepositoryState::Tracked {
            return "This folder contains version-controlled files.".into();
        }
        if !self.source.complete || !self.output.complete {
            return "Some of this project could not be checked; its size or activity is incomplete.".into();
        }
        if self.output.user_content {
            return "This output folder may contain files you made. Review it yourself.".into();
        }
        if self.repository == RepositoryState::Unknown {
            return "Repository safety could not be verified.".into();
        }
        if self.marker_digest.is_none() {
            return "The folder has not been verified as rebuildable output. It may contain local changes.".into();
        }
        if self.repository == RepositoryState::NotIgnored {
            return "The repository does not ignore this output folder.".into();
        }
        if !self.processes_clear {
            return "A relevant tool may be running, or running tools could not be checked.".into();
        }
        if !self.verified(now) {
            return format!("The project or its output changed within {} days, or its modification time is unavailable.", self.stale_days);
        }
        String::new()
    }
}

/// Names only nominate candidates. Manifests must be real sibling files.
pub fn identify(path: &Path) -> Option<(ArtifactKind, String)> {
    let project = path.parent()?;
    let name = paths::file_name_lower(path);
    let (kind, manifests): (_, &[&str]) = match name.as_str() {
        "target" => (ArtifactKind::Cargo, &["Cargo.toml", "pom.xml"]),
        ".next" => (
            ArtifactKind::Next,
            &["package.json", "next.config.js", "next.config.mjs"],
        ),
        "obj" => (
            ArtifactKind::Dotnet,
            &["*.csproj", "*.fsproj", "*.vbproj", "*.sln"],
        ),
        "node_modules" => (ArtifactKind::Dependency, &["package.json"]),
        "pods" => (ArtifactKind::Dependency, &["Podfile"]),
        ".venv" => (
            ArtifactKind::Dependency,
            &["pyproject.toml", "requirements.txt", "setup.py"],
        ),
        "vendor" => (
            ArtifactKind::Dependency,
            &["composer.json", "go.mod", "Gemfile"],
        ),
        "build" => (
            ArtifactKind::Ambiguous,
            &[
                "build.gradle",
                "build.gradle.kts",
                "CMakeLists.txt",
                "pom.xml",
            ],
        ),
        "dist" => (
            ArtifactKind::Ambiguous,
            &["package.json", "pyproject.toml", "setup.py"],
        ),
        ".nuxt" => (
            ArtifactKind::Ambiguous,
            &["package.json", "nuxt.config.ts", "nuxt.config.js"],
        ),
        ".parcel-cache" => (ArtifactKind::Ambiguous, &["package.json"]),
        ".gradle" => (
            ArtifactKind::Ambiguous,
            &[
                "build.gradle",
                "build.gradle.kts",
                "settings.gradle",
                "settings.gradle.kts",
            ],
        ),
        _ => return None,
    };
    for manifest in manifests {
        let found = if let Some(suffix) = manifest.strip_prefix('*') {
            fs::read_dir(project)
                .ok()?
                .take(4096)
                .filter_map(Result::ok)
                .find(|e| e.file_name().to_string_lossy().ends_with(suffix) && real_file(&e.path()))
                .map(|e| e.file_name().to_string_lossy().into_owned())
        } else {
            real_file(&project.join(manifest)).then(|| manifest.to_string())
        };
        if let Some(manifest) = found {
            let kind = if manifest == "pom.xml" || manifest.ends_with(".sln") {
                ArtifactKind::Ambiguous
            } else {
                kind
            };
            return Some((kind, manifest));
        }
    }
    None
}
fn real_file(path: &Path) -> bool {
    fsx::identity_of(path).is_ok_and(|m| m.kind == EntryKind::File)
}
fn real_dir(path: &Path) -> bool {
    fsx::identity_of(path).is_ok_and(|m| m.kind == EntryKind::Dir)
}

fn marker(path: &Path) -> Option<Vec<u8>> {
    // The inspection boundary checks ancestors. The marker and its immediate
    // parent must also be real, and the open is bound to the observed identity.
    if !real_dir(path.parent()?) {
        return None;
    }
    let before = fsx::identity_of(path).ok()?;
    if before.kind != EntryKind::File || before.size > MARKER_LIMIT {
        return None;
    }
    let file = fsx::open_no_follow(path).ok()?;
    if !before.is_same_state(&fsx::identity_of_file(&file, path).ok()?) {
        return None;
    }
    let mut data = Vec::new();
    file.take(MARKER_LIMIT + 1).read_to_end(&mut data).ok()?;
    if data.len() as u64 > MARKER_LIMIT || !before.is_same_state(&fsx::identity_of(path).ok()?) {
        return None;
    }
    Some(data)
}
fn json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&marker(path)?).ok()
}
fn verify_markers(path: &Path, kind: ArtifactKind, manifest: &str) -> Option<String> {
    let project = path.parent()?;
    let mut hash = blake3::Hasher::new();
    hash.update(&marker(&project.join(manifest))?);
    let names: &[&str] = match kind {
        ArtifactKind::Cargo => {
            let manifest_bytes = marker(&project.join(manifest))?;
            let manifest: toml::Value =
                toml::from_str(std::str::from_utf8(&manifest_bytes).ok()?).ok()?;
            if !manifest.get("package").is_some_and(|v| v.is_table())
                && !manifest.get("workspace").is_some_and(|v| v.is_table())
            {
                return None;
            }
            let tag = marker(&path.join("CACHEDIR.TAG"))?;
            if !tag.starts_with(b"Signature: 8a477f597d28d172789f06886806bc55")
                || !String::from_utf8_lossy(&tag).contains("cargo")
            {
                return None;
            }
            // Cargo profile layouts, including cross-compiled profiles.
            let mut profiles = vec![path.join("debug"), path.join("release")];
            for entry in fs::read_dir(path).ok()?.take(512).filter_map(Result::ok) {
                if real_dir(&entry.path()) {
                    profiles.push(entry.path().join("debug"));
                    profiles.push(entry.path().join("release"));
                }
            }
            if !profiles.iter().any(|p| {
                real_dir(p) && real_dir(&p.join(".fingerprint")) && real_dir(&p.join("deps"))
            }) {
                return None;
            }
            &["CACHEDIR.TAG"]
        }
        ArtifactKind::Next => {
            let package = json(&project.join(manifest))?;
            if package
                .pointer("/dependencies/next")
                .and_then(|v| v.as_str())
                .is_none()
                && package
                    .pointer("/devDependencies/next")
                    .and_then(|v| v.as_str())
                    .is_none()
            {
                return None;
            }
            if marker(&path.join("BUILD_ID"))?.is_empty()
                || !json(&path.join("build-manifest.json"))?.is_object()
            {
                return None;
            }
            &["BUILD_ID", "build-manifest.json"]
        }
        ArtifactKind::Dotnet => {
            let assets = json(&path.join("project.assets.json"))?;
            if assets.get("version").and_then(|v| v.as_u64()).is_none()
                || !assets.get("targets")?.is_object()
                || !assets.get("libraries")?.is_object()
            {
                return None;
            }
            let owner = assets.pointer("/project/restore/projectPath")?.as_str()?;
            if paths::folded_components(Path::new(owner))
                != paths::folded_components(&project.join(manifest))
            {
                return None;
            }
            &["project.assets.json"]
        }
        _ => return None,
    };
    for name in names {
        hash.update(name.as_bytes());
        hash.update(&marker(&path.join(name))?);
    }
    Some(hash.finalize().to_hex().to_string())
}

/// Discover repository ownership without recursively walking its internals.
/// Failure and unsupported VCS markers are kept distinct from "no repository".
pub(crate) fn repository_boundary(project: &Path) -> Result<Option<PathBuf>, ()> {
    for ancestor in project.ancestors() {
        for name in [".hg", ".svn", ".jj", ".git"] {
            match fs::symlink_metadata(ancestor.join(name)) {
                Ok(_) if name == ".git" => return Ok(Some(ancestor.to_path_buf())),
                Ok(_) => return Err(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(()),
            }
        }
    }
    Ok(None)
}

/// Only reusable metadata is cached; live actions always start with an empty cache.
#[derive(Default)]
pub struct InspectionCache {
    sources: HashMap<PathBuf, TreeState>,
    repositories: HashMap<PathBuf, bool>,
}

// Git libraries legitimately resolve worktree pointers and object paths. Before
// giving one a repository, reject linked/protected metadata and external object
// alternates so the narrow Git exception cannot open unrelated private files.
fn git_metadata_safe(root: &Path, protected: &ProtectedPaths, cancel: &dyn Fn() -> bool) -> bool {
    fn resolve(root: &Path, protected: &ProtectedPaths) -> Option<Vec<PathBuf>> {
        let marker_path = root.join(".git");
        let git_dir = if real_dir(&marker_path) {
            marker_path
        } else {
            let bytes = marker(&marker_path)?;
            let text = std::str::from_utf8(&bytes).ok()?.trim();
            let path = Path::new(text.strip_prefix("gitdir: ")?);
            paths::normalize(&root.join(path))
        };
        let base = paths::link_check_base(&git_dir, Some(protected.home()));
        if paths::first_link_below(&git_dir, &base).is_some()
            || protected
                .rule_for(&git_dir)
                .is_some_and(|r| r.name != "Repository internals")
            || !real_dir(&git_dir)
        {
            return None;
        }
        let mut dirs = vec![git_dir.clone()];
        match fs::symlink_metadata(git_dir.join("commondir")) {
            Ok(_) => {
                let bytes = marker(&git_dir.join("commondir"))?;
                let common = std::str::from_utf8(&bytes).ok()?.trim();
                dirs.push(paths::normalize(&git_dir.join(common)));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
        Some(dirs)
    }
    let Some(dirs) = resolve(root, protected) else {
        return false;
    };
    let start = Instant::now();
    let mut seen = 0;
    for dir in crate::scanning::roots::deduplicate(dirs) {
        let base = paths::link_check_base(&dir, Some(protected.home()));
        if paths::first_link_below(&dir, &base).is_some() || !real_dir(&dir) {
            return false;
        }
        for item in walkdir::WalkDir::new(&dir)
            .follow_links(false)
            .follow_root_links(false)
            .same_file_system(true)
            .max_depth(MAX_DEPTH)
        {
            seen += 1;
            if seen > MAX_ENTRIES || start.elapsed() >= BUDGET || cancel() {
                return false;
            }
            let Ok(item) = item else {
                return false;
            };
            let path = item.path();
            if protected
                .rule_for(path)
                .is_some_and(|r| r.name != "Repository internals")
            {
                return false;
            }
            let Ok(identity) = fsx::identity_of(path) else {
                return false;
            };
            if !matches!(identity.kind, EntryKind::File | EntryKind::Dir)
                || item.depth() == MAX_DEPTH
            {
                return false;
            }
            if path.ends_with("objects/info/alternates")
                || path.ends_with("objects/info/http-alternates")
            {
                return false;
            }
            if matches!(
                paths::file_name_lower(path).as_str(),
                "config" | "index" | "packed-refs"
            ) && identity.size > 32 * 1024 * 1024
            {
                return false;
            }
        }
    }
    true
}

pub fn repository_state(root: &Path, output: &Path) -> RepositoryState {
    fn check(root: &Path, output: &Path) -> Option<RepositoryState> {
        if fsx::identity_of(&root.join(".git")).ok()?.kind == EntryKind::Link {
            return None;
        }
        let repo = gix::open::Options::isolated()
            .config_overrides(["core.excludesFile=", "core.attributesFile="])
            .open(root)
            .ok()?
            .to_thread_local();
        if repo.is_bare()
            || !paths::is_within(repo.workdir()?, root)
            || !paths::is_within(root, repo.workdir()?)
        {
            return None;
        }
        let relative = output.strip_prefix(root).ok()?;
        let index = repo.index_or_empty().ok()?;
        let tracked = |state: &gix::index::State| {
            state.entries().iter().any(|e| {
                let name = gix::path::from_bstr(e.path(state));
                paths::is_within(&name, relative)
                    || (e.mode == gix::index::entry::Mode::COMMIT
                        && paths::is_within(relative, &name))
            })
        };
        if index.entries().len() > MAX_ENTRIES {
            return None;
        }
        if tracked(&index) {
            return Some(RepositoryState::Tracked);
        }
        let head = repo.head().ok()?;
        if !head.is_unborn() {
            // Use the same platform-aware path comparison for HEAD and the
            // index, including a staged deletion whose casing changed on disk.
            let tree = repo.head_tree_id().ok()?;
            let head_index = repo.index_from_tree(&tree).ok()?;
            if head_index.entries().len() > MAX_ENTRIES {
                return None;
            }
            if tracked(&head_index) {
                return Some(RepositoryState::Tracked);
            }
        }
        // Reject linked ignore files before handing paths to the library.
        for parent in output.ancestors().take_while(|p| paths::is_within(p, root)) {
            let ignore = parent.join(".gitignore");
            match fsx::identity_of(&ignore) {
                Ok(i) if i.kind != EntryKind::File || i.size > MARKER_LIMIT => return None,
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return None,
                _ => {}
            }
        }
        let mut ignores = repo.excludes(&index, None, Default::default()).ok()?;
        let ignored = ignores
            .at_path(relative, Some(gix::index::entry::Mode::DIR))
            .ok()?
            .is_excluded();
        Some(if ignored {
            RepositoryState::Ignored
        } else {
            RepositoryState::NotIgnored
        })
    }
    check(root, output).unwrap_or(RepositoryState::Unknown)
}

/// Bounded metadata snapshot. Source traversal excludes identified generated
/// trees, but arbitrary folders called `build` are still inspected.
pub fn tree(
    root: &Path,
    protected: &ProtectedPaths,
    source: bool,
    cancel: &dyn Fn() -> bool,
) -> TreeState {
    tree_with_mode(root, protected, source, false, cancel)
}
fn tree_with_mode(
    root: &Path,
    protected: &ProtectedPaths,
    source: bool,
    read_markers: bool,
    cancel: &dyn Fn() -> bool,
) -> TreeState {
    let mut out = TreeState {
        complete: true,
        ..Default::default()
    };
    let mut hash = blake3::Hasher::new();
    let started = Instant::now();
    let mut walk = walkdir::WalkDir::new(root)
        .follow_links(false)
        .follow_root_links(false)
        .same_file_system(true)
        .max_depth(MAX_DEPTH)
        .sort_by_file_name()
        .into_iter();
    let root_volume = fsx::identity_of(root)
        .ok()
        .and_then(|i| i.file_id)
        .and_then(|s| s.split_once(':').map(|(v, _)| v.to_owned()));
    let mut seen = 0;
    while let Some(entry) = walk.next() {
        seen += 1;
        if seen > MAX_ENTRIES || started.elapsed() >= BUDGET || cancel() {
            out.complete = false;
            break;
        }
        let Ok(entry) = entry else {
            out.complete = false;
            continue;
        };
        let path = entry.path();
        let name = paths::file_name_lower(path);
        if source && name == ".git" {
            // A nested repository is not covered by the enclosing repository check.
            if path.parent() != Some(root) {
                out.complete = false;
            }
            if entry.file_type().is_dir() {
                walk.skip_current_dir();
            }
            continue;
        }
        if protected.is_protected(path) {
            out.complete = false;
            if entry.file_type().is_dir() {
                walk.skip_current_dir();
            }
            continue;
        }
        let Ok(identity) = fsx::identity_of(path) else {
            out.complete = false;
            continue;
        };
        if !matches!(identity.kind, EntryKind::File | EntryKind::Dir) {
            out.complete = false;
            if entry.file_type().is_dir() {
                walk.skip_current_dir();
            }
            continue;
        }
        if source && entry.depth() > 0 && identity.kind == EntryKind::Dir {
            if let Some((kind, manifest)) = identify(path) {
                if kind == ArtifactKind::Dependency
                    || (read_markers && verify_markers(path, kind, &manifest).is_some())
                {
                    walk.skip_current_dir();
                    continue;
                }
            }
        }
        if root_volume.as_deref().is_some_and(|v| {
            identity
                .file_id
                .as_deref()
                .and_then(|id| id.split_once(':'))
                .is_some_and(|(current, _)| current != v)
        }) {
            out.complete = false;
            if entry.file_type().is_dir() {
                walk.skip_current_dir();
            }
            continue;
        }
        if !source && identity.kind == EntryKind::File {
            let ext = paths::extension(path).unwrap_or_default();
            if [
                "md", "txt", "pdf", "doc", "docx", "blend", "psd", "sav", "save", "bak",
            ]
            .contains(&ext.as_str())
                || (entry.depth() == 1
                    && ["rs", "ts", "tsx", "py", "cs", "c", "cpp"].contains(&ext.as_str()))
            {
                out.user_content = true;
            }
        }
        if identity.kind == EntryKind::Dir
            && (entry.depth() == MAX_DEPTH || paths::is_opaque_bundle(path))
        {
            out.complete = false;
            walk.skip_current_dir();
        }
        let Some(relative) = path.strip_prefix(root).ok().and_then(|p| p.to_str()) else {
            out.complete = false;
            continue;
        };
        hash.update(relative.as_bytes());
        hash.update(&[0]);
        if source && identity.kind == EntryKind::Dir {
            // Moving a verified output folder changes its parent's directory
            // mtime. Source paths and identities still detect additions,
            // removals and replacements without invalidating sibling output.
            hash.update(format!("dir:{:?}", identity.file_id).as_bytes());
            continue;
        }
        hash.update(format!("{identity:?}").as_bytes());
        if identity.kind == EntryKind::File {
            out.bytes = out.bytes.saturating_add(identity.size);
        }
        match identity.mtime_ns {
            Some(t) => out.newest_unix = Some(out.newest_unix.unwrap_or(0).max(t / 1_000_000_000)),
            None => out.complete = false,
        }
    }
    out.digest = hash.finalize().to_hex().to_string();
    out
}

#[allow(clippy::too_many_arguments)]
pub fn inspect(
    path: &Path,
    protected: &ProtectedPaths,
    roots: &[PathBuf],
    now_processes: &[String],
    stale_days: u32,
    preliminary: bool,
    cancel: &dyn Fn() -> bool,
    cache: &mut InspectionCache,
) -> Option<DeveloperArtifact> {
    let root = roots
        .iter()
        .filter(|r| paths::is_within(path, r))
        .max_by_key(|r| r.components().count())?;
    if paths::first_link_below(path, root).is_some() || !real_dir(root) || !real_dir(path) {
        return None;
    }
    let (artifact_kind, manifest) = identify(path)?;
    let project = path.parent()?.to_path_buf();
    let ownership = if preliminary {
        Err(())
    } else {
        repository_boundary(&project)
    };
    let boundary = ownership
        .as_ref()
        .ok()
        .and_then(|p| p.clone())
        .unwrap_or_else(|| project.clone());
    let within = roots.iter().any(|r| paths::is_within(&boundary, r));
    let repository = match &ownership {
        Ok(Some(root)) if within => {
            let safe = *cache
                .repositories
                .entry(root.clone())
                .or_insert_with(|| git_metadata_safe(root, protected, cancel));
            if safe {
                repository_state(root, path)
            } else {
                RepositoryState::Unknown
            }
        }
        Ok(None) => RepositoryState::NotRepository,
        _ => RepositoryState::Unknown,
    };
    let source = if within {
        cache
            .sources
            .entry(boundary.clone())
            .or_insert_with(|| tree_with_mode(&boundary, protected, true, !preliminary, cancel))
            .clone()
    } else {
        TreeState::default()
    };
    let output = tree(path, protected, false, cancel);
    let marker_digest = if preliminary {
        None
    } else {
        verify_markers(path, artifact_kind, &manifest)
    };
    let processes_clear = !preliminary
        && !now_processes.is_empty()
        && !now_processes.iter().any(|p| {
            let p = p.to_lowercase();
            artifact_kind
                .processes()
                .iter()
                .any(|tool| p.contains(tool))
        });
    Some(DeveloperArtifact {
        version: 1,
        project,
        boundary,
        artifact_kind,
        manifest,
        marker_digest,
        source,
        output,
        repository,
        processes_clear,
        stale_days,
        preliminary,
    })
}

/// Run after path authorization and immediately before the existing move gate.
pub fn revalidate(
    candidate: &crate::model::CleanupCandidate,
    ctx: &crate::safety::ActionContext<'_>,
) -> crate::Result<()> {
    use crate::ScuttleError;
    let stale = || {
        ScuttleError::Stale("The project or its build output changed or could not be verified. Rummage again before moving it.".into())
    };
    let old = candidate.developer_artifact().ok_or_else(stale)?;
    if old.preliminary {
        return Err(stale());
    }
    let processes = ctx
        .developer_platform
        .map(|p| p.running_processes())
        .unwrap_or_default();
    let current = inspect(
        &candidate.path,
        ctx.protected,
        ctx.allowed_roots,
        &processes,
        old.stale_days,
        false,
        &|| false,
        &mut InspectionCache::default(),
    )
    .ok_or_else(stale)?;
    if current.repository == RepositoryState::Tracked {
        return Err(ScuttleError::Refused(
            "This folder now contains version-controlled files. Scuttle will not move it.".into(),
        ));
    }
    if !current.output.complete
        || !current.source.complete
        || current.project != old.project
        || current.boundary != old.boundary
        || current.marker_digest != old.marker_digest
        || current.source != old.source
        || current.output != old.output
        || current.repository != old.repository
        || current.artifact_kind != old.artifact_kind
    {
        return Err(stale());
    }
    // A previously suggested item must still earn both suggestion and expiry.
    if old.verified(chrono::Utc::now().timestamp())
        && !current.verified(chrono::Utc::now().timestamp())
    {
        return Err(stale());
    }
    Ok(())
}
