//! Platform filesystem primitives the drawer's moves are built on.
//!
//! Two properties the standard library does not give us, and that moving
//! somebody's files needs:
//!
//! * **Publishing never replaces.** `std::fs::rename` overwrites an existing
//!   destination on Windows (`MOVEFILE_REPLACE_EXISTING`) and on Unix. Every
//!   move here goes through [`rename_noreplace`], which fails with
//!   `AlreadyExists` instead, so "never overwrite" is enforced by the kernel
//!   rather than by a check that can lose a race.
//! * **Acting on the thing that was checked.** [`rename_verified`] compares
//!   what is at the path with what Scuttle recorded before it moves anything.
//!   On Windows the check and the rename go through one open handle, so they
//!   are bound to the same object. On Unix there is no rename-by-descriptor:
//!   the object is checked before and again after, and a swap in between is
//!   reversed. That narrows the window; it does not close it, and nothing in
//!   Scuttle claims otherwise.

use std::io;
use std::path::Path;
use std::time::SystemTime;

/// What kind of thing is at a path, without following links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    /// A symlink, junction or other reparse point.
    Link,
    /// Sockets, devices, pipes.
    Other,
}

/// What Scuttle can tell about an object cheaply enough to compare later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// `dev:ino` on Unix, volume serial and file index on Windows. Absent when
    /// the platform would not say.
    pub file_id: Option<String>,
    pub size: u64,
    pub mtime_ns: Option<i64>,
    /// Birth time. Plenty of filesystems do not have one, so it is compared
    /// only when both sides do.
    pub created_ns: Option<i64>,
    pub kind: EntryKind,
}

impl Identity {
    /// Is `other` the same object in the same state?
    ///
    /// The file id is the strongest signal and decides on its own when both
    /// sides have one. Size and modification time are always compared, so a
    /// file rewritten in place is caught even though it kept its id. Creation
    /// time is compared only when both sides have it.
    pub fn is_same_state(&self, other: &Identity) -> bool {
        if self.kind != other.kind {
            return false;
        }
        if let (Some(a), Some(b)) = (&self.file_id, &other.file_id) {
            if a != b {
                return false;
            }
        }
        if self.size != other.size || self.mtime_ns != other.mtime_ns {
            return false;
        }
        match (self.created_ns, other.created_ns) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
    }

    /// Same object, ignoring whether its contents have since changed. Used
    /// after a rename, where the moved object should be exactly the one that
    /// was checked.
    pub fn is_same_object(&self, other: &Identity) -> bool {
        match (&self.file_id, &other.file_id) {
            (Some(a), Some(b)) => a == b && self.kind == other.kind,
            _ => self.is_same_state(other),
        }
    }
}

pub(crate) fn nanos(time: io::Result<SystemTime>) -> Option<i64> {
    time.ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
}

fn kind_of(meta: &std::fs::Metadata) -> EntryKind {
    let file_type = meta.file_type();
    if file_type.is_symlink() {
        return EntryKind::Link;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return EntryKind::Link;
        }
    }
    if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

/// Describe whatever is at `path`, never following a link.
pub fn identity_of(path: &Path) -> io::Result<Identity> {
    let meta = std::fs::symlink_metadata(path)?;
    Ok(identity_from(path, &meta))
}

/// Build an identity from metadata already in hand (a directory walk has it).
pub fn identity_from(path: &Path, meta: &std::fs::Metadata) -> Identity {
    Identity {
        file_id: file_id(path, meta),
        size: meta.len(),
        mtime_ns: nanos(meta.modified()),
        created_ns: nanos(meta.created()),
        kind: kind_of(meta),
    }
}

#[cfg(unix)]
fn file_id(_path: &Path, meta: &std::fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    Some(format!("{}:{}", meta.dev(), meta.ino()))
}

#[cfg(windows)]
fn file_id(path: &Path, _meta: &std::fs::Metadata) -> Option<String> {
    win::file_id(path)
}

#[cfg(not(any(unix, windows)))]
fn file_id(_path: &Path, _meta: &std::fs::Metadata) -> Option<String> {
    None
}

/// Open a file for reading without following a link at the final component.
pub fn open_no_follow(path: &Path) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Share everything so reading never blocks another program, and open
        // a reparse point itself rather than what it points at.
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2 | 0x4)
            .custom_flags(0x0020_0000)
            .open(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::fs::File::open(path)
    }
}

/// Describe an already-open file. The answer is about the object the handle
/// refers to, whatever has happened to its path since.
pub fn identity_of_file(file: &std::fs::File, path: &Path) -> io::Result<Identity> {
    let meta = file.metadata()?;
    #[allow(unused_mut)]
    let mut identity = identity_from(path, &meta);
    #[cfg(windows)]
    {
        identity.file_id = win::file_id_of_handle(file);
    }
    Ok(identity)
}

/// Did this error mean "source and destination are on different volumes"?
///
/// The one reason a rename fails that a copy can fix. Everything else — access
/// denied, in use, no space — would fail the copy the same way, and falling
/// back for those is how the old code turned a refusal into a data-loss bug.
pub fn is_cross_device(err: &io::Error) -> bool {
    #[cfg(unix)]
    {
        err.raw_os_error() == Some(libc::EXDEV)
    }
    #[cfg(windows)]
    {
        // ERROR_NOT_SAME_DEVICE
        err.raw_os_error() == Some(17)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = err;
        false
    }
}

/// Move `from` to `to` without ever replacing something already at `to`.
///
/// Fails with `AlreadyExists` when the destination is occupied, with
/// [`is_cross_device`] errors when it is on another volume, and with
/// `Unsupported` when the filesystem cannot make the no-replace guarantee for
/// this kind of object. It does not copy.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    imp::rename_noreplace(from, to)
}

/// Move a file or directory that Scuttle has already described, and confirm
/// that what moved is what was described.
///
/// The moved object is the one recorded in `expected`, or the move is undone
/// and reported as [`io::ErrorKind::InvalidData`] with a `replaced` message.
pub fn rename_verified(from: &Path, to: &Path, expected: &Identity) -> io::Result<()> {
    imp::rename_verified(from, to, expected)
}

/// Remove a regular file, but only if it is still the object described.
pub fn remove_verified(path: &Path, expected: &Identity) -> io::Result<()> {
    imp::remove_verified(path, expected)
}

/// Marker error text used when the object at a path was not what was recorded.
pub const REPLACED: &str = "the file was replaced after it was checked";

pub fn replaced_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, REPLACED)
}

pub fn is_replaced(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::InvalidData && err.to_string() == REPLACED
}

/// Something Scuttle is about to act on: a path, or an entry of a directory it
/// has pinned open.
///
/// The mover is written against this, so the same code moves a file found by
/// path and a file found beneath a pinned directory. Pinning matters on Unix:
/// once the cache root is open as a descriptor, every child is reached through
/// it, and swapping a component of the path *above* the root for a link can no
/// longer redirect anything.
pub trait Source {
    /// The name shown to a person: a file name, never a path.
    fn name(&self) -> String;
    fn identity(&self) -> io::Result<Identity>;
    fn open(&self) -> io::Result<std::fs::File>;
    fn rename_verified(&self, to: &Path, expected: &Identity) -> io::Result<()>;
    fn remove_verified(&self, expected: &Identity) -> io::Result<()>;
}

/// A source found by path.
pub struct PathSource<'a>(pub &'a Path);

impl Source for PathSource<'_> {
    fn name(&self) -> String {
        self.0
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "this".into())
    }
    fn identity(&self) -> io::Result<Identity> {
        identity_of(self.0)
    }
    fn open(&self) -> io::Result<std::fs::File> {
        open_no_follow(self.0)
    }
    fn rename_verified(&self, to: &Path, expected: &Identity) -> io::Result<()> {
        rename_verified(self.0, to, expected)
    }
    fn remove_verified(&self, expected: &Identity) -> io::Result<()> {
        remove_verified(self.0, expected)
    }
}

#[cfg(unix)]
pub use pinned::{PinnedDir, PinnedFile};

/// Windows has no descriptor-relative operations to pin with; instead every
/// rename and delete goes through a handle that was checked (see the module
/// header), so a pinned directory is only a path there.
#[cfg(not(unix))]
pub struct PinnedDir {
    path: std::path::PathBuf,
}

#[cfg(not(unix))]
impl PinnedDir {
    pub fn open(path: &Path) -> io::Result<PinnedDir> {
        let meta = std::fs::symlink_metadata(path)?;
        if kind_of(&meta) != EntryKind::Dir {
            return Err(replaced_error());
        }
        Ok(PinnedDir {
            path: path.to_path_buf(),
        })
    }
    pub fn child_dir(&self, name: &str) -> io::Result<PinnedDir> {
        PinnedDir::open(&self.path.join(name))
    }
    pub fn try_clone(&self) -> io::Result<PinnedDir> {
        Ok(PinnedDir {
            path: self.path.clone(),
        })
    }
    pub fn file<'a>(&'a self, name: &str) -> PinnedFile<'a> {
        PinnedFile {
            _dir: self,
            path: self.path.join(name),
        }
    }
}

#[cfg(not(unix))]
pub struct PinnedFile<'a> {
    _dir: &'a PinnedDir,
    path: std::path::PathBuf,
}

#[cfg(not(unix))]
impl Source for PinnedFile<'_> {
    fn name(&self) -> String {
        PathSource(&self.path).name()
    }
    fn identity(&self) -> io::Result<Identity> {
        identity_of(&self.path)
    }
    fn open(&self) -> io::Result<std::fs::File> {
        open_no_follow(&self.path)
    }
    fn rename_verified(&self, to: &Path, expected: &Identity) -> io::Result<()> {
        rename_verified(&self.path, to, expected)
    }
    fn remove_verified(&self, expected: &Identity) -> io::Result<()> {
        remove_verified(&self.path, expected)
    }
}

#[cfg(unix)]
mod pinned {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    /// A directory held open. Children are reached through the descriptor with
    /// `O_NOFOLLOW`, so they are exactly the children of *this* directory
    /// object, however the path that led here changes afterwards.
    pub struct PinnedDir {
        fd: OwnedFd,
    }

    fn c_name(name: &str) -> io::Result<CString> {
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a single path component",
            ));
        }
        CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "name contains a NUL byte"))
    }

    fn open_dir_at(dirfd: i32, name: &CString) -> io::Result<OwnedFd> {
        // SAFETY: `name` is NUL-terminated; a returned descriptor is owned here.
        let fd = unsafe {
            libc::openat(
                dirfd,
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    }

    impl PinnedDir {
        pub fn open(path: &Path) -> io::Result<PinnedDir> {
            let c = CString::new(path.as_os_str().as_bytes())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;
            Ok(PinnedDir {
                fd: open_dir_at(libc::AT_FDCWD, &c)?,
            })
        }

        pub fn child_dir(&self, name: &str) -> io::Result<PinnedDir> {
            Ok(PinnedDir {
                fd: open_dir_at(self.fd.as_raw_fd(), &c_name(name)?)?,
            })
        }

        pub fn try_clone(&self) -> io::Result<PinnedDir> {
            Ok(PinnedDir {
                fd: self.fd.try_clone()?,
            })
        }

        pub fn file<'a>(&'a self, name: &str) -> PinnedFile<'a> {
            PinnedFile {
                dir: self,
                name: name.to_string(),
            }
        }
    }

    pub struct PinnedFile<'a> {
        dir: &'a PinnedDir,
        name: String,
    }

    // `st_mtime` and friends are `i64` on 64-bit targets and narrower elsewhere;
    // the conversion is needed on the latter.
    #[allow(clippy::useless_conversion)]
    fn identity_from_stat(st: &libc::stat) -> Identity {
        let mode = st.st_mode & libc::S_IFMT;
        let kind = if mode == libc::S_IFREG {
            EntryKind::File
        } else if mode == libc::S_IFDIR {
            EntryKind::Dir
        } else if mode == libc::S_IFLNK {
            EntryKind::Link
        } else {
            EntryKind::Other
        };
        let mtime_ns = i64::from(st.st_mtime)
            .checked_mul(1_000_000_000)
            .and_then(|s| s.checked_add(i64::from(st.st_mtime_nsec)));
        #[cfg(target_os = "macos")]
        let created_ns = i64::from(st.st_birthtime)
            .checked_mul(1_000_000_000)
            .and_then(|s| s.checked_add(i64::from(st.st_birthtime_nsec)));
        #[cfg(not(target_os = "macos"))]
        let created_ns = None;
        Identity {
            file_id: Some(format!("{}:{}", st.st_dev, st.st_ino)),
            size: u64::try_from(st.st_size).unwrap_or(0),
            mtime_ns,
            created_ns,
            kind,
        }
    }

    impl PinnedFile<'_> {
        fn cname(&self) -> io::Result<CString> {
            c_name(&self.name)
        }
        fn dirfd(&self) -> i32 {
            self.dir.fd.as_raw_fd()
        }

        #[cfg(target_os = "macos")]
        fn raw_rename(&self, to: &CString) -> io::Result<()> {
            let from = self.cname()?;
            // SAFETY: valid NUL-terminated strings and an open descriptor.
            let rc = unsafe {
                libc::renameatx_np(
                    self.dirfd(),
                    from.as_ptr(),
                    libc::AT_FDCWD,
                    to.as_ptr(),
                    libc::RENAME_EXCL,
                )
            };
            if rc == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        #[cfg(target_os = "linux")]
        fn raw_rename(&self, to: &CString) -> io::Result<()> {
            let from = self.cname()?;
            // SAFETY: valid NUL-terminated strings and an open descriptor.
            let rc = unsafe {
                libc::syscall(
                    libc::SYS_renameat2,
                    self.dirfd(),
                    from.as_ptr(),
                    libc::AT_FDCWD,
                    to.as_ptr(),
                    1u32, // RENAME_NOREPLACE
                )
            };
            if rc == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        fn raw_rename(&self, _to: &CString) -> io::Result<()> {
            Err(io::Error::from_raw_os_error(libc::ENOSYS))
        }

        fn rename_noreplace(&self, to: &Path) -> io::Result<()> {
            let c_to = CString::new(to.as_os_str().as_bytes())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;
            let err = match self.raw_rename(&c_to) {
                Ok(()) => return Ok(()),
                Err(err) => err,
            };
            let unsupported = matches!(
                err.raw_os_error(),
                Some(libc::ENOSYS) | Some(libc::EINVAL) | Some(libc::ENOTSUP)
            );
            if !unsupported {
                return Err(err);
            }
            // As in `rename_noreplace`: link cannot replace, so for a plain
            // file it is the same guarantee in two steps.
            let from = self.cname()?;
            // SAFETY: valid strings and descriptor for both calls.
            unsafe {
                if libc::linkat(
                    self.dirfd(),
                    from.as_ptr(),
                    libc::AT_FDCWD,
                    c_to.as_ptr(),
                    0,
                ) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                if libc::unlinkat(self.dirfd(), from.as_ptr(), 0) != 0 {
                    let unlink_err = io::Error::last_os_error();
                    let _ = std::fs::remove_file(to);
                    return Err(unlink_err);
                }
            }
            Ok(())
        }
    }

    impl Source for PinnedFile<'_> {
        fn name(&self) -> String {
            self.name.clone()
        }

        fn identity(&self) -> io::Result<Identity> {
            let name = self.cname()?;
            // SAFETY: zeroed stat is valid; the call fills it.
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            let rc = unsafe {
                libc::fstatat(
                    self.dirfd(),
                    name.as_ptr(),
                    &mut st,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(identity_from_stat(&st))
        }

        fn open(&self) -> io::Result<std::fs::File> {
            let name = self.cname()?;
            // SAFETY: valid string and descriptor; the fd is owned by the File.
            let fd = unsafe {
                libc::openat(
                    self.dirfd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(unsafe { std::fs::File::from_raw_fd(fd) })
            }
        }

        fn rename_verified(&self, to: &Path, expected: &Identity) -> io::Result<()> {
            let before = self.identity()?;
            if !before.is_same_state(expected) {
                return Err(replaced_error());
            }
            self.rename_noreplace(to)?;
            // No rename-by-descriptor on Unix: confirm afterwards that what
            // arrived is what was checked, and put it back if it is not. The
            // window between the check and the rename is narrowed, not closed.
            match identity_of(to) {
                Ok(after) if after.is_same_object(expected) => Ok(()),
                _ => {
                    let _ = imp::rename_noreplace_to_dir(to, self.dirfd(), &self.name);
                    Err(replaced_error())
                }
            }
        }

        fn remove_verified(&self, expected: &Identity) -> io::Result<()> {
            let now = self.identity()?;
            if now.kind != EntryKind::File || !now.is_same_object(expected) {
                return Err(replaced_error());
            }
            let name = self.cname()?;
            // SAFETY: valid string and descriptor.
            let rc = unsafe { libc::unlinkat(self.dirfd(), name.as_ptr(), 0) };
            if rc != 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    fn c_path(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))
    }

    #[cfg(target_os = "macos")]
    fn raw_noreplace(from: &CString, to: &CString) -> io::Result<()> {
        // SAFETY: both pointers are valid NUL-terminated strings for the call.
        let rc = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(target_os = "linux")]
    fn raw_noreplace(from: &CString, to: &CString) -> io::Result<()> {
        // SAFETY: both pointers are valid NUL-terminated strings for the call.
        let rc = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                1u32, // RENAME_NOREPLACE
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn raw_noreplace(_from: &CString, _to: &CString) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }

    pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
        let c_from = c_path(from)?;
        let c_to = c_path(to)?;

        let err = match raw_noreplace(&c_from, &c_to) {
            Ok(()) => return Ok(()),
            Err(err) => err,
        };
        let unsupported = matches!(
            err.raw_os_error(),
            Some(libc::ENOSYS) | Some(libc::EINVAL) | Some(libc::ENOTSUP)
        );
        if !unsupported {
            return Err(err);
        }

        // The filesystem cannot rename without replacing. `link` cannot
        // replace either, so for a plain file it gives the same guarantee in
        // two steps: publish the new name, then drop the old one.
        let meta = std::fs::symlink_metadata(from)?;
        if !meta.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "this filesystem cannot move a folder without risking an overwrite",
            ));
        }
        std::fs::hard_link(from, to)?;
        if let Err(err) = std::fs::remove_file(from) {
            // Leave exactly one name behind: the original.
            let _ = std::fs::remove_file(to);
            return Err(err);
        }
        Ok(())
    }

    /// Put something back under a name inside an open directory, without
    /// replacing anything there. Used only to undo a move that turned out to
    /// have moved the wrong object.
    pub fn rename_noreplace_to_dir(from: &Path, dirfd: i32, name: &str) -> io::Result<()> {
        let c_from = c_path(from)?;
        let c_name = CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in name"))?;
        #[cfg(target_os = "macos")]
        let rc = unsafe {
            libc::renameatx_np(
                libc::AT_FDCWD,
                c_from.as_ptr(),
                dirfd,
                c_name.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(target_os = "linux")]
        let rc = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                c_from.as_ptr(),
                dirfd,
                c_name.as_ptr(),
                1u32,
            ) as i32
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let rc = {
            let _ = (&c_from, &c_name, dirfd);
            -1
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub fn rename_verified(from: &Path, to: &Path, expected: &Identity) -> io::Result<()> {
        let before = identity_of(from)?;
        if !before.is_same_state(expected) {
            return Err(replaced_error());
        }
        rename_noreplace(from, to)?;

        // Unix cannot rename by descriptor, so confirm afterwards that what
        // arrived is what was checked, and put it back if it is not.
        match identity_of(to) {
            Ok(after) if after.is_same_object(expected) => Ok(()),
            _ => {
                let _ = rename_noreplace(to, from);
                Err(replaced_error())
            }
        }
    }

    pub fn remove_verified(path: &Path, expected: &Identity) -> io::Result<()> {
        let now = identity_of(path)?;
        if now.kind != EntryKind::File || !now.is_same_object(expected) {
            return Err(replaced_error());
        }
        std::fs::remove_file(path)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;

    pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
        win::move_no_replace(from, to)
    }

    pub fn rename_verified(from: &Path, to: &Path, expected: &Identity) -> io::Result<()> {
        // Prefer renaming through the handle that was checked: the check and
        // the action then refer to one object, whatever happens to the path.
        // A handle with delete access is refused for some files (read-only
        // ones); those fall back to check, rename, check-again.
        match win::open_for_rename(from) {
            Ok(handle) => {
                let now = identity_of_handle(&handle, from)?;
                if !now.is_same_state(expected) {
                    return Err(replaced_error());
                }
                win::rename_by_handle(&handle, to)
            }
            Err(err) if err.raw_os_error() == Some(5) => {
                let before = identity_of(from)?;
                if !before.is_same_state(expected) {
                    return Err(replaced_error());
                }
                rename_noreplace(from, to)?;
                match identity_of(to) {
                    Ok(after) if after.is_same_object(expected) => Ok(()),
                    _ => {
                        let _ = rename_noreplace(to, from);
                        Err(replaced_error())
                    }
                }
            }
            Err(err) => Err(err),
        }
    }

    pub fn remove_verified(path: &Path, expected: &Identity) -> io::Result<()> {
        let handle = win::open_for_rename(path)?;
        let now = identity_of_handle(&handle, path)?;
        if now.kind != EntryKind::File || !now.is_same_object(expected) {
            return Err(replaced_error());
        }
        win::delete_by_handle(&handle)
    }

    fn identity_of_handle(handle: &std::fs::File, path: &Path) -> io::Result<Identity> {
        let meta = handle.metadata()?;
        let mut identity = identity_from(path, &meta);
        identity.file_id = win::file_id_of_handle(handle);
        Ok(identity)
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::*;

    pub fn rename_noreplace(_from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no safe move on this platform",
        ))
    }
    pub fn rename_verified(from: &Path, to: &Path, _expected: &Identity) -> io::Result<()> {
        rename_noreplace(from, to)
    }
    pub fn remove_verified(_path: &Path, _expected: &Identity) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no safe removal on this platform",
        ))
    }
}

#[cfg(windows)]
mod win {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfo, FileRenameInfo, GetFileInformationByHandle, MoveFileExW,
        SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO,
        FILE_RENAME_INFO,
    };

    const DELETE: u32 = 0x0001_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    /// `\\?\C:\...` form, so paths past `MAX_PATH` work and nothing is
    /// reinterpreted. Only absolute drive paths are rewritten.
    ///
    /// Separators are normalised on the way in. An ordinary Win32 path may use
    /// `/` — the API translates it — but a verbatim one reaches the object
    /// manager almost as written, and `/` is left alone there. A component
    /// like `.scuttle/quarantine` then becomes one name the kernel cannot
    /// resolve, and every open, rename and delete through it fails.
    ///
    /// `Path::join("a/b")` produces exactly that shape on Windows, so this is
    /// easy to hand in without noticing.
    pub(super) fn verbatim(path: &Path) -> PathBuf {
        if !path.is_absolute() {
            return path.to_path_buf();
        }
        let text = path.as_os_str().to_string_lossy();
        if text.starts_with(r"\\?\") {
            return path.to_path_buf();
        }
        let text = text.replace('/', "\\");
        if let Some(rest) = text.strip_prefix(r"\\") {
            // UNC: \\server\share -> \\?\UNC\server\share
            return PathBuf::from(format!(r"\\?\UNC\{rest}"));
        }
        PathBuf::from(format!(r"\\?\{text}"))
    }

    fn wide(path: &Path) -> Vec<u16> {
        verbatim(path)
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        // Flags = 0: no MOVEFILE_REPLACE_EXISTING, no MOVEFILE_COPY_ALLOWED.
        // An occupied destination is ERROR_ALREADY_EXISTS / ERROR_FILE_EXISTS
        // and a different volume is ERROR_NOT_SAME_DEVICE.
        let from = wide(from);
        let to = wide(to);
        // SAFETY: both buffers are NUL-terminated and outlive the call.
        let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Open without following a reparse point, sharing everything, asking only
    /// for the rights a rename or delete needs.
    pub fn open_for_rename(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .access_mode(DELETE | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_ALL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(verbatim(path))
    }

    pub fn file_id(path: &Path) -> Option<String> {
        let handle = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_ALL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(verbatim(path))
            .ok()?;
        file_id_of_handle(&handle)
    }

    pub fn file_id_of_handle(handle: &File) -> Option<String> {
        // SAFETY: zeroed is a valid BY_HANDLE_FILE_INFORMATION, and the handle
        // is open for the duration of the call.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe { GetFileInformationByHandle(handle.as_raw_handle() as HANDLE, &mut info) };
        if ok == 0 {
            return None;
        }
        Some(format!(
            "{:x}:{:x}{:08x}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }

    pub fn rename_by_handle(handle: &File, to: &Path) -> io::Result<()> {
        let name: Vec<u16> = verbatim(to).as_os_str().encode_wide().collect();
        let name_bytes = name.len() * 2;
        let header = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
        let total = header + name_bytes + 2;

        // u64 backing so the struct's alignment is satisfied.
        let mut buffer = vec![0u64; total.div_ceil(8)];
        let info = buffer.as_mut_ptr() as *mut FILE_RENAME_INFO;
        // SAFETY: the buffer is large enough for the header plus the name and
        // is aligned for FILE_RENAME_INFO; every write stays inside it.
        unsafe {
            (*info).Anonymous.ReplaceIfExists = false; // never replace
            (*info).RootDirectory = std::ptr::null_mut();
            (*info).FileNameLength = name_bytes as u32;
            let dst = (info as *mut u8).add(header) as *mut u16;
            std::ptr::copy_nonoverlapping(name.as_ptr(), dst, name.len());
            let ok = SetFileInformationByHandle(
                handle.as_raw_handle() as HANDLE,
                FileRenameInfo,
                info as *const _,
                total as u32,
            );
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    pub fn delete_by_handle(handle: &File) -> io::Result<()> {
        let info = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: `info` is a valid FILE_DISPOSITION_INFO for the call.
        let ok = unsafe {
            SetFileInformationByHandle(
                handle.as_raw_handle() as HANDLE,
                FileDispositionInfo,
                &info as *const _ as *const _,
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn publishing_never_replaces_an_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("from.txt");
        let to = tmp.path().join("to.txt");
        fs::write(&from, "new").unwrap();
        fs::write(&to, "already here").unwrap();

        let err = rename_noreplace(&from, &to).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{err}");
        assert_eq!(fs::read_to_string(&to).unwrap(), "already here");
        assert_eq!(fs::read_to_string(&from).unwrap(), "new");
    }

    #[test]
    fn publishing_never_replaces_an_existing_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("from");
        let to = tmp.path().join("to");
        fs::create_dir_all(&from).unwrap();
        fs::create_dir_all(&to).unwrap();
        fs::write(to.join("keep.txt"), "mine").unwrap();

        assert!(rename_noreplace(&from, &to).is_err());
        assert_eq!(fs::read_to_string(to.join("keep.txt")).unwrap(), "mine");
    }

    #[test]
    fn a_free_destination_is_published_normally() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("from.txt");
        let to = tmp.path().join("to.txt");
        fs::write(&from, "bytes").unwrap();
        rename_noreplace(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(&to).unwrap(), "bytes");
    }

    #[test]
    fn a_verified_move_of_an_unchanged_file_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("a.bin");
        let to = tmp.path().join("b.bin");
        fs::write(&from, "payload").unwrap();
        let identity = identity_of(&from).unwrap();

        rename_verified(&from, &to, &identity).unwrap();
        assert_eq!(fs::read_to_string(&to).unwrap(), "payload");
    }

    #[test]
    fn a_file_replaced_by_a_lookalike_is_not_moved() {
        // Same name, same size, same modification time — different object.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.bin");
        fs::write(&path, "original").unwrap();
        let identity = identity_of(&path).unwrap();
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();

        // The imposter exists alongside the original before it takes its
        // place, so it cannot be handed the original's file id back.
        let staging = tmp.path().join("staging.bin");
        fs::write(&staging, "imposter").unwrap(); // same length
        fs::File::options()
            .write(true)
            .open(&staging)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        fs::rename(&staging, &path).unwrap();

        let to = tmp.path().join("moved.bin");
        let err = rename_verified(&path, &to, &identity).unwrap_err();
        assert!(is_replaced(&err), "{err}");
        assert!(!to.exists(), "the lookalike must not have been moved");
        assert_eq!(fs::read_to_string(&path).unwrap(), "imposter");
    }

    #[test]
    fn a_file_rewritten_in_place_is_not_moved() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.bin");
        fs::write(&path, "original").unwrap();
        let identity = identity_of(&path).unwrap();
        fs::write(&path, "rewritten and longer").unwrap();

        let err = rename_verified(&path, &tmp.path().join("b.bin"), &identity).unwrap_err();
        assert!(is_replaced(&err), "{err}");
        assert!(path.exists());
    }

    #[test]
    fn identity_without_a_creation_time_still_compares_sensibly() {
        let a = Identity {
            file_id: Some("1:2".into()),
            size: 4,
            mtime_ns: Some(10),
            created_ns: None,
            kind: EntryKind::File,
        };
        let mut b = a.clone();
        b.created_ns = Some(99);
        assert!(
            a.is_same_state(&b),
            "one side lacking birth time must not refuse"
        );
        b.created_ns = Some(5);
        let mut c = a.clone();
        c.created_ns = Some(6);
        assert!(
            !b.is_same_state(&c),
            "two different birth times are a difference"
        );
    }

    #[test]
    #[cfg(windows)]
    #[test]
    fn a_verbatim_path_never_carries_a_forward_slash() {
        // The failure this prevents: every move, rename and delete refusing on
        // Windows because a path was built with `Path::join("a/b")`. An
        // ordinary Win32 path tolerates `/`; a `\\?\` one does not, and the
        // kernel sees a single component with a slash in its name.
        let made = super::win::verbatim(std::path::Path::new(r"C:\Users\x\.scuttle/quarantine"));
        let text = made.to_string_lossy();
        assert!(!text.contains('/'), "{text}");
        assert_eq!(text, r"\\?\C:\Users\x\.scuttle\quarantine");
    }

    #[cfg(windows)]
    #[test]
    fn a_verbatim_path_is_left_alone_once_it_is_one() {
        let already = std::path::Path::new(r"\\?\C:\Users\x");
        assert_eq!(super::win::verbatim(already), already);
    }

    #[cfg(windows)]
    #[test]
    fn a_unc_path_keeps_its_share_and_loses_its_slashes() {
        let made = super::win::verbatim(std::path::Path::new(r"\\server\share\a/b"));
        assert_eq!(made.to_string_lossy(), r"\\?\UNC\server\share\a\b");
    }

    #[cfg(windows)]
    #[test]
    fn a_relative_path_is_not_made_verbatim() {
        // A verbatim path must be absolute; prefixing a relative one would
        // produce something the kernel cannot resolve either.
        let rel = std::path::Path::new(r"a\b");
        assert_eq!(super::win::verbatim(rel), rel);
    }

    #[test]
    fn a_different_file_id_is_a_different_object_even_if_everything_else_matches() {
        let a = Identity {
            file_id: Some("1:2".into()),
            size: 4,
            mtime_ns: Some(10),
            created_ns: None,
            kind: EntryKind::File,
        };
        let mut b = a.clone();
        b.file_id = Some("1:3".into());
        assert!(!a.is_same_state(&b));
        assert!(!a.is_same_object(&b));
    }

    #[test]
    fn a_link_is_reported_as_a_link_not_as_what_it_points_at() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real.txt");
        fs::write(&real, "x").unwrap();
        let link = tmp.path().join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_file(&real, &link).is_err() {
            return; // symlinks need a privilege on some Windows setups
        }
        assert_eq!(identity_of(&link).unwrap().kind, EntryKind::Link);
        assert_eq!(identity_of(&real).unwrap().kind, EntryKind::File);
    }

    #[test]
    #[cfg(unix)]
    fn a_pinned_directory_reaches_its_own_children_even_if_its_path_is_swapped_for_a_link() {
        // The ancestor-swap case: after the cache root is pinned, the path that
        // led to it is replaced by a link into somewhere else. Anything reached
        // through the pin is still a child of the original directory.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cache");
        let elsewhere = tmp.path().join("elsewhere");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(root.join("a.bin"), "mine").unwrap();
        fs::write(elsewhere.join("a.bin"), "not the cache's").unwrap();

        let pinned = PinnedDir::open(&root).unwrap();
        let moved_root = tmp.path().join("cache-moved");
        fs::rename(&root, &moved_root).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &root).unwrap();

        let file = pinned.file("a.bin");
        let identity = file.identity().unwrap();
        let dest = tmp.path().join("out.bin");
        file.rename_verified(&dest, &identity).unwrap();

        assert_eq!(fs::read_to_string(&dest).unwrap(), "mine");
        assert_eq!(
            fs::read_to_string(elsewhere.join("a.bin")).unwrap(),
            "not the cache's",
            "the swapped-in path must not have been touched"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_pinned_file_that_is_replaced_is_refused_and_a_link_is_never_opened() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cache");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.bin"), "original").unwrap();
        fs::write(tmp.path().join("target.txt"), "secret").unwrap();

        let pinned = PinnedDir::open(&root).unwrap();
        let identity = pinned.file("a.bin").identity().unwrap();

        fs::remove_file(root.join("a.bin")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("target.txt"), root.join("a.bin")).unwrap();

        let file = pinned.file("a.bin");
        assert_eq!(file.identity().unwrap().kind, EntryKind::Link);
        assert!(file.open().is_err(), "O_NOFOLLOW must refuse a link");
        let err = file
            .rename_verified(&tmp.path().join("out.bin"), &identity)
            .unwrap_err();
        assert!(is_replaced(&err), "{err}");
        assert!(tmp.path().join("target.txt").exists());
    }

    #[test]
    #[cfg(unix)]
    fn a_pinned_directory_refuses_to_open_a_link_as_its_child() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cache");
        fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("sub")).unwrap();
        let pinned = PinnedDir::open(&root).unwrap();
        assert!(pinned.child_dir("sub").is_err());
        assert!(pinned.child_dir("real").is_ok());
        assert!(pinned.child_dir("../escape").is_err());
    }

    #[test]
    fn only_a_volume_crossing_counts_as_cross_device() {
        assert!(!is_cross_device(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        assert!(!is_cross_device(&io::Error::from(io::ErrorKind::NotFound)));
        #[cfg(unix)]
        assert!(is_cross_device(&io::Error::from_raw_os_error(libc::EXDEV)));
        #[cfg(windows)]
        assert!(is_cross_device(&io::Error::from_raw_os_error(17)));
    }
}
