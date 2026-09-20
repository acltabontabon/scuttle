//! Which volume something is on, and how much room a volume has.
//!
//! Only used to *plan*: whether a move will be a rename or a copy decides
//! whether progress can talk about bytes, and whether there is room for the
//! copy is worth knowing before writing anything. The rename itself is the
//! authority — if it says "different device", that is what happened,
//! whatever this guessed.

use std::path::Path;

/// The nearest ancestor of `path` that exists, so a destination that has not
/// been created yet can still be placed on a volume.
fn existing_ancestor(path: &Path) -> Option<&Path> {
    path.ancestors().find(|p| p.exists())
}

/// Do these two paths look like they are on the same volume?
pub fn same_volume_hint(a: &Path, b: &Path) -> bool {
    #[cfg(test)]
    if testing::DIFFERENT.with(|d| d.get()) {
        return false;
    }
    let (Some(a), Some(b)) = (existing_ancestor(a), existing_ancestor(b)) else {
        return true;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev(),
            _ => true,
        }
    }
    #[cfg(windows)]
    {
        use std::path::Component;
        let prefix = |p: &Path| match p.components().next() {
            Some(Component::Prefix(prefix)) => Some(prefix.as_os_str().to_ascii_lowercase()),
            _ => None,
        };
        prefix(a) == prefix(b)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (a, b);
        true
    }
}

/// Free bytes on the volume holding `path`, if it can be told.
pub fn free_bytes(path: &Path) -> Option<u64> {
    #[cfg(test)]
    if let Some(forced) = testing::FREE.with(|f| f.get()) {
        return Some(forced);
    }
    let target = existing_ancestor(path)?;
    crate::space::volume_usage(target).map(|(_, free)| free)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::cell::Cell;
    thread_local! {
        /// Pretend the volume has exactly this much room.
        pub static FREE: Cell<Option<u64>> = const { Cell::new(None) };
        /// Pretend every pair of paths is on a different volume.
        pub static DIFFERENT: Cell<bool> = const { Cell::new(false) };
    }
}
