//! One budget for discovery, metadata and bounded manifest reads. No links.
use std::collections::BTreeSet;
use std::fs::{self, Metadata};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::quarantine::fsx;
use crate::safety::{paths, ProtectedPaths};

pub const MAX_ROWS: usize = 1_000;
const MAX_ENTRIES: usize = 50_000;
const MAX_CHILDREN: usize = 5_000;
const MAX_MANIFEST_BYTES: u64 = 1_048_576;
const MAX_TOTAL_READ: usize = 8 * 1_048_576;

pub struct Reader<'a> {
    protected: &'a ProtectedPaths,
    cancelled: &'a dyn Fn() -> bool,
    deadline: Instant,
    visited: usize,
    read: usize,
    pub complete: bool,
    pub notes: BTreeSet<String>,
}
pub struct Measurement {
    pub bytes: u64,
    pub newest: Option<i64>,
    pub complete: bool,
    pub fingerprint: String,
}

impl<'a> Reader<'a> {
    pub fn new(protected: &'a ProtectedPaths, cancelled: &'a dyn Fn() -> bool) -> Self {
        Self {
            protected,
            cancelled,
            deadline: Instant::now() + Duration::from_secs(5),
            visited: 0,
            read: 0,
            complete: true,
            notes: BTreeSet::new(),
        }
    }
    pub fn incomplete(&mut self, note: &str) {
        self.complete = false;
        self.notes.insert(note.into());
    }
    pub fn available(&mut self) -> bool {
        if (self.cancelled)() {
            self.incomplete("Inspection stopped; uninspected entries are preserved.");
            return false;
        }
        if self.visited >= MAX_ENTRIES || Instant::now() >= self.deadline {
            self.incomplete(
                "The inspection budget was reached; uninspected entries are preserved.",
            );
            return false;
        }
        true
    }
    fn metadata(&mut self, path: &Path) -> Option<Metadata> {
        if !self.available() {
            return None;
        }
        self.visited += 1;
        if !path.is_absolute() || self.protected.is_protected(path) {
            self.incomplete("Protected locations were left uninspected.");
            return None;
        }
        let base = path.ancestors().last()?;
        if paths::first_link_below(path, base).is_some() {
            self.incomplete("Linked locations were left uninspected.");
            return None;
        }
        match fs::symlink_metadata(path) {
            // symlink_metadata already supplies the entry kind. A second
            // lstat (and a Windows file-ID handle) adds no classification
            // evidence and doubles work on this hot path.
            Ok(meta) if ordinary_entry(&meta) => Some(meta),
            Ok(_) => {
                self.incomplete("Linked or special entries were left uninspected.");
                None
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                self.incomplete("Some entries could not be read.");
                None
            }
        }
    }
    pub fn directory(&mut self, path: &Path) -> bool {
        self.metadata(path).is_some_and(|m| m.is_dir())
    }
    pub fn file(&mut self, path: &Path) -> bool {
        self.metadata(path).is_some_and(|m| m.is_file())
    }
    pub fn children(&mut self, path: &Path) -> Vec<PathBuf> {
        if !self.directory(path) {
            return Vec::new();
        }
        self.read_children(path)
    }
    // The caller has freshly validated this directory. Repeating metadata
    // here wastes the shared budget; read_dir reports a disappearance itself.
    fn read_children(&mut self, path: &Path) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(path) else {
            self.incomplete("Some directories could not be read.");
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries {
            if !self.available() {
                return Vec::new();
            }
            self.visited += 1;
            match entry {
                Ok(entry) => out.push(entry.path()),
                Err(_) => {
                    self.incomplete("Some directory entries could not be read.");
                    return Vec::new();
                }
            }
            if out.len() > MAX_CHILDREN {
                // Do not return an arbitrary prefix of read_dir order.
                self.incomplete("An oversized directory was left uninspected.");
                return Vec::new();
            }
        }
        out.sort();
        out
    }
    pub fn dirs(&mut self, path: &Path) -> Vec<PathBuf> {
        self.children(path)
            .into_iter()
            .filter(|p| self.directory(p))
            .collect()
    }
    pub fn text(&mut self, path: &Path) -> Option<String> {
        let meta = self.metadata(path)?;
        if !meta.is_file() {
            return None;
        }
        if meta.len() > MAX_MANIFEST_BYTES
            || self.read.saturating_add(meta.len() as usize) > MAX_TOTAL_READ
        {
            self.incomplete("A dependency manifest exceeded the read budget.");
            return None;
        }
        let expected = fsx::identity_from(path, &meta);
        let mut bytes = Vec::new();
        let result = open_manifest(path).and_then(|mut file| {
            if fsx::identity_of_file(&file, path)? != expected {
                return Err(std::io::Error::other("manifest identity changed"));
            }
            (&mut file)
                .take(MAX_MANIFEST_BYTES + 1)
                .read_to_end(&mut bytes)?;
            if fsx::identity_of_file(&file, path)? != expected {
                return Err(std::io::Error::other("manifest changed while reading"));
            }
            Ok(())
        });
        self.read = self.read.saturating_add(bytes.len());
        if result.is_err() || bytes.len() as u64 != meta.len() {
            self.incomplete("A manifest could not be read completely or changed while reading.");
            return None;
        }
        let Some(after) = self.metadata(path) else {
            self.incomplete("A dependency manifest disappeared after reading.");
            return None;
        };
        if after.len() != meta.len() || after.modified().ok() != meta.modified().ok() {
            self.incomplete("A dependency manifest changed during inspection.");
            return None;
        }
        match String::from_utf8(bytes) {
            Ok(text) => Some(text),
            Err(_) => {
                self.incomplete("A dependency manifest was not valid UTF-8.");
                None
            }
        }
    }
    pub fn measure(&mut self, root: &Path) -> Measurement {
        let mut out = Measurement {
            bytes: 0,
            newest: None,
            complete: true,
            fingerprint: String::new(),
        };
        let mut hash = blake3::Hasher::new();
        let mut pending = vec![(root.to_path_buf(), 0)];
        while let Some((path, depth)) = pending.pop() {
            let Some(meta) = self.metadata(&path) else {
                out.complete = false;
                self.incomplete("An entry disappeared or could not be measured completely.");
                continue;
            };
            hash.update(
                format!(
                    "{:?}:{:?}",
                    path.strip_prefix(root).unwrap_or(&path),
                    fsx::identity_from(&path, &meta)
                )
                .as_bytes(),
            );
            match meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .and_then(|d| i64::try_from(d.as_secs()).ok())
            {
                Some(t) => out.newest = Some(out.newest.map_or(t, |previous| previous.max(t))),
                None => {
                    out.complete = false;
                    self.incomplete("An entry's modification time could not be established.");
                }
            }
            if meta.is_file() {
                out.bytes = out.bytes.saturating_add(meta.len());
            } else if depth < 12 {
                let was_complete = self.complete;
                let children = self.read_children(&path);
                if was_complete && !self.complete {
                    out.complete = false;
                }
                for child in children.into_iter().rev() {
                    pending.push((child, depth + 1));
                }
            } else {
                out.complete = false;
                self.incomplete("An entry's measurement exceeded the directory depth limit.");
            }
            if !self.available() {
                out.complete = false;
                break;
            }
        }
        // No row claims a full measurement after a global inspection error.
        out.complete &= self.complete;
        out.fingerprint = hash.finalize().to_hex().to_string();
        out
    }
}

fn ordinary_entry(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Junctions and other reparse points must remain excluded even when
        // the metadata reports a regular file/directory underneath them.
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    !meta.file_type().is_symlink() && (meta.is_file() || meta.is_dir())
}

/// Pin every Unix ancestor before opening a manifest. Checking a path for
/// links and then opening it by pathname leaves an avoidable parent-swap gap.
fn open_manifest(path: &Path) -> std::io::Result<fs::File> {
    #[cfg(unix)]
    {
        use fsx::Source;
        use std::path::Component;
        let mut dir = fsx::PinnedDir::open(Path::new("/"))?;
        let parent = path
            .parent()
            .ok_or_else(|| std::io::Error::other("missing parent"))?;
        for component in parent.components() {
            if let Component::Normal(name) = component {
                let name = name
                    .to_str()
                    .ok_or_else(|| std::io::Error::other("non-UTF-8 directory"))?;
                dir = dir.child_dir(name)?;
            }
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| std::io::Error::other("missing filename"))?;
        dir.file(name).open()
    }
    #[cfg(not(unix))]
    {
        fsx::open_no_follow(path)
    }
}
