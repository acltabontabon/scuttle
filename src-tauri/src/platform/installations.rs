//! Recognising an installed application by its shape.
//!
//! The Discord incident taught this module's one rule: *where* a file sits and
//! *what it is called* are not evidence that it is disposable. An `.exe` under
//! `%LOCALAPPDATA%` is, far more often than not, a program somebody uses every
//! day. Uninstall metadata is often missing, processes are often not running,
//! and neither absence proves anything.
//!
//! So this module looks for the structure applications leave behind — a
//! Squirrel updater next to versioned `app-*` folders, an Electron
//! `resources/app.asar`, an executable beside its DLLs, a macOS bundle — and
//! reports that as an installation. Structure is cheap to check, hard to fake
//! by accident, and does not depend on any registry being complete.
//!
//! It is deliberately platform-agnostic: a Squirrel layout means the same thing
//! whichever OS the tests run on, and fixtures never need a real application.

use std::path::{Path, PathBuf};

use crate::safety::paths;

/// What kind of installation a folder looks like. Informational: every kind
/// is treated the same way — as an application, not as a leftover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// A Squirrel-style per-user install: `Update.exe` beside `app-x.y.z`.
    Updater,
    /// An Electron application (`resources/app.asar`).
    Electron,
    /// A folder of versioned application directories.
    Versioned,
    /// An executable with the libraries it loads beside it — installed or
    /// portable, Scuttle cannot tell which and does not need to.
    ExecutableWithLibraries,
    /// A folder carrying its own uninstaller.
    WithUninstaller,
    /// A macOS `.app` bundle.
    Bundle,
    /// Something inside a location the platform reserves for installed
    /// applications (Program Files, `%LOCALAPPDATA%\Programs`, /Applications).
    InstallLocation,
}

impl InstallKind {
    pub fn describe(&self) -> &'static str {
        match self {
            InstallKind::Updater => "an application with its own updater",
            InstallKind::Electron => "an installed desktop application",
            InstallKind::Versioned => "versioned application folders",
            InstallKind::ExecutableWithLibraries => "a program with the libraries it runs on",
            InstallKind::WithUninstaller => "an installed program with its own uninstaller",
            InstallKind::Bundle => "an application bundle",
            InstallKind::InstallLocation => "a folder where applications are installed",
        }
    }
}

/// An application installation found at `root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRoot {
    pub root: PathBuf,
    pub kind: InstallKind,
    /// The folder's own name, as a person would read it.
    pub name: String,
}

/// Does this directory, by itself, look like an application installation?
///
/// Reads the directory once (and at most two small child directories). Links
/// are never followed.
pub fn looks_installed(dir: &Path) -> Option<InstallKind> {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.ends_with(".app") && dir.join("Contents").join("Info.plist").is_file() {
        return Some(InstallKind::Bundle);
    }

    let meta = std::fs::symlink_metadata(dir).ok()?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;

    let mut has_update_exe = false;
    let mut versioned_dirs = 0usize;
    let mut has_packages = false;
    let mut exes = 0usize;
    let mut libraries = 0usize;
    let mut has_uninstaller = false;
    let mut has_resources = false;

    for entry in entries.flatten().take(4096) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let child = entry.file_name().to_string_lossy().to_lowercase();
        if file_type.is_dir() {
            if is_versioned_dir(&child) {
                versioned_dirs += 1;
            } else if child == "packages" {
                has_packages = true;
            } else if child == "resources" {
                has_resources = true;
            }
            continue;
        }
        let ext = Path::new(&child)
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        match ext.as_str() {
            "exe" => {
                exes += 1;
                if child == "update.exe" {
                    has_update_exe = true;
                }
                if child.starts_with("unins") || child.starts_with("uninstall") {
                    has_uninstaller = true;
                }
            }
            "dll" | "pak" | "asar" | "node" => libraries += 1,
            _ => {}
        }
    }

    if has_update_exe && (versioned_dirs > 0 || has_packages) {
        return Some(InstallKind::Updater);
    }
    if has_resources && dir.join("resources").join("app.asar").exists() {
        return Some(InstallKind::Electron);
    }
    if has_uninstaller {
        return Some(InstallKind::WithUninstaller);
    }
    if exes > 0 && libraries > 0 {
        return Some(InstallKind::ExecutableWithLibraries);
    }
    if versioned_dirs > 0 && (exes > 0 || has_packages) {
        return Some(InstallKind::Versioned);
    }
    None
}

/// `app-1.0.9001`, `1.2.3`, `v12.0.1` — the way updaters name each version.
fn is_versioned_dir(name: &str) -> bool {
    let rest = name
        .strip_prefix("app-")
        .or_else(|| name.strip_prefix('v'))
        .unwrap_or(name);
    let mut parts = rest.split('.');
    let first_ok = parts
        .next()
        .is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    let dots = rest.matches('.').count();
    first_ok
        && dots >= 1
        && rest
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// Where applications are known to be installed on this machine: reserved
/// locations (whose immediate children are each an application) and explicit
/// install folders read from uninstall metadata.
#[derive(Debug, Clone, Default)]
pub struct InstallAreas {
    /// Each immediate child is an installed application.
    pub containers: Vec<PathBuf>,
    /// Each of these is an installed application.
    pub explicit: Vec<PathBuf>,
}

/// Knowing of no install locations at all. Structure is still recognised.
pub static NO_INSTALL_AREAS: InstallAreas = InstallAreas {
    containers: Vec::new(),
    explicit: Vec::new(),
};

impl InstallAreas {
    /// The installation that `path` is part of — the path itself, or the
    /// outermost ancestor below `boundary` that looks installed.
    ///
    /// `boundary` is the scanned root: Scuttle never climbs above what it was
    /// asked to look at, but everything below it is checked, so a file three
    /// folders deep inside an application is still recognised as the
    /// application's.
    pub fn enclosing(&self, path: &Path, boundary: Option<&Path>) -> Option<InstallRoot> {
        self.enclosing_with(path, boundary, &looks_installed)
    }

    /// An installation *inside* a directory, found within a few levels. A
    /// folder that contains an application is an application folder, however
    /// it was found.
    pub fn contained(&self, dir: &Path, max_depth: usize) -> Option<InstallRoot> {
        self.contained_with(dir, max_depth, &looks_installed)
    }

    fn enclosing_with(
        &self,
        path: &Path,
        boundary: Option<&Path>,
        looks: &dyn Fn(&Path) -> Option<InstallKind>,
    ) -> Option<InstallRoot> {
        let path = paths::normalize(path);
        for explicit in &self.explicit {
            if paths::is_within(&path, explicit) {
                return Some(root_of(explicit, InstallKind::InstallLocation));
            }
        }
        for container in &self.containers {
            if paths::is_strictly_within(&path, container) {
                // The child of the container that holds `path`.
                let depth = paths::normalize(container).components().count();
                let child: PathBuf = path.components().take(depth + 1).collect();
                return Some(root_of(&child, InstallKind::InstallLocation));
            }
        }

        let mut best = None;
        for ancestor in path.ancestors() {
            if let Some(boundary) = boundary {
                if !paths::is_strictly_within(ancestor, boundary) {
                    break;
                }
            }
            if ancestor.parent().is_none() {
                break;
            }
            if let Some(kind) = looks(ancestor) {
                // Keep climbing: `Discord\app-1.0.9001` is an application
                // folder, but the installation is `Discord`, and moving only
                // part of it breaks it just as thoroughly.
                best = Some(root_of(ancestor, kind));
            }
        }
        best
    }

    fn contained_with(
        &self,
        dir: &Path,
        max_depth: usize,
        looks: &dyn Fn(&Path) -> Option<InstallKind>,
    ) -> Option<InstallRoot> {
        let dir = paths::normalize(dir);
        for explicit in &self.explicit {
            if paths::is_strictly_within(explicit, &dir) {
                return Some(root_of(explicit, InstallKind::InstallLocation));
            }
        }
        for container in &self.containers {
            if paths::is_within(container, &dir) {
                return Some(root_of(container, InstallKind::InstallLocation));
            }
        }
        let walker = walkdir::WalkDir::new(&dir)
            .follow_links(false)
            .max_depth(max_depth)
            .into_iter()
            .filter_entry(|e| e.file_type().is_dir());
        for entry in walker.flatten().take(20_000) {
            if entry.depth() == 0 {
                continue;
            }
            if let Some(kind) = looks(entry.path()) {
                return Some(root_of(entry.path(), kind));
            }
        }
        None
    }
}

/// [`InstallAreas`] with every directory's verdict remembered, for a scan that
/// asks about thousands of files in the same few folders.
#[derive(Debug, Default)]
pub struct InstallIndex {
    pub areas: InstallAreas,
    seen: std::sync::Mutex<std::collections::HashMap<PathBuf, Option<InstallKind>>>,
}

impl InstallIndex {
    pub fn new(areas: InstallAreas) -> InstallIndex {
        InstallIndex {
            areas,
            seen: Default::default(),
        }
    }

    fn looks(&self, dir: &Path) -> Option<InstallKind> {
        if let Some(known) = self.seen.lock().ok().and_then(|m| m.get(dir).copied()) {
            return known;
        }
        let verdict = looks_installed(dir);
        if let Ok(mut map) = self.seen.lock() {
            map.insert(dir.to_path_buf(), verdict);
        }
        verdict
    }

    pub fn enclosing(&self, path: &Path, boundary: Option<&Path>) -> Option<InstallRoot> {
        self.areas
            .enclosing_with(path, boundary, &|dir| self.looks(dir))
    }

    pub fn contained(&self, dir: &Path, max_depth: usize) -> Option<InstallRoot> {
        self.areas
            .contained_with(dir, max_depth, &|d| self.looks(d))
    }

    /// The installation a path belongs to or holds, whichever applies.
    pub fn involved(
        &self,
        path: &Path,
        is_dir: bool,
        boundary: Option<&Path>,
    ) -> Option<InstallRoot> {
        self.enclosing(path, boundary).or_else(|| {
            if is_dir {
                self.contained(path, CONTAINED_DEPTH)
            } else {
                None
            }
        })
    }
}

/// How far below a folder Scuttle looks for an application inside it.
pub const CONTAINED_DEPTH: usize = 3;

fn root_of(path: &Path, kind: InstallKind) -> InstallRoot {
    InstallRoot {
        root: path.to_path_buf(),
        kind,
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
    }
}

/// Did this file arrive from the internet? Windows records it in a
/// `Zone.Identifier` alternate data stream, macOS in the
/// `com.apple.quarantine` extended attribute. `None` means "could not tell",
/// which is never evidence either way.
pub fn downloaded_from_internet(path: &Path) -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        match std::fs::read_to_string(PathBuf::from(stream)) {
            Ok(contents) => Some(contents.contains("ZoneId=3") || contents.contains("ZoneId=4")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
            Err(_) => None,
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
        let name = CString::new("com.apple.quarantine").ok()?;
        // SAFETY: both strings are valid NUL-terminated C strings; a null
        // buffer with size 0 only asks for the attribute's length.
        let len = unsafe {
            libc::getxattr(
                c_path.as_ptr(),
                name.as_ptr(),
                std::ptr::null_mut(),
                0,
                0,
                libc::XATTR_NOFOLLOW,
            )
        };
        if len >= 0 {
            Some(true)
        } else {
            match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::ENOATTR) => Some(false),
                _ => None,
            }
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0u8; bytes]).unwrap();
    }

    /// A synthetic Squirrel install, shaped like Discord's. No real
    /// application is involved.
    fn discord_like(local: &Path) -> PathBuf {
        let root = local.join("Discord");
        touch(&root.join("Update.exe"), 2048);
        touch(&root.join("app-1.0.9001").join("Discord.exe"), 4096);
        touch(&root.join("app-1.0.9001").join("ffmpeg.dll"), 1024);
        touch(&root.join("app-1.0.9000").join("Discord.exe"), 4096);
        touch(
            &root.join("packages").join("Discord-1.0.9001-full.nupkg"),
            4096,
        );
        touch(&root.join("packages").join("DiscordSetup.exe"), 4096);
        root
    }

    #[test]
    fn a_squirrel_install_is_recognised_as_one_application() {
        let tmp = tempfile::tempdir().unwrap();
        let root = discord_like(tmp.path());
        assert_eq!(looks_installed(&root), Some(InstallKind::Updater));

        let areas = InstallAreas::default();
        for inner in [
            root.join("Update.exe"),
            root.join("app-1.0.9001").join("Discord.exe"),
            root.join("app-1.0.9000").join("Discord.exe"),
            root.join("packages").join("DiscordSetup.exe"),
        ] {
            let found = areas
                .enclosing(&inner, Some(tmp.path()))
                .unwrap_or_else(|| panic!("{} was not recognised", inner.display()));
            assert_eq!(found.root, root, "the whole install, not a piece of it");
        }
    }

    #[test]
    fn a_parent_containing_an_install_knows_it() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("Local");
        let root = discord_like(&local);
        let found = InstallAreas::default().contained(&local, 3).unwrap();
        assert_eq!(found.root, root);
    }

    #[test]
    fn a_lone_download_is_not_an_installation() {
        let tmp = tempfile::tempdir().unwrap();
        let downloads = tmp.path().join("Downloads");
        touch(&downloads.join("DiscordSetup.exe"), 4096);
        touch(&downloads.join("notes.txt"), 10);
        assert_eq!(looks_installed(&downloads), None);
        assert!(InstallAreas::default()
            .enclosing(&downloads.join("DiscordSetup.exe"), Some(tmp.path()))
            .is_none());
    }

    #[test]
    fn a_portable_app_in_downloads_is_an_application() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = tmp.path().join("Downloads").join("PortableTool");
        touch(&tool.join("tool.exe"), 4096);
        touch(&tool.join("Qt5Core.dll"), 4096);
        let found = InstallAreas::default()
            .enclosing(&tool.join("tool.exe"), Some(&tmp.path().join("Downloads")))
            .unwrap();
        assert_eq!(found.root, tool);
        assert_eq!(found.kind, InstallKind::ExecutableWithLibraries);
    }

    #[test]
    fn install_locations_claim_their_children() {
        let tmp = tempfile::tempdir().unwrap();
        let programs = tmp.path().join("Programs");
        let inner = programs.join("SomeEditor").join("bin").join("editor.exe");
        touch(&inner, 10);
        let areas = InstallAreas {
            containers: vec![programs.clone()],
            explicit: vec![],
        };
        assert_eq!(
            areas.enclosing(&inner, None).unwrap().root,
            programs.join("SomeEditor")
        );
    }

    #[test]
    fn a_macos_bundle_is_an_application() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Tool.app");
        touch(&app.join("Contents").join("Info.plist"), 10);
        assert_eq!(looks_installed(&app), Some(InstallKind::Bundle));
    }

    #[test]
    fn version_names() {
        assert!(is_versioned_dir("app-1.0.9001"));
        assert!(is_versioned_dir("1.2.3"));
        assert!(is_versioned_dir("v12.0"));
        assert!(!is_versioned_dir("apple"));
        assert!(!is_versioned_dir("cache"));
        assert!(!is_versioned_dir("12"));
    }

    #[cfg(unix)]
    #[test]
    fn links_are_never_followed_into_an_install() {
        let tmp = tempfile::tempdir().unwrap();
        let real = discord_like(&tmp.path().join("elsewhere"));
        let scanned = tmp.path().join("scanned");
        fs::create_dir_all(&scanned).unwrap();
        std::os::unix::fs::symlink(&real, scanned.join("link")).unwrap();
        assert_eq!(looks_installed(&scanned.join("link")), None);
    }
}
