//! Reading the shape of a directory without reading its contents.
//!
//! Scuttle classifies directories from filenames, extensions and sizes — never
//! from the bytes inside documents. That is a privacy position as much as a
//! performance one: knowing a folder holds seventeen `.sav` files is enough to
//! be careful, and opening them would tell us nothing more useful.

use std::path::{Path, PathBuf};

use super::walk::{self, FileEntry, Step, WalkOptions};
use crate::safety::ProtectedPaths;

/// How many entries are examined before the profile is called good enough.
const MAX_ENTRIES: usize = 6000;

/// What a directory appears to contain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentProfile {
    pub bytes: u64,
    pub files: u64,
    /// Files that look machine-generated: caches, shaders, blobs, logs.
    pub generated: u64,
    /// Files that look like saved progress or profiles.
    pub save_data: u64,
    /// Files that look authored: documents, projects, media with real names.
    pub authored: u64,
    /// Modifications are the only timestamps trusted here.
    pub newest_unix: i64,
    /// False when the walk was truncated or hit something unreadable.
    pub complete: bool,
}

impl ContentProfile {
    pub fn is_empty(&self) -> bool {
        self.files == 0
    }

    /// The share of examined files that look generated.
    pub fn generated_ratio(&self) -> f32 {
        if self.files == 0 {
            return 0.0;
        }
        self.generated as f32 / self.files as f32
    }

    /// Enough generated content that calling it "application data" is fair.
    pub fn looks_generated(&self) -> bool {
        self.files >= 3 && self.generated_ratio() >= 0.6 && self.authored == 0
    }

    /// A short phrase naming what tipped the balance, for evidence text.
    pub fn generated_reason(&self) -> String {
        let percent = (self.generated_ratio() * 100.0).round() as u32;
        format!("{percent}% of it is cache and log files")
    }
}

/// Filename fragments and extensions that mean "a program wrote this".
const GENERATED_EXTENSIONS: [&str; 16] = [
    "cache", "log", "tmp", "temp", "bin", "dat", "pak", "blob", "idx", "lock", "pid", "crash",
    "dmp", "etl", "shader", "spv",
];
const GENERATED_FRAGMENTS: [&str; 10] = [
    "cache",
    "shader",
    "gpucache",
    "code cache",
    "logs",
    "crashpad",
    "service worker",
    "indexeddb",
    "localstorage",
    "tmp",
];

/// Filename fragments and extensions that mean "losing this would hurt".
const SAVE_EXTENSIONS: [&str; 8] = [
    "sav", "save", "savegame", "profile", "slot", "bak", "ess", "dsv",
];
const SAVE_FRAGMENTS: [&str; 9] = [
    "save",
    "saves",
    "savegame",
    "savedata",
    "profile",
    "playerdata",
    "characters",
    "worlds",
    "screenshots",
];

/// Extensions that mean a person made this.
const AUTHORED_EXTENSIONS: [&str; 24] = [
    "doc", "docx", "odt", "rtf", "pages", "pdf", "xls", "xlsx", "numbers", "ppt", "pptx", "key",
    "psd", "ai", "sketch", "fig", "indd", "aep", "blend", "md", "txt", "csv", "epub", "mobi",
];

/// What a single file looks like.
///
/// Worked out once per file and then folded into every directory that contains
/// it. Deriving this inside the per-directory loop meant re-lowercasing every
/// path component nine times over, four levels deep — which is what it cost
/// before anyone measured it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileClass {
    pub size: u64,
    pub modified_unix: i64,
    pub kind: FileKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// Saved progress, profiles, worlds. Losing these would hurt.
    SaveData,
    /// Documents and project files: somebody made this.
    Authored,
    /// Caches, logs, shader blobs: a program made this.
    Generated,
    /// None of the above.
    Ordinary,
}

impl FileClass {
    pub fn of(entry: &FileEntry) -> FileClass {
        let name = entry.name_lower();
        let ext = entry.extension().unwrap_or_default();

        // One pass over the components, comparing without allocating: an
        // `OsStr` that is valid UTF-8 borrows rather than copies, and
        // `eq_ignore_ascii_case` needs no lowercased copy to compare against.
        let in_save_area = entry.path.components().any(|component| {
            let segment = component.as_os_str().to_string_lossy();
            SAVE_FRAGMENTS
                .iter()
                .any(|fragment| segment.eq_ignore_ascii_case(fragment))
        });

        let generated = GENERATED_EXTENSIONS.contains(&ext)
            || GENERATED_FRAGMENTS.iter().any(|fragment| name.contains(*fragment))
            // A long hexadecimal name with no extension is a content-addressed
            // blob: a cache, by any other name.
            || (ext.is_empty() && name.len() > 24 && name.chars().all(|c| c.is_ascii_hexdigit()));

        let kind = if in_save_area || SAVE_EXTENSIONS.contains(&ext) {
            FileKind::SaveData
        } else if AUTHORED_EXTENSIONS.contains(&ext) {
            FileKind::Authored
        } else if generated {
            FileKind::Generated
        } else {
            FileKind::Ordinary
        };

        FileClass {
            size: entry.size,
            modified_unix: entry.modified_unix.unwrap_or(0),
            kind,
        }
    }
}

impl ContentProfile {
    /// Fold an already-classified file in. O(1), and the only thing called in
    /// the per-directory loop.
    pub fn add(&mut self, class: &FileClass) {
        self.files += 1;
        self.bytes += class.size;
        self.newest_unix = self.newest_unix.max(class.modified_unix);
        match class.kind {
            FileKind::SaveData => self.save_data += 1,
            FileKind::Authored => self.authored += 1,
            FileKind::Generated => self.generated += 1,
            FileKind::Ordinary => {}
        }
    }

    /// Classify and fold in one go, for callers with a single directory.
    pub fn observe(&mut self, entry: &FileEntry) {
        if entry.is_dir {
            return;
        }
        self.add(&FileClass::of(entry));
    }
}

/// Walk a directory and describe what is in it.
///
/// Prefer [`crate::scanning::ScanContext::profile_of`], which returns the
/// profile already built during the shared traversal when there is one. This
/// is the fallback for directories the scan did not cover.
pub fn profile(
    dir: &Path,
    protected: &ProtectedPaths,
    cancel: &dyn Fn() -> bool,
) -> ContentProfile {
    let mut out = ContentProfile {
        complete: true,
        ..Default::default()
    };
    let mut seen = 0usize;
    // Both closures need to be able to mark the profile partial.
    let complete = std::cell::Cell::new(true);

    walk::walk(
        dir,
        protected,
        &WalkOptions {
            max_depth: 8,
            report_dirs: false,
        },
        |entry| {
            seen += 1;
            if seen >= MAX_ENTRIES || cancel() {
                complete.set(false);
                return Step::Stop;
            }
            out.observe(&entry);
            Step::Continue
        },
        |_| complete.set(false),
    );

    out.complete = complete.get();
    out
}

/// Directory profiles accumulated during the shared traversal.
///
/// Without this, every detector that wants to know how big a candidate
/// directory is walks that directory itself — and the directories detectors
/// care about are exactly the ones holding most of the files. Measured on a
/// real machine, that second pass cost as much as the entire rest of the scan.
#[derive(Debug, Default)]
pub struct DirectoryIndex {
    profiles: std::collections::HashMap<PathBuf, ContentProfile>,
    saturated: bool,
    /// The depth limit the traversal ran under, so profiles that may have been
    /// truncated by it can say so.
    walk_depth: usize,
}

/// How far below a scan root profiles are kept. Ghost candidates sit one level
/// down; cache rules are rarely deeper than four.
const MAX_INDEX_DEPTH: usize = 4;

/// A ceiling, so one pathological tree cannot eat memory.
const MAX_INDEXED_DIRECTORIES: usize = 120_000;

impl DirectoryIndex {
    pub fn with_walk_depth(walk_depth: usize) -> DirectoryIndex {
        DirectoryIndex {
            walk_depth,
            ..Default::default()
        }
    }

    /// Fold a file into each of its indexed ancestors.
    pub fn observe(&mut self, entry: &FileEntry) {
        if entry.is_dir || entry.depth <= 1 {
            return;
        }
        // Classified once, folded in up to four times.
        let class = FileClass::of(entry);
        let levels = MAX_INDEX_DEPTH.min(entry.depth - 1);
        for level in 1..=levels {
            let Some(ancestor) = entry.path.ancestors().nth(entry.depth - level) else {
                break;
            };
            // A file sitting exactly at the traversal's depth limit means
            // anything below it was never seen, so every ancestor's totals are
            // a floor rather than a fact.
            let truncated = self.walk_depth > 0 && entry.depth >= self.walk_depth;
            if let Some(profile) = self.profiles.get_mut(ancestor) {
                profile.add(&class);
                profile.complete &= !truncated;
                continue;
            }
            if self.profiles.len() >= MAX_INDEXED_DIRECTORIES {
                self.saturated = true;
                return;
            }
            let mut profile = ContentProfile {
                complete: !truncated,
                ..Default::default()
            };
            profile.add(&class);
            self.profiles.insert(ancestor.to_path_buf(), profile);
        }
    }

    pub fn get(&self, path: &Path) -> Option<&ContentProfile> {
        self.profiles.get(path)
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// True when the index hit its ceiling and is therefore incomplete.
    pub fn saturated(&self) -> bool {
        self.saturated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detectors::test_support::Harness;

    fn open_table() -> ProtectedPaths {
        ProtectedPaths::for_home("/nonexistent-home-for-tests")
    }

    fn never() -> impl Fn() -> bool {
        || false
    }

    #[test]
    fn a_shader_cache_reads_as_generated() {
        let h = Harness::new();
        h.materialise(&[
            crate::detectors::test_support::fixture_entry("g/shadercache/a.bin", 10, 100),
            crate::detectors::test_support::fixture_entry("g/shadercache/b.bin", 10, 100),
            crate::detectors::test_support::fixture_entry("g/logs/run.log", 10, 100),
            crate::detectors::test_support::fixture_entry("g/data.cache", 10, 100),
        ]);
        let p = profile(&h.path("g"), &open_table(), &never());
        assert_eq!(p.files, 4);
        assert!(p.looks_generated(), "{p:?}");
        assert!(p.generated_reason().contains('%'));
    }

    #[test]
    fn save_files_are_counted_separately_and_stop_it_looking_generated() {
        let h = Harness::new();
        h.materialise(&[
            crate::detectors::test_support::fixture_entry("g/cache/a.bin", 10, 100),
            crate::detectors::test_support::fixture_entry("g/cache/b.bin", 10, 100),
            crate::detectors::test_support::fixture_entry("g/saves/slot1.sav", 10, 100),
        ]);
        let p = profile(&h.path("g"), &open_table(), &never());
        assert_eq!(p.save_data, 1);
        assert_eq!(p.generated, 2);
    }

    #[test]
    fn authored_documents_stop_a_folder_reading_as_generated() {
        let h = Harness::new();
        h.materialise(&[
            crate::detectors::test_support::fixture_entry("g/a.cache", 10, 100),
            crate::detectors::test_support::fixture_entry("g/b.cache", 10, 100),
            crate::detectors::test_support::fixture_entry("g/c.cache", 10, 100),
            crate::detectors::test_support::fixture_entry("g/notes.docx", 10, 100),
        ]);
        let p = profile(&h.path("g"), &open_table(), &never());
        assert_eq!(p.authored, 1);
        assert!(
            !p.looks_generated(),
            "one real document is enough to stop calling a folder disposable"
        );
    }

    #[test]
    fn an_empty_directory_profiles_as_empty() {
        let h = Harness::new();
        std::fs::create_dir_all(h.path("empty")).unwrap();
        let p = profile(&h.path("empty"), &open_table(), &never());
        assert!(p.is_empty());
        assert!(!p.looks_generated());
    }

    #[test]
    fn cancellation_marks_the_profile_incomplete() {
        let h = Harness::new();
        h.materialise(&[crate::detectors::test_support::fixture_entry(
            "g/a.bin", 10, 10,
        )]);
        let p = profile(&h.path("g"), &open_table(), &|| true);
        assert!(!p.complete);
    }

    #[test]
    fn the_index_accumulates_into_the_topmost_directories() {
        // Ghost candidates are the directories just below a scan root, so a
        // file six levels down must still count towards the one at level one.
        let mut index = DirectoryIndex::with_walk_depth(12);
        index.observe(&entry("/root/AppSupport/DeadApp/a/b/c/blob.cache", 6, 1000));
        index.observe(&entry("/root/AppSupport/DeadApp/a/b/c/other.cache", 6, 500));

        let top = index
            .get(Path::new("/root/AppSupport"))
            .expect("the topmost indexed ancestor");
        assert_eq!(top.bytes, 1500);
        assert_eq!(top.files, 2);
    }

    #[test]
    fn the_index_agrees_with_a_fresh_walk() {
        // If these two ever diverge, findings change depending on whether the
        // profile came from the index or the fallback.
        let h = Harness::new();
        h.materialise(&[
            crate::detectors::test_support::fixture_entry("app/cache/a.cache", 10, 400),
            crate::detectors::test_support::fixture_entry("app/cache/b.cache", 10, 600),
            crate::detectors::test_support::fixture_entry("app/saves/slot.sav", 10, 100),
            crate::detectors::test_support::fixture_entry("app/notes.docx", 10, 200),
        ]);

        let walked = profile(&h.path("app"), &open_table(), &never());

        let mut index = DirectoryIndex::with_walk_depth(12);
        walk::walk(
            &h.home,
            &open_table(),
            &WalkOptions {
                max_depth: 12,
                report_dirs: true,
            },
            |e| {
                index.observe(&e);
                Step::Continue
            },
            |_| {},
        );
        let indexed = index.get(&h.path("app")).expect("app profile");

        assert_eq!(indexed.bytes, walked.bytes);
        assert_eq!(indexed.files, walked.files);
        assert_eq!(indexed.generated, walked.generated);
        assert_eq!(indexed.save_data, walked.save_data);
        assert_eq!(indexed.authored, walked.authored);
    }

    #[test]
    fn a_depth_limited_walk_produces_an_honest_profile() {
        let mut index = DirectoryIndex::with_walk_depth(4);
        // Sitting exactly at the limit: anything deeper was never seen.
        index.observe(&entry("/root/a/b/c/deep.bin", 4, 100));
        assert!(!index.get(Path::new("/root/a")).expect("profile").complete);

        let mut shallow = DirectoryIndex::with_walk_depth(12);
        shallow.observe(&entry("/root/a/b/c/deep.bin", 4, 100));
        assert!(shallow.get(Path::new("/root/a")).expect("profile").complete);
    }

    #[test]
    fn files_directly_in_a_scan_root_index_nothing() {
        // Their only ancestor is the root itself, which is never a candidate.
        let mut index = DirectoryIndex::with_walk_depth(12);
        index.observe(&entry("/root/loose.bin", 1, 100));
        assert!(index.is_empty());
    }

    fn entry(path: &str, depth: usize, size: u64) -> FileEntry {
        let path = PathBuf::from(path);
        FileEntry {
            name: crate::safety::paths::file_name_lower(&path),
            ext: crate::safety::paths::extension(&path).unwrap_or_default(),
            path,
            size,
            modified_unix: Some(1_700_000_000),
            accessed_unix: None,
            created_unix: None,
            is_dir: false,
            depth,
            opaque: false,
        }
    }

    #[test]
    fn totals_add_up() {
        let h = Harness::new();
        h.materialise(&[
            crate::detectors::test_support::fixture_entry("g/a.bin", 10, 1000),
            crate::detectors::test_support::fixture_entry("g/sub/b.bin", 10, 2000),
        ]);
        let p = profile(&h.path("g"), &open_table(), &never());
        assert_eq!(p.bytes, 3000);
        assert_eq!(p.files, 2);
        assert!(p.complete);
    }
}
