//! Streaming traversal.
//!
//! Properties this walker is required to have:
//!
//! * It never follows symlinks, junctions or other reparse points, so
//!   traversal loops are structurally impossible and a link can never carry
//!   the walk out of the requested roots.
//! * It never descends into a protected path.
//! * It treats macOS bundles and a small set of "leaf" directories as single
//!   opaque objects — reporting them, but not their insides.
//! * A permission error, or a file that vanishes mid-walk, is recorded and
//!   stepped over. Neither aborts anything.
//! * It yields entries one at a time. A directory tree is never materialised
//!   in memory.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use walkdir::WalkDir;

use crate::error::{Hiccup, HiccupKind};
use crate::safety::paths;
use crate::safety::ProtectedPaths;

/// One thing seen during traversal.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub modified_unix: Option<i64>,
    pub accessed_unix: Option<i64>,
    pub created_unix: Option<i64>,
    pub is_dir: bool,
    /// Depth below the scan root this entry came from.
    pub depth: usize,
    /// True when this is a directory Scuttle deliberately did not open.
    pub opaque: bool,
    /// Lowercased file name, and extension without the dot.
    ///
    /// Derived once here rather than by each consumer: several detectors and
    /// the content profiler all want them, and re-lowercasing the same name
    /// six times per file is measurable across a few hundred thousand of them.
    pub name: String,
    pub ext: String,
}

impl FileEntry {
    /// A synthetic entry standing for a whole directory, for detectors that
    /// report a measured tree rather than a file the walk handed them.
    pub fn for_directory(path: &Path, bytes: u64, newest_unix: i64) -> FileEntry {
        FileEntry {
            name: paths::file_name_lower(path),
            ext: paths::extension(path).unwrap_or_default(),
            path: path.to_path_buf(),
            size: bytes,
            modified_unix: Some(newest_unix),
            accessed_unix: None,
            created_unix: None,
            is_dir: true,
            depth: 1,
            opaque: false,
        }
    }

    pub fn name_lower(&self) -> &str {
        &self.name
    }

    /// The lowercased extension, or `None` when there is not one.
    pub fn extension(&self) -> Option<&str> {
        if self.ext.is_empty() {
            None
        } else {
            Some(&self.ext)
        }
    }

    /// Whole days since the file was last modified, from `now`.
    pub fn age_days(&self, now_unix: i64) -> Option<u32> {
        self.modified_unix
            .map(|m| ((now_unix - m).max(0) / 86_400) as u32)
    }

    /// Days since last read *or* write. Access times are unreliable (many
    /// systems mount with `noatime`), so this falls back to modification.
    pub fn idle_days(&self, now_unix: i64) -> Option<u32> {
        let touched = match (self.modified_unix, self.accessed_unix) {
            (Some(m), Some(a)) => Some(m.max(a)),
            (Some(m), None) => Some(m),
            (None, Some(a)) => Some(a),
            (None, None) => None,
        }?;
        Some(((now_unix - touched).max(0) / 86_400) as u32)
    }
}

/// Directories whose contents are never individually interesting. Reported as
/// one object; not opened.
const LEAF_DIRECTORIES: [&str; 6] = [
    "node_modules",
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    ".gradle",
];

/// What the caller wants to do with each entry.
pub enum Step {
    Continue,
    /// Stop the walk entirely — used for cancellation.
    Stop,
}

pub struct WalkOptions {
    pub max_depth: usize,
    /// Entries smaller than this are still counted but not handed to
    /// detectors, which keeps the common case cheap.
    pub report_dirs: bool,
}

impl Default for WalkOptions {
    fn default() -> Self {
        WalkOptions {
            max_depth: 12,
            report_dirs: true,
        }
    }
}

/// Walk `root`, calling `on_entry` for each thing found and `on_hiccup` for
/// each thing that went wrong.
pub fn walk<E, H>(
    root: &Path,
    protected: &ProtectedPaths,
    options: &WalkOptions,
    mut on_entry: E,
    mut on_hiccup: H,
) where
    E: FnMut(FileEntry) -> Step,
    H: FnMut(Hiccup),
{
    let mut it = WalkDir::new(root)
        .follow_links(false)
        .max_depth(options.max_depth)
        .same_file_system(true)
        .into_iter();

    loop {
        let next = match it.next() {
            None => break,
            Some(n) => n,
        };

        let entry = match next {
            Ok(e) => e,
            Err(err) => {
                let path = err
                    .path()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| root.to_path_buf());
                let kind = match err.io_error().map(std::io::Error::kind) {
                    Some(std::io::ErrorKind::PermissionDenied) => HiccupKind::PermissionDenied,
                    Some(std::io::ErrorKind::NotFound) => HiccupKind::Vanished,
                    _ if err.loop_ancestor().is_some() => HiccupKind::LoopAvoided,
                    _ => HiccupKind::Unreadable,
                };
                on_hiccup(Hiccup::new(kind, &path));
                continue;
            }
        };

        let path = entry.path();
        let file_type = entry.file_type();

        // Links are noted and stepped over. Scuttle reasons about real data.
        if file_type.is_symlink() {
            continue;
        }

        // The protected table prunes the walk, not just the results: there is
        // no reason to read the contents of someone's keychain directory.
        if protected.is_protected(path) {
            if file_type.is_dir() {
                it.skip_current_dir();
            }
            continue;
        }

        let is_dir = file_type.is_dir();
        // The leaf rule is about not descending into these during a broad
        // sweep. When the caller points the walker directly at one — to
        // measure a `node_modules` it already found, say — opening it is
        // exactly what was asked for.
        let opaque = is_dir
            && entry.depth() > 0
            && (paths::is_opaque_bundle(path) || is_leaf_directory(path));

        if is_dir && !options.report_dirs && !opaque {
            continue;
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(err) => {
                let kind = match err.io_error().map(std::io::Error::kind) {
                    Some(std::io::ErrorKind::PermissionDenied) => HiccupKind::PermissionDenied,
                    Some(std::io::ErrorKind::NotFound) => HiccupKind::Vanished,
                    _ => HiccupKind::Unreadable,
                };
                on_hiccup(Hiccup::new(kind, path));
                if is_dir {
                    it.skip_current_dir();
                }
                continue;
            }
        };

        let file_entry = FileEntry {
            name: paths::file_name_lower(path),
            ext: paths::extension(path).unwrap_or_default(),
            path: path.to_path_buf(),
            size: if is_dir { 0 } else { meta.len() },
            modified_unix: to_unix(meta.modified().ok()),
            accessed_unix: to_unix(meta.accessed().ok()),
            created_unix: to_unix(meta.created().ok()),
            is_dir,
            depth: entry.depth(),
            opaque,
        };

        let step = on_entry(file_entry);

        if opaque {
            it.skip_current_dir();
        }
        if matches!(step, Step::Stop) {
            break;
        }
    }
}

fn is_leaf_directory(path: &Path) -> bool {
    let name = paths::file_name_lower(path);
    LEAF_DIRECTORIES.contains(&name.as_str())
}

fn to_unix(t: Option<SystemTime>) -> Option<i64> {
    t.and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}

/// Total size of a directory tree, bounded so one pathological folder cannot
/// stall a scan. Returns the bytes counted and whether the count is complete.
pub fn measure_tree(
    root: &Path,
    protected: &ProtectedPaths,
    max_entries: usize,
    cancel: &dyn Fn() -> bool,
) -> (u64, u64, bool) {
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut seen = 0usize;
    // Shared between the two closures below; both may mark the count partial.
    let complete = std::cell::Cell::new(true);

    walk(
        root,
        protected,
        &WalkOptions {
            max_depth: 32,
            report_dirs: false,
        },
        |entry| {
            seen += 1;
            if !entry.is_dir {
                bytes += entry.size;
                files += 1;
            }
            if seen >= max_entries {
                complete.set(false);
                return Step::Stop;
            }
            if cancel() {
                complete.set(false);
                return Step::Stop;
            }
            Step::Continue
        },
        |_| {
            // An unreadable corner means the total is a floor, not a fact.
            complete.set(false);
        },
    );

    (bytes, files, complete.get())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn never() -> impl Fn() -> bool {
        || false
    }

    struct Tree {
        tmp: tempfile::TempDir,
    }

    impl Tree {
        fn new() -> Tree {
            Tree {
                tmp: tempfile::tempdir().unwrap(),
            }
        }
        fn root(&self) -> PathBuf {
            self.tmp.path().to_path_buf()
        }
        fn file(&self, rel: &str, bytes: usize) -> PathBuf {
            let p = self.root().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, vec![b'x'; bytes]).unwrap();
            p
        }
        fn dir(&self, rel: &str) -> PathBuf {
            let p = self.root().join(rel);
            fs::create_dir_all(&p).unwrap();
            p
        }
        fn collect(&self, protected: &ProtectedPaths) -> Vec<FileEntry> {
            let mut out = Vec::new();
            walk(
                &self.root(),
                protected,
                &WalkOptions::default(),
                |e| {
                    out.push(e);
                    Step::Continue
                },
                |_| {},
            );
            out
        }
    }

    fn open_table() -> ProtectedPaths {
        // A home directory that exists nowhere, so no rule matches the tmp tree.
        ProtectedPaths::for_home("/nonexistent-home-for-tests")
    }

    #[test]
    fn a_symlink_loop_does_not_hang_the_walker() {
        let t = Tree::new();
        let a = t.dir("a/b/c");
        let loop_link = a.join("back");
        #[cfg(unix)]
        std::os::unix::fs::symlink(t.root(), &loop_link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(t.root(), &loop_link).unwrap();
        t.file("a/b/c/real.txt", 10);

        // If loop prevention were broken this would never return.
        let entries = t.collect(&open_table());
        assert!(entries.iter().any(|e| e.name_lower() == "real.txt"));
        assert!(
            !entries.iter().any(|e| e.name_lower() == "back"),
            "the link itself must not be reported as data"
        );
    }

    #[test]
    fn a_symlink_cannot_carry_the_walk_outside_the_root() {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "nope").unwrap();

        let t = Tree::new();
        let link = t.root().join("escape");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), &link).unwrap();

        let entries = t.collect(&open_table());
        assert!(
            !entries.iter().any(|e| e.name_lower() == "secret.txt"),
            "the walk escaped its root through a link"
        );
    }

    #[test]
    fn protected_directories_are_not_opened() {
        let t = Tree::new();
        let home = t.dir("home/tester");
        fs::create_dir_all(home.join(".ssh")).unwrap();
        fs::write(home.join(".ssh/id_ed25519"), "PRIVATE").unwrap();
        fs::write(home.join("Downloads_x.dmg"), "ok").unwrap();

        let protected = ProtectedPaths::for_home(&home);
        let entries = t.collect(&protected);
        assert!(!entries.iter().any(|e| e.name_lower() == "id_ed25519"));
        assert!(!entries.iter().any(|e| e.name_lower() == ".ssh"));
        assert!(entries.iter().any(|e| e.name_lower() == "downloads_x.dmg"));
    }

    #[test]
    fn bundles_are_reported_whole_and_never_opened() {
        let t = Tree::new();
        t.file("Apps/Thing.app/Contents/MacOS/thing", 100);
        let entries = t.collect(&open_table());
        let bundle = entries
            .iter()
            .find(|e| e.name_lower() == "thing.app")
            .unwrap();
        assert!(bundle.opaque);
        assert!(!entries.iter().any(|e| e.name_lower() == "contents"));
    }

    #[test]
    fn pointing_the_walker_at_a_leaf_directory_opens_it() {
        // Otherwise nothing could ever measure how big a node_modules is.
        let t = Tree::new();
        t.file("node_modules/left-pad/index.js", 20);
        let mut names = Vec::new();
        walk(
            &t.root().join("node_modules"),
            &open_table(),
            &WalkOptions::default(),
            |e| {
                names.push(e.name_lower().to_string());
                Step::Continue
            },
            |_| {},
        );
        assert!(names.contains(&"index.js".to_string()));
    }

    #[test]
    fn node_modules_is_one_object_not_thirty_thousand() {
        let t = Tree::new();
        t.file("proj/node_modules/left-pad/index.js", 20);
        t.file("proj/src/main.ts", 20);
        let entries = t.collect(&open_table());
        assert!(entries
            .iter()
            .any(|e| e.name_lower() == "node_modules" && e.opaque));
        assert!(!entries.iter().any(|e| e.name_lower() == "left-pad"));
        assert!(entries.iter().any(|e| e.name_lower() == "main.ts"));
    }

    #[test]
    fn an_unreadable_directory_does_not_abort_the_walk() {
        let t = Tree::new();
        t.file("readable/a.txt", 5);
        let locked = t.dir("locked");
        fs::write(locked.join("hidden.txt"), "x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        }
        t.file("also-readable/b.txt", 5);

        let mut hiccups = Vec::new();
        let mut names = Vec::new();
        walk(
            &t.root(),
            &open_table(),
            &WalkOptions::default(),
            |e| {
                names.push(e.name_lower().to_string());
                Step::Continue
            },
            |h| hiccups.push(h),
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(
                hiccups
                    .iter()
                    .any(|h| h.kind == HiccupKind::PermissionDenied),
                "expected a permission hiccup, got {hiccups:?}"
            );
        }
        assert!(names.contains(&"a.txt".to_string()));
        assert!(names.contains(&"b.txt".to_string()));
    }

    #[test]
    fn stopping_early_actually_stops() {
        let t = Tree::new();
        for i in 0..50 {
            t.file(&format!("many/f{i}.txt"), 1);
        }
        let mut count = 0;
        walk(
            &t.root(),
            &open_table(),
            &WalkOptions::default(),
            |_| {
                count += 1;
                if count == 5 {
                    Step::Stop
                } else {
                    Step::Continue
                }
            },
            |_| {},
        );
        assert_eq!(count, 5);
    }

    #[test]
    fn max_depth_is_respected() {
        let t = Tree::new();
        t.file("a/b/c/d/e/deep.txt", 1);
        let mut names = Vec::new();
        walk(
            &t.root(),
            &open_table(),
            &WalkOptions {
                max_depth: 3,
                report_dirs: true,
            },
            |e| {
                names.push(e.name_lower().to_string());
                Step::Continue
            },
            |_| {},
        );
        assert!(!names.contains(&"deep.txt".to_string()));
        assert!(names.contains(&"c".to_string()));
    }

    #[test]
    fn measure_tree_sums_real_files_only() {
        let t = Tree::new();
        t.file("x/a.bin", 1000);
        t.file("x/b.bin", 2000);
        t.dir("x/empty");
        let (bytes, files, complete) =
            measure_tree(&t.root().join("x"), &open_table(), 10_000, &never());
        assert_eq!(bytes, 3000);
        assert_eq!(files, 2);
        assert!(complete);
    }

    #[test]
    fn measure_tree_reports_incompleteness_when_budget_runs_out() {
        let t = Tree::new();
        for i in 0..40 {
            t.file(&format!("x/f{i}.bin"), 10);
        }
        let (_, _, complete) = measure_tree(&t.root().join("x"), &open_table(), 5, &never());
        assert!(!complete, "a truncated measurement must admit it");
    }

    #[test]
    fn age_falls_back_to_modification_when_atime_is_missing() {
        let e = FileEntry {
            name: "x".into(),
            ext: String::new(),
            path: "/x".into(),
            size: 0,
            modified_unix: Some(1_000_000),
            accessed_unix: None,
            created_unix: None,
            is_dir: false,
            depth: 1,
            opaque: false,
        };
        let now = 1_000_000 + 86_400 * 10;
        assert_eq!(e.age_days(now), Some(10));
        assert_eq!(e.idle_days(now), Some(10));
    }

    #[test]
    fn idle_days_uses_the_most_recent_of_read_or_write() {
        let e = FileEntry {
            name: "x".into(),
            ext: String::new(),
            path: "/x".into(),
            size: 0,
            modified_unix: Some(1_000_000),
            accessed_unix: Some(1_000_000 + 86_400 * 9),
            created_unix: None,
            is_dir: false,
            depth: 1,
            opaque: false,
        };
        let now = 1_000_000 + 86_400 * 10;
        assert_eq!(e.age_days(now), Some(10));
        assert_eq!(e.idle_days(now), Some(1));
    }
}
