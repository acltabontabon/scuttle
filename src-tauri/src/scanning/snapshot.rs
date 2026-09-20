//! The reviewed set of a shared folder.
//!
//! When Scuttle tells someone "33.6 GB of DirectX shader cache", that number is
//! a claim about particular files. Moving "the folder" later would take whatever
//! the folder holds by then, including files that did not exist when the person
//! looked. So at scan time each cache folder's eligible files are written to
//! disk — path relative to the folder, size, modification time, creation time
//! where the filesystem has one, and file identity — and a later move acts on
//! that set and nothing else, re-checking each file immediately before it acts.
//!
//! The set lives in SQLite and is written in batches, so a folder with a few
//! hundred thousand files costs a few hundred thousand rows on disk and never a
//! few hundred thousand entries in memory.

use std::path::Path;

use walkdir::WalkDir;

use crate::quarantine::fsx::{self, EntryKind};
use crate::safety::ProtectedPaths;
use crate::storage::{SnapshotEntry, SnapshotState, Store};
use crate::Result;

/// The most files one finding will carry. A folder with more than this cannot
/// be reviewed as a set, and says so rather than being moved by a rule nobody
/// looked at.
pub const MAX_ENTRIES: u64 = 1_000_000;

/// Deepest level walked. Anything deeper is simply not part of the set.
pub const MAX_DEPTH: usize = 32;

const BATCH: usize = 1000;

/// What decides whether a file belongs to the reviewed set.
pub struct SnapshotPolicy<'a> {
    pub protected: &'a ProtectedPaths,
    /// Files modified within this many seconds of `now_unix` are left out.
    pub settle_secs: i64,
    pub now_unix: i64,
    pub cancelled: &'a dyn Fn() -> bool,
    /// The most files to record. [`MAX_ENTRIES`] outside of tests.
    pub limit: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub state: SnapshotState,
    pub files: u64,
    pub bytes: u64,
}

/// `/`-separated relative path, or `None` for a name that is not valid UTF-8.
/// Such a file cannot round-trip through storage, so it is not offered.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in rel.components() {
        parts.push(component.as_os_str().to_str()?.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Walk `root` and record its eligible files as the reviewed set of
/// `finding_id`, replacing any earlier set.
pub fn record(
    store: &Store,
    finding_id: &str,
    root: &Path,
    policy: &SnapshotPolicy<'_>,
) -> Result<Recorded> {
    store.begin_snapshot(finding_id)?;

    let mut batch: Vec<SnapshotEntry> = Vec::with_capacity(BATCH);
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut truncated = false;

    let walker = WalkDir::new(root)
        .follow_links(false)
        .max_depth(MAX_DEPTH)
        .into_iter()
        // Protected places are pruned, not merely filtered: nothing beneath
        // them is looked at.
        .filter_entry(|entry| !policy.protected.is_protected(entry.path()));

    for entry in walker {
        if (policy.cancelled)() {
            store.begin_snapshot(finding_id)?;
            return Ok(Recorded {
                state: SnapshotState::None,
                files: 0,
                bytes: 0,
            });
        }
        // Unreadable entries are stepped over, as everywhere else.
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let identity = fsx::identity_from(path, &meta);
        if identity.kind != EntryKind::File {
            continue;
        }
        let Some(rel) = relative(root, path) else {
            continue;
        };

        if policy.settle_secs > 0 {
            let Some(mtime_ns) = identity.mtime_ns else {
                continue;
            };
            let modified_secs = mtime_ns.div_euclid(1_000_000_000);
            if modified_secs > policy.now_unix - policy.settle_secs {
                continue;
            }
        }

        if files >= policy.limit {
            truncated = true;
            break;
        }
        files += 1;
        bytes += identity.size;
        batch.push(SnapshotEntry {
            rel,
            size: identity.size,
            mtime_ns: identity.mtime_ns,
            created_ns: identity.created_ns,
            file_id: identity.file_id,
        });
        if batch.len() >= BATCH {
            store.append_snapshot_entries(finding_id, &batch)?;
            batch.clear();
        }
    }
    if !batch.is_empty() {
        store.append_snapshot_entries(finding_id, &batch)?;
    }

    let state = if truncated {
        SnapshotState::Truncated
    } else {
        SnapshotState::Complete
    };
    store.finish_snapshot(
        finding_id,
        state,
        policy.now_unix,
        files,
        bytes,
        // A folder too large to review as a set keeps the size it was measured
        // at; there is no set whose total could replace it.
        !truncated,
    )?;
    Ok(Recorded {
        state,
        files,
        bytes,
    })
}
