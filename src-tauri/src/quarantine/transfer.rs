//! Moving one thing from one place to another, safely.
//!
//! This is the mechanism only. Which things may move at all is decided
//! elsewhere ([`crate::safety`]); this module makes sure that once one does,
//! the worst outcome is "it did not move", never "it is gone".
//!
//! The rules it is written to:
//!
//! * **Rename first, and copy only across volumes.** A copy is the answer to
//!   exactly one rename failure: the destination is on another volume. Access
//!   denied, in use, no space and every other refusal would fail a copy just
//!   as surely, and copying before failing is how a refusal used to turn into
//!   lost files.
//! * **Never overwrite.** Everything is published with a rename that fails when
//!   the destination exists.
//! * **The source goes last.** A copy is written under a temporary name,
//!   checked, published, and only then is the original removed — and only if
//!   it is still the object that was copied.
//! * **Say what went wrong.** Every failure carries the operation phase and the
//!   OS error code, and is classified into something a person can act on.
//!
//! What a copy's verification establishes, exactly: the destination holds as
//! many bytes as were read from the source handle; the source handle still
//! refers to the same object with the same size and modification time as when
//! the copy began; and, for files small enough to re-read, the destination's
//! content hash equals the hash of the bytes that were read. It does not prove
//! the source and destination are byte-identical on disk for larger files, and
//! it does not carry extended attributes, alternate data streams or ACLs
//! across.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use super::fsx::{self, EntryKind, Identity, PathSource, Source};

/// Files up to this size are re-read from the destination and their hash
/// compared before the original is removed.
pub const REREAD_VERIFY_LIMIT: u64 = 128 * 1024 * 1024;

const CHUNK: usize = 1024 * 1024;

/// The suffix of a copy that is still being written. Anything carrying it is
/// never presented as held content.
pub const PART_SUFFIX: &str = ".scuttle-part";

/// Where in an operation something went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Check,
    Snapshot,
    Prepare,
    Publish,
    Copy,
    Verify,
    RemoveSource,
    Record,
    Restore,
}

impl Phase {
    pub fn label(&self) -> &'static str {
        match self {
            Phase::Check => "checking",
            Phase::Snapshot => "reading the folder",
            Phase::Prepare => "preparing",
            Phase::Publish => "moving",
            Phase::Copy => "copying",
            Phase::Verify => "verifying",
            Phase::RemoveSource => "removing the original",
            Phase::Record => "recording",
            Phase::Restore => "restoring",
        }
    }
}

/// What kind of failure it was, in terms of what a person could do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Changed since Scuttle looked. Scuttle's own gate, not the OS.
    Stale,
    /// Something else is now where the item was.
    Replaced,
    AccessDenied,
    InUse,
    Missing,
    Collision,
    NoSpace,
    ReadOnly,
    PathTooLong,
    /// A move across drives that could not be done without losing details.
    CrossVolumeRefused,
    /// The filesystem cannot make the guarantees a safe move needs.
    Unsupported,
    /// A link, protected or out-of-scope path. Scuttle's own gate.
    Unsafe,
    Cancelled,
    Other,
}

/// What to offer the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NextStep {
    /// Look at it again: the scan is out of date for this item.
    Refresh,
    /// It may work on another try.
    Retry,
    FreeSpace,
    /// There is nothing to be done that Scuttle should do for you.
    LeaveAlone,
    /// Something is holding the file open. Only ever offered on evidence.
    CloseApp,
}

impl FailureKind {
    /// The stable name used in storage and in diagnostics.
    pub fn slug(&self) -> &'static str {
        match self {
            FailureKind::Stale => "stale",
            FailureKind::Replaced => "replaced",
            FailureKind::AccessDenied => "access_denied",
            FailureKind::InUse => "in_use",
            FailureKind::Missing => "missing",
            FailureKind::Collision => "collision",
            FailureKind::NoSpace => "no_space",
            FailureKind::ReadOnly => "read_only",
            FailureKind::PathTooLong => "path_too_long",
            FailureKind::CrossVolumeRefused => "cross_volume_refused",
            FailureKind::Unsupported => "unsupported",
            FailureKind::Unsafe => "unsafe",
            FailureKind::Cancelled => "cancelled",
            FailureKind::Other => "other",
        }
    }

    /// The slugs of every kind a retry may try again.
    pub const RETRYABLE_SLUGS: [&'static str; 6] = [
        "in_use",
        "access_denied",
        "collision",
        "no_space",
        "cancelled",
        "other",
    ];

    /// Could trying the same item again reasonably succeed without anyone
    /// changing anything the scan looked at?
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            FailureKind::InUse
                | FailureKind::AccessDenied
                | FailureKind::Collision
                | FailureKind::NoSpace
                | FailureKind::Cancelled
                | FailureKind::Other
        )
    }

    pub fn next_step(&self) -> NextStep {
        match self {
            FailureKind::Stale | FailureKind::Replaced | FailureKind::Missing => NextStep::Refresh,
            FailureKind::InUse => NextStep::CloseApp,
            FailureKind::Collision | FailureKind::Cancelled | FailureKind::Other => NextStep::Retry,
            FailureKind::NoSpace => NextStep::FreeSpace,
            FailureKind::AccessDenied
            | FailureKind::ReadOnly
            | FailureKind::PathTooLong
            | FailureKind::CrossVolumeRefused
            | FailureKind::Unsupported
            | FailureKind::Unsafe => NextStep::LeaveAlone,
        }
    }

    /// One sentence, in Scuttle's voice, that says what happened without
    /// claiming more than the failure proves.
    pub fn explain(&self) -> &'static str {
        match self {
            FailureKind::Stale => "It changed since Scuttle looked at it.",
            FailureKind::Replaced => "Something else has taken its place since Scuttle looked.",
            FailureKind::AccessDenied => {
                "The system wouldn't let Scuttle change it. Scuttle uses your normal access \
                 and doesn't try to override that."
            }
            FailureKind::InUse => "Another program is using it.",
            FailureKind::Missing => "It was already gone.",
            FailureKind::Collision => {
                "Something was already at the destination, so nothing was overwritten."
            }
            FailureKind::NoSpace => "There isn't enough room on the drive that holds the Drawer.",
            FailureKind::ReadOnly => "The drive is read-only.",
            FailureKind::PathTooLong => "Its path is too long for the system to move.",
            FailureKind::CrossVolumeRefused => {
                "Moving it to another drive would lose details, so Scuttle left it where it is."
            }
            FailureKind::Unsupported => {
                "This drive can't move things without risking an overwrite, so Scuttle left it."
            }
            FailureKind::Unsafe => {
                "It is a link or a protected place, and Scuttle leaves those alone."
            }
            FailureKind::Cancelled => "Stopped before it was moved.",
            FailureKind::Other => "The system reported an error while moving it.",
        }
    }
}

impl FailureKind {
    /// Did the file change or vanish, rather than the move itself failing?
    /// These are not errors in the operation; they are the world having moved
    /// since the scan, which is exactly what the reviewed set guards against.
    pub fn is_change(&self) -> bool {
        matches!(
            self,
            FailureKind::Stale | FailureKind::Replaced | FailureKind::Missing
        )
    }

    /// Left alone by Scuttle's own rule, not because anything went wrong.
    pub fn is_deliberate_skip(&self) -> bool {
        matches!(self, FailureKind::Unsafe | FailureKind::CrossVolumeRefused)
    }
}

/// A classified failure. `item` is a file name only, never a full path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FsFailure {
    pub kind: FailureKind,
    pub phase: Phase,
    pub os_code: Option<i32>,
    pub item: String,
}

impl FsFailure {
    pub fn new(kind: FailureKind, phase: Phase, item: impl Into<String>) -> FsFailure {
        FsFailure {
            kind,
            phase,
            os_code: None,
            item: item.into(),
        }
    }

    /// Classify an OS error. The raw code is kept exactly as the OS gave it.
    pub fn from_io(err: &io::Error, phase: Phase, item: impl Into<String>) -> FsFailure {
        FsFailure {
            kind: classify(err),
            phase,
            os_code: err.raw_os_error(),
            item: item.into(),
        }
    }

    pub fn next_step(&self) -> NextStep {
        self.kind.next_step()
    }

    pub fn explain(&self) -> &'static str {
        self.kind.explain()
    }
}

impl std::fmt::Display for FsFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.item, self.kind.explain())
    }
}

/// One kind of thing that went wrong, counted, with a few example names.
///
/// A folder with ten thousand locked files is one group with a count of ten
/// thousand, not ten thousand rows. Only file *names* are kept, never paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueGroup {
    pub kind: FailureKind,
    pub phase: Phase,
    pub os_code: Option<i32>,
    pub count: u64,
    pub samples: Vec<String>,
    pub next_step: NextStep,
    pub explanation: &'static str,
}

const SAMPLES_PER_GROUP: usize = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IssueLog {
    groups: Vec<IssueGroup>,
}

impl IssueLog {
    pub fn add(&mut self, failure: &FsFailure) {
        if let Some(group) = self.groups.iter_mut().find(|g| {
            g.kind == failure.kind && g.phase == failure.phase && g.os_code == failure.os_code
        }) {
            group.count += 1;
            if group.samples.len() < SAMPLES_PER_GROUP && !group.samples.contains(&failure.item) {
                group.samples.push(failure.item.clone());
            }
            return;
        }
        self.groups.push(IssueGroup {
            kind: failure.kind,
            phase: failure.phase,
            os_code: failure.os_code,
            count: 1,
            samples: vec![failure.item.clone()],
            next_step: failure.next_step(),
            explanation: failure.explain(),
        });
    }

    pub fn groups(&self) -> &[IssueGroup] {
        &self.groups
    }

    pub fn into_groups(self) -> Vec<IssueGroup> {
        self.groups
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub fn total(&self) -> u64 {
        self.groups.iter().map(|g| g.count).sum()
    }
}

/// Running counts of a batch of moves. Distinct on purpose: an item Scuttle
/// declined to move because it had changed is not the same as one the system
/// refused to move.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    /// Items looked at, whatever became of them.
    pub processed: u64,
    pub moved: u64,
    /// Left where they were because they changed since the scan, or because
    /// Scuttle leaves that kind of thing alone.
    pub skipped: u64,
    /// The system refused, or something ran out.
    pub failed: u64,
    /// Bytes now in the drawer. A rename moves no bytes, but the files are
    /// still in the drawer, so this counts files' sizes either way.
    pub moved_bytes: u64,
    /// Bytes actually copied across a volume boundary.
    pub copied_bytes: u64,
}

impl Tally {
    pub fn record_failure(&mut self, failure: &FsFailure) {
        self.processed += 1;
        if failure.kind.is_change() || failure.kind.is_deliberate_skip() {
            self.skipped += 1;
        } else {
            self.failed += 1;
        }
    }
}

/// Map an OS error onto what a person can act on.
///
/// Keyed on the raw code where there is one, because `io::ErrorKind` folds
/// distinct Windows failures together — a sharing violation and an access
/// denial are different situations with different advice.
pub fn classify(err: &io::Error) -> FailureKind {
    if fsx::is_replaced(err) {
        return FailureKind::Replaced;
    }
    if let Some(code) = err.raw_os_error() {
        if let Some(kind) = classify_code(code) {
            return kind;
        }
    }
    match err.kind() {
        io::ErrorKind::PermissionDenied => FailureKind::AccessDenied,
        io::ErrorKind::NotFound => FailureKind::Missing,
        io::ErrorKind::AlreadyExists => FailureKind::Collision,
        io::ErrorKind::Unsupported => FailureKind::Unsupported,
        _ => FailureKind::Other,
    }
}

#[cfg(windows)]
fn classify_code(code: i32) -> Option<FailureKind> {
    Some(match code {
        5 => FailureKind::AccessDenied,        // ERROR_ACCESS_DENIED
        32 | 33 | 1224 => FailureKind::InUse, // SHARING_VIOLATION, LOCK_VIOLATION, USER_MAPPED_FILE
        2 | 3 => FailureKind::Missing,        // FILE_NOT_FOUND, PATH_NOT_FOUND
        17 => FailureKind::CrossVolumeRefused, // NOT_SAME_DEVICE
        19 => FailureKind::ReadOnly,          // WRITE_PROTECT
        39 | 112 => FailureKind::NoSpace,     // HANDLE_DISK_FULL, DISK_FULL
        80 | 183 => FailureKind::Collision,   // FILE_EXISTS, ALREADY_EXISTS
        206 => FailureKind::PathTooLong,      // FILENAME_EXCED_RANGE
        _ => return None,
    })
}

#[cfg(unix)]
fn classify_code(code: i32) -> Option<FailureKind> {
    Some(match code {
        c if c == libc::EACCES || c == libc::EPERM => FailureKind::AccessDenied,
        c if c == libc::EBUSY || c == libc::ETXTBSY => FailureKind::InUse,
        c if c == libc::ENOENT => FailureKind::Missing,
        c if c == libc::EXDEV => FailureKind::CrossVolumeRefused,
        c if c == libc::ENOSPC || c == libc::EDQUOT => FailureKind::NoSpace,
        c if c == libc::EROFS => FailureKind::ReadOnly,
        c if c == libc::EEXIST || c == libc::ENOTEMPTY => FailureKind::Collision,
        c if c == libc::ENAMETOOLONG => FailureKind::PathTooLong,
        _ => return None,
    })
}

#[cfg(not(any(windows, unix)))]
fn classify_code(_code: i32) -> Option<FailureKind> {
    None
}

/// The steps of a move at which a test can inject a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Rename,
    CopyWrite,
    CopySync,
    Verify,
    PublishPart,
    RemoveSource,
}

/// A plan for failing on purpose. Only tests provide one.
pub trait Faults {
    fn at(&self, step: Step) -> Option<io::Error>;
}

/// How a move is controlled from outside: cancellation, byte progress and, in
/// tests, injected failures.
#[derive(Default)]
pub struct Ctl<'a> {
    pub cancel: Option<&'a AtomicBool>,
    pub faults: Option<&'a dyn Faults>,
    pub on_bytes: Option<&'a dyn Fn(u64)>,
}

impl<'a> Ctl<'a> {
    pub fn none() -> Ctl<'static> {
        Ctl::default()
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
    }

    fn fault(&self, step: Step) -> io::Result<()> {
        match self.faults.and_then(|f| f.at(step)) {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn bytes(&self, n: u64) {
        if let Some(report) = self.on_bytes {
            report(n);
        }
    }
}

/// How a move was accomplished. This is what progress reporting is honest
/// about: a rename moved no bytes, a copy did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Renamed,
    Copied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub method: Method,
    pub bytes: u64,
    /// blake3 of the content, when it was read (copies only).
    pub hash: Option<String>,
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "this".into())
}

fn part_path(to: &Path) -> PathBuf {
    let mut name = to.file_name().unwrap_or_default().to_os_string();
    name.push(PART_SUFFIX);
    to.with_file_name(name)
}

/// Move a file or a whole directory from `from` to `to`.
pub fn move_entry(from: &Path, to: &Path, ctl: &Ctl<'_>) -> Result<Moved, FsFailure> {
    let item = name_of(from);
    let identity =
        fsx::identity_of(from).map_err(|e| FsFailure::from_io(&e, Phase::Check, &item))?;
    match identity.kind {
        EntryKind::File => move_source(&PathSource(from), to, &identity, ctl),
        EntryKind::Dir => move_dir(from, to, identity, ctl),
        EntryKind::Link | EntryKind::Other => {
            Err(FsFailure::new(FailureKind::Unsafe, Phase::Check, item))
        }
    }
}

/// Move one file that has already been described by `expected`.
pub fn move_file_verified(
    from: &Path,
    to: &Path,
    expected: &Identity,
    ctl: &Ctl<'_>,
) -> Result<Moved, FsFailure> {
    move_source(&PathSource(from), to, expected, ctl)
}

fn move_dir(from: &Path, to: &Path, identity: Identity, ctl: &Ctl<'_>) -> Result<Moved, FsFailure> {
    let item = name_of(from);
    let attempt = ctl
        .fault(Step::Rename)
        .and_then(|()| fsx::rename_verified(from, to, &identity));
    match attempt {
        Ok(()) => Ok(Moved {
            method: Method::Renamed,
            bytes: 0,
            hash: None,
        }),
        // A folder that has to cross drives would have to be taken apart and
        // rebuilt, and the rebuild does not carry everything across. Refuse,
        // and leave it exactly where it is.
        Err(err) if fsx::is_cross_device(&err) => Err(FsFailure::new(
            FailureKind::CrossVolumeRefused,
            Phase::Publish,
            item,
        )),
        Err(err) => Err(FsFailure::from_io(&err, Phase::Publish, item)),
    }
}

/// Move one file, found by path or beneath a pinned directory, that has
/// already been described by `expected`.
pub fn move_source(
    source: &dyn Source,
    to: &Path,
    expected: &Identity,
    ctl: &Ctl<'_>,
) -> Result<Moved, FsFailure> {
    let item = source.name();

    let attempt = ctl
        .fault(Step::Rename)
        .and_then(|()| source.rename_verified(to, expected));
    match attempt {
        Ok(()) => {
            return Ok(Moved {
                method: Method::Renamed,
                bytes: 0,
                hash: None,
            })
        }
        Err(err) if fsx::is_cross_device(&err) => {}
        Err(err) => return Err(FsFailure::from_io(&err, Phase::Publish, item)),
    }

    copy_then_remove(source, to, expected, ctl)
}

/// The cross-volume path: copy, check, publish, and only then remove.
fn copy_then_remove(
    src: &dyn Source,
    to: &Path,
    before: &Identity,
    ctl: &Ctl<'_>,
) -> Result<Moved, FsFailure> {
    let item = src.name();
    let label = Path::new(&item);
    let part = part_path(to);
    let fail = |err: &io::Error, phase: Phase| FsFailure::from_io(err, phase, &item);

    let mut source = src.open().map_err(|e| fail(&e, Phase::Copy))?;
    let opened = fsx::identity_of_file(&source, label).map_err(|e| fail(&e, Phase::Check))?;
    if !opened.is_same_state(before) {
        return Err(FsFailure::new(FailureKind::Replaced, Phase::Check, &item));
    }
    let source_meta = source.metadata().map_err(|e| fail(&e, Phase::Check))?;

    // `create_new`: an existing destination — or leftover part — is a
    // collision, never something to write over.
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .map_err(|e| fail(&e, Phase::Copy))?;

    let cleanup_part = |part: &Path| {
        let _ = std::fs::remove_file(part);
    };

    let mut hasher = blake3::Hasher::new();
    let mut written: u64 = 0;
    let mut buffer = vec![0u8; CHUNK];
    let outcome: Result<(), FsFailure> = (|| {
        loop {
            if ctl.cancelled() {
                return Err(FsFailure::new(FailureKind::Cancelled, Phase::Copy, &item));
            }
            let read = source
                .read(&mut buffer)
                .map_err(|e| fail(&e, Phase::Copy))?;
            if read == 0 {
                break;
            }
            ctl.fault(Step::CopyWrite)
                .map_err(|e| fail(&e, Phase::Copy))?;
            out.write_all(&buffer[..read])
                .map_err(|e| fail(&e, Phase::Copy))?;
            hasher.update(&buffer[..read]);
            written += read as u64;
            ctl.bytes(read as u64);
        }
        ctl.fault(Step::CopySync)
            .map_err(|e| fail(&e, Phase::Copy))?;
        out.sync_all().map_err(|e| fail(&e, Phase::Copy))?;
        Ok(())
    })();
    if let Err(failure) = outcome {
        drop(out);
        cleanup_part(&part);
        return Err(failure);
    }

    // Verification. See the module header for what this does and does not
    // establish.
    let verified: Result<String, FsFailure> = (|| {
        ctl.fault(Step::Verify)
            .map_err(|e| fail(&e, Phase::Verify))?;
        if written != before.size {
            return Err(FsFailure::new(FailureKind::Stale, Phase::Verify, &item));
        }
        let after = fsx::identity_of_file(&source, label).map_err(|e| fail(&e, Phase::Verify))?;
        if !after.is_same_state(before) {
            // Changed while it was being copied: what was written may be a
            // mixture of two versions.
            return Err(FsFailure::new(FailureKind::Stale, Phase::Verify, &item));
        }
        let hash = hasher.finalize().to_hex().to_string();
        if before.size <= REREAD_VERIFY_LIMIT {
            let reread = hash_file(&part).map_err(|e| fail(&e, Phase::Verify))?;
            if reread != hash {
                return Err(FsFailure::new(FailureKind::Other, Phase::Verify, &item));
            }
        }
        // Carry the file's own timestamp and permission bits across.
        if let Ok(mtime) = source_meta.modified() {
            let _ = out.set_modified(mtime);
        }
        drop(out);
        let _ = std::fs::set_permissions(&part, source_meta.permissions());
        Ok(hash)
    })();
    let hash = match verified {
        Ok(hash) => hash,
        Err(failure) => {
            cleanup_part(&part);
            return Err(failure);
        }
    };

    if let Err(err) = ctl
        .fault(Step::PublishPart)
        .and_then(|()| fsx::rename_noreplace(&part, to))
    {
        cleanup_part(&part);
        return Err(fail(&err, Phase::Publish));
    }
    let published = fsx::identity_of(to).ok();

    // The original goes last, and only if it is still what was copied.
    if let Err(err) = ctl
        .fault(Step::RemoveSource)
        .and_then(|()| src.remove_verified(before))
    {
        // The original could not be removed, so this move did not happen.
        // Take the copy back out — but only if it is still the object that was
        // published, never a file that has since arrived under that name.
        if let Some(published) = published {
            let _ = fsx::remove_verified(to, &published);
        }
        return Err(fail(&err, Phase::RemoveSource));
    }

    Ok(Moved {
        method: Method::Copied,
        bytes: written,
        hash: Some(hash),
    })
}

/// blake3 of a file's content.
pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::cell::Cell;

    /// Fail with a chosen OS error at one step, optionally after pretending
    /// the volumes differ so the copy path is taken.
    pub struct Script {
        pub cross_device: bool,
        pub fail_at: Option<(Step, i32)>,
        pub hits: Cell<u32>,
        /// Run once when the given step is first reached.
        pub before: Option<(Step, Box<dyn Fn()>)>,
    }

    impl Script {
        pub fn new() -> Script {
            Script {
                cross_device: false,
                fail_at: None,
                hits: Cell::new(0),
                before: None,
            }
        }
        pub fn cross_device(mut self) -> Script {
            self.cross_device = true;
            self
        }
        pub fn failing(mut self, step: Step, raw: i32) -> Script {
            self.fail_at = Some((step, raw));
            self
        }
        pub fn on(mut self, step: Step, action: impl Fn() + 'static) -> Script {
            self.before = Some((step, Box::new(action)));
            self
        }
    }

    pub fn exdev() -> i32 {
        #[cfg(unix)]
        return libc::EXDEV;
        #[cfg(windows)]
        return 17;
    }

    pub fn eacces() -> i32 {
        #[cfg(unix)]
        return libc::EACCES;
        #[cfg(windows)]
        return 5;
    }

    pub fn in_use() -> i32 {
        #[cfg(unix)]
        return libc::EBUSY;
        #[cfg(windows)]
        return 32;
    }

    pub fn disk_full() -> i32 {
        #[cfg(unix)]
        return libc::ENOSPC;
        #[cfg(windows)]
        return 112;
    }

    impl Faults for Script {
        fn at(&self, step: Step) -> Option<io::Error> {
            if let Some((when, action)) = &self.before {
                if *when == step {
                    action();
                }
            }
            if step == Step::Rename && self.cross_device {
                return Some(io::Error::from_raw_os_error(exdev()));
            }
            match self.fail_at {
                Some((at, raw)) if at == step => {
                    self.hits.set(self.hits.get() + 1);
                    Some(io::Error::from_raw_os_error(raw))
                }
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use std::fs;
    use std::path::Path;

    fn part_files(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with(PART_SUFFIX))
            .collect()
    }

    struct Setup {
        _tmp: tempfile::TempDir,
        src: PathBuf,
        dst: PathBuf,
        dir: PathBuf,
    }

    fn setup(contents: &[u8]) -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let src = dir.join("source.bin");
        fs::write(&src, contents).unwrap();
        let dst = dir.join("dest.bin");
        Setup {
            _tmp: tmp,
            src,
            dst,
            dir,
        }
    }

    #[test]
    fn a_same_volume_move_is_a_rename_that_moves_no_bytes() {
        let s = setup(b"payload");
        let moved = move_entry(&s.src, &s.dst, &Ctl::none()).unwrap();
        assert_eq!(moved.method, Method::Renamed);
        assert_eq!(moved.bytes, 0, "a rename copies nothing");
        assert!(!s.src.exists());
        assert_eq!(fs::read(&s.dst).unwrap(), b"payload");
    }

    #[test]
    fn a_cross_volume_move_copies_verifies_and_then_removes_the_original() {
        let s = setup(b"payload");
        let mtime = fs::metadata(&s.src).unwrap().modified().unwrap();
        let script = Script::new().cross_device();
        let ctl = Ctl {
            faults: Some(&script),
            ..Ctl::default()
        };

        let moved = move_entry(&s.src, &s.dst, &ctl).unwrap();

        assert_eq!(moved.method, Method::Copied);
        assert_eq!(moved.bytes, 7);
        assert!(moved.hash.is_some());
        assert!(
            !s.src.exists(),
            "the original is removed once the copy is in place"
        );
        assert_eq!(fs::read(&s.dst).unwrap(), b"payload");
        assert_eq!(
            fs::metadata(&s.dst).unwrap().modified().unwrap(),
            mtime,
            "the file keeps its own timestamp"
        );
        assert!(part_files(&s.dir).is_empty());
    }

    #[test]
    fn a_destination_that_appears_is_never_overwritten() {
        // Both routes: the rename and the copy publish without replacing.
        for cross in [false, true] {
            let s = setup(b"the original");
            fs::write(&s.dst, b"someone else's").unwrap();
            let script = if cross {
                Script::new().cross_device()
            } else {
                Script::new()
            };
            let ctl = Ctl {
                faults: Some(&script),
                ..Ctl::default()
            };

            let failure = move_entry(&s.src, &s.dst, &ctl).unwrap_err();

            assert_eq!(
                failure.kind,
                FailureKind::Collision,
                "cross={cross}: {failure:?}"
            );
            assert_eq!(
                fs::read(&s.dst).unwrap(),
                b"someone else's",
                "cross={cross}"
            );
            assert_eq!(fs::read(&s.src).unwrap(), b"the original", "cross={cross}");
            assert!(part_files(&s.dir).is_empty(), "cross={cross}");
        }
    }

    #[test]
    fn a_source_that_changes_during_the_copy_is_kept_and_nothing_is_published() {
        let s = setup(b"before");
        let path = s.src.clone();
        let script = Script::new().cross_device().on(Step::Verify, move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            fs::write(&path, b"a different and longer body").unwrap();
        });
        let ctl = Ctl {
            faults: Some(&script),
            ..Ctl::default()
        };

        let failure = move_entry(&s.src, &s.dst, &ctl).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Stale, "{failure:?}");
        assert_eq!(failure.phase, Phase::Verify);
        assert!(s.src.exists(), "the source must survive");
        assert!(!s.dst.exists(), "nothing may be published from a torn copy");
        assert!(part_files(&s.dir).is_empty());
    }

    #[test]
    fn a_source_replaced_before_the_copy_starts_is_not_copied() {
        let s = setup(b"original");
        let identity = fsx::identity_of(&s.src).unwrap();
        let mtime = fs::metadata(&s.src).unwrap().modified().unwrap();
        let staging = s.dir.join("staging.bin");
        fs::write(&staging, b"imposter").unwrap();
        fs::File::options()
            .write(true)
            .open(&staging)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        fs::rename(&staging, &s.src).unwrap();

        let script = Script::new().cross_device();
        let ctl = Ctl {
            faults: Some(&script),
            ..Ctl::default()
        };
        let failure = move_file_verified(&s.src, &s.dst, &identity, &ctl).unwrap_err();
        assert!(
            matches!(failure.kind, FailureKind::Replaced | FailureKind::Stale),
            "{failure:?}"
        );
        assert!(!s.dst.exists());
        assert_eq!(fs::read(&s.src).unwrap(), b"imposter");
    }

    /// The invariant the old `copy_tree` + `remove_dir_all` fallback broke:
    /// however a move fails, every original file's content is still available
    /// in the source, in the destination, or both.
    #[test]
    fn no_failure_at_any_step_can_lose_a_files_content() {
        let steps: &[(&str, Option<Step>, bool, i32)] = &[
            (
                "rename refused: access denied",
                Some(Step::Rename),
                false,
                eacces(),
            ),
            (
                "rename refused: in use",
                Some(Step::Rename),
                false,
                in_use(),
            ),
            (
                "copy write fails: disk full",
                Some(Step::CopyWrite),
                true,
                disk_full(),
            ),
            ("sync fails", Some(Step::CopySync), true, in_use()),
            ("verify fails", Some(Step::Verify), true, in_use()),
            ("publish fails", Some(Step::PublishPart), true, eacces()),
            (
                "source removal fails: locked",
                Some(Step::RemoveSource),
                true,
                in_use(),
            ),
        ];
        let body: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();

        for (label, step, cross, raw) in steps {
            let s = setup(&body);
            let mut script = Script::new().failing(step.unwrap(), *raw);
            if *cross {
                script = script.cross_device();
            }
            let ctl = Ctl {
                faults: Some(&script),
                ..Ctl::default()
            };

            let outcome = move_entry(&s.src, &s.dst, &ctl);
            assert!(
                outcome.is_err(),
                "{label}: the injected failure must surface"
            );

            let in_source = fs::read(&s.src).ok().as_deref() == Some(&body[..]);
            let in_dest = fs::read(&s.dst).ok().as_deref() == Some(&body[..]);
            assert!(
                in_source || in_dest,
                "{label}: the content is nowhere — that is data loss"
            );
            assert!(
                part_files(&s.dir).is_empty(),
                "{label}: a partial copy was left behind"
            );
            // A failed move must not also leave a duplicate that looks held.
            assert!(
                !(in_source && in_dest),
                "{label}: failure left both a source and a published copy"
            );
        }
    }

    #[test]
    fn a_directory_is_never_taken_apart_to_cross_a_volume() {
        // The old fallback copied the tree and then deleted the source tree
        // file by file. A locked file stopped that halfway, and the cleanup
        // then removed the copy too. A directory that would have to cross
        // drives is now refused before anything is touched.
        let tmp = tempfile::tempdir().unwrap();
        let tree = tmp.path().join("cache");
        fs::create_dir_all(tree.join("sub")).unwrap();
        for n in 0..5 {
            fs::write(tree.join(format!("f{n}.bin")), format!("body {n}")).unwrap();
        }
        fs::write(tree.join("sub/deep.bin"), "deep").unwrap();
        let dest = tmp.path().join("moved");

        let script = Script::new().cross_device();
        let ctl = Ctl {
            faults: Some(&script),
            ..Ctl::default()
        };
        let failure = move_entry(&tree, &dest, &ctl).unwrap_err();

        assert_eq!(failure.kind, FailureKind::CrossVolumeRefused);
        assert!(!dest.exists(), "nothing may be written for a refused move");
        for n in 0..5 {
            assert_eq!(
                fs::read_to_string(tree.join(format!("f{n}.bin"))).unwrap(),
                format!("body {n}")
            );
        }
        assert_eq!(
            fs::read_to_string(tree.join("sub/deep.bin")).unwrap(),
            "deep"
        );
    }

    #[test]
    fn a_directory_moves_whole_by_rename_and_never_over_an_existing_one() {
        let tmp = tempfile::tempdir().unwrap();
        let tree = tmp.path().join("cache");
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join("a.bin"), "a").unwrap();
        let dest = tmp.path().join("moved");
        move_entry(&tree, &dest, &Ctl::none()).unwrap();
        assert_eq!(fs::read_to_string(dest.join("a.bin")).unwrap(), "a");

        let other = tmp.path().join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("keep.bin"), "keep").unwrap();
        let failure = move_entry(&dest, &other, &Ctl::none()).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Collision, "{failure:?}");
        assert_eq!(fs::read_to_string(other.join("keep.bin")).unwrap(), "keep");
        assert!(dest.join("a.bin").exists());
    }

    #[test]
    fn cancelling_mid_copy_removes_the_partial_and_keeps_the_source() {
        let s = setup(&vec![7u8; 3 * CHUNK]);
        let cancel = AtomicBool::new(false);
        let report = |_: u64| cancel.store(true, Ordering::Relaxed);
        let script = Script::new().cross_device();
        let ctl = Ctl {
            cancel: Some(&cancel),
            faults: Some(&script),
            on_bytes: Some(&report),
        };

        let failure = move_entry(&s.src, &s.dst, &ctl).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Cancelled);
        assert!(s.src.exists());
        assert!(!s.dst.exists());
        assert!(part_files(&s.dir).is_empty());
    }

    #[test]
    fn a_link_is_refused_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real.txt");
        fs::write(&real, "x").unwrap();
        let link = tmp.path().join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_file(&real, &link).is_err() {
            return;
        }
        let failure = move_entry(&link, &tmp.path().join("dest"), &Ctl::none()).unwrap_err();
        assert_eq!(failure.kind, FailureKind::Unsafe);
        assert!(real.exists() && link.symlink_metadata().is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn an_unwritable_parent_is_reported_as_access_denied_with_no_copy_attempted() {
        // The same shape the old fallback mishandled: the source can be read
        // but neither renamed nor removed. It must now stop at the refusal.
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let locked = tmp.path().join("locked");
        fs::create_dir_all(&locked).unwrap();
        let source = locked.join("held.bin");
        fs::write(&source, "the only copy").unwrap();
        let destination = tmp.path().join("moved.bin");

        let original = fs::metadata(&locked).unwrap().permissions();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
        let outcome = move_entry(&source, &destination, &Ctl::none());
        fs::set_permissions(&locked, original).unwrap();

        let failure = outcome.unwrap_err();
        assert_eq!(failure.kind, FailureKind::AccessDenied, "{failure:?}");
        assert_eq!(failure.phase, Phase::Publish);
        assert!(failure.os_code.is_some(), "the OS code is kept");
        assert!(!destination.exists(), "no copy may be made after a refusal");
        assert_eq!(fs::read_to_string(&source).unwrap(), "the only copy");
    }

    #[test]
    fn the_classification_keeps_the_code_and_only_advises_closing_an_app_for_in_use() {
        let table: Vec<(i32, FailureKind)> = {
            #[cfg(windows)]
            {
                vec![
                    (5, FailureKind::AccessDenied),
                    (32, FailureKind::InUse),
                    (33, FailureKind::InUse),
                    (1224, FailureKind::InUse),
                    (2, FailureKind::Missing),
                    (3, FailureKind::Missing),
                    (17, FailureKind::CrossVolumeRefused),
                    (19, FailureKind::ReadOnly),
                    (39, FailureKind::NoSpace),
                    (112, FailureKind::NoSpace),
                    (80, FailureKind::Collision),
                    (183, FailureKind::Collision),
                    (206, FailureKind::PathTooLong),
                ]
            }
            #[cfg(unix)]
            {
                vec![
                    (libc::EACCES, FailureKind::AccessDenied),
                    (libc::EPERM, FailureKind::AccessDenied),
                    (libc::EBUSY, FailureKind::InUse),
                    (libc::ETXTBSY, FailureKind::InUse),
                    (libc::ENOENT, FailureKind::Missing),
                    (libc::EXDEV, FailureKind::CrossVolumeRefused),
                    (libc::ENOSPC, FailureKind::NoSpace),
                    (libc::EROFS, FailureKind::ReadOnly),
                    (libc::EEXIST, FailureKind::Collision),
                    (libc::ENAMETOOLONG, FailureKind::PathTooLong),
                ]
            }
        };
        for (code, expected) in table {
            let failure = FsFailure::from_io(
                &io::Error::from_raw_os_error(code),
                Phase::Publish,
                "thing.bin",
            );
            assert_eq!(failure.kind, expected, "code {code}");
            assert_eq!(failure.os_code, Some(code), "the raw code must be kept");
            assert_eq!(failure.phase, Phase::Publish);
            let advises_closing = failure.next_step() == NextStep::CloseApp;
            assert_eq!(
                advises_closing,
                expected == FailureKind::InUse,
                "only an in-use failure may advise closing an app (code {code})"
            );
        }
    }

    #[test]
    fn a_plain_access_denial_never_advises_closing_anything() {
        assert_eq!(FailureKind::AccessDenied.next_step(), NextStep::LeaveAlone);
        assert!(!FailureKind::AccessDenied
            .explain()
            .to_lowercase()
            .contains("close"));
    }
}
