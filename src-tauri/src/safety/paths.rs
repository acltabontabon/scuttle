//! Path arithmetic that does not lie.
//!
//! Every containment question in Scuttle goes through here. Naive string
//! prefix checks are the classic way to get this wrong — `/home/alice2` is
//! not inside `/home/alice`, and `C:\Users\Bob\..\Admin` is not inside
//! `C:\Users\Bob`. Components are compared, never characters.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// macOS (APFS/HFS+ default) and Windows compare paths case-insensitively.
/// Getting this wrong means a protected-path rule can be bypassed by
/// capitalisation.
pub const CASE_INSENSITIVE_FS: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// Fold a component for comparison on this platform.
pub fn fold(component: &OsStr) -> String {
    let s = component.to_string_lossy();
    if CASE_INSENSITIVE_FS {
        s.to_lowercase()
    } else {
        s.into_owned()
    }
}

/// Remove `.` and resolve `..` textually, without touching the filesystem.
///
/// This is deliberately *not* [`std::fs::canonicalize`]: we want to reason
/// about the path the user asked about, before deciding whether following
/// links is acceptable. A `..` that would escape the root is clamped rather
/// than allowed to walk upward past it.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // Only pop a real directory name. Never pop the prefix or the
                // root — `/..` is `/`.
                let can_pop = out
                    .components()
                    .next_back()
                    .is_some_and(|c| matches!(c, Component::Normal(_)));
                if can_pop {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Normalized, case-folded components. The comparison currency of this module.
///
/// Written to allocate once rather than building an intermediate `PathBuf`:
/// during a scan this runs for every file, and it showed up as the single
/// hottest thing in the program when it did not.
pub fn folded_components(path: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(8);
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Only pop a real directory name; `/..` is `/`.
                if out.len() > 1 || (out.len() == 1 && !is_root_token(&out[0])) {
                    out.pop();
                }
            }
            other => out.push(fold(other.as_os_str())),
        }
    }
    out
}

/// Is this folded component a filesystem root rather than a directory name?
fn is_root_token(token: &str) -> bool {
    token == "/" || token == "\\" || token.ends_with(':') || token.starts_with("\\\\")
}

/// Is `child` at or below `ancestor`?
///
/// Equal paths count as contained: a rule protecting `~/.ssh` protects
/// `~/.ssh` itself, not merely its contents.
pub fn is_within(child: &Path, ancestor: &Path) -> bool {
    let c = folded_components(child);
    let a = folded_components(ancestor);
    if a.is_empty() || c.len() < a.len() {
        return false;
    }
    c[..a.len()] == a[..]
}

/// Is `child` strictly below `ancestor`?
pub fn is_strictly_within(child: &Path, ancestor: &Path) -> bool {
    is_within(child, ancestor) && folded_components(child).len() > folded_components(ancestor).len()
}

/// Does any component of the path equal `name` (case-folded)?
pub fn has_component(path: &Path, name: &str) -> bool {
    let needle = if CASE_INSENSITIVE_FS {
        name.to_lowercase()
    } else {
        name.to_string()
    };
    folded_components(path).contains(&needle)
}

/// Lowercased file extension, without the dot.
pub fn extension(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_lowercase())
}

/// Lowercased file name.
pub fn file_name_lower(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Walk the path downward from `base`, checking whether any component below
/// it is a symlink, junction or other reparse point.
///
/// Scoping this to `base` matters: on macOS `/var` and `/tmp` *are* symlinks,
/// and on Windows `C:\Users\Public\Documents` is a junction. Links above the
/// area Scuttle was asked to look at are the operating system's business.
/// Links *below* it are the dangerous kind — the thing we validated and the
/// thing we would delete may not be the same object.
///
/// Returns `None` when `path` is not below `base`; containment is checked
/// separately and reported with a clearer message.
pub fn first_link_below(path: &Path, base: &Path) -> Option<PathBuf> {
    let normalized = normalize(path);
    if !is_within(&normalized, base) {
        return None;
    }
    let base_depth = folded_components(base).len();
    let mut prefix = PathBuf::new();
    for (index, comp) in normalized.components().enumerate() {
        prefix.push(comp.as_os_str());
        if index < base_depth {
            continue;
        }
        match std::fs::symlink_metadata(&prefix) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Some(prefix);
                }
                #[cfg(windows)]
                if is_reparse_point(&meta) {
                    return Some(prefix);
                }
            }
            // A missing or unreadable prefix is not a link; other checks will
            // reject the path for their own reasons.
            Err(_) => return None,
        }
    }
    None
}

#[cfg(windows)]
fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// How deep below a root a path sits. Used to refuse actions on paths that are
/// suspiciously close to a volume or home root.
pub fn depth_below(path: &Path, root: &Path) -> Option<usize> {
    if !is_within(path, root) {
        return None;
    }
    Some(folded_components(path).len() - folded_components(root).len())
}

/// macOS bundles (`.app`, `.photoslibrary`, ...) are directories that behave
/// like single files. Scuttle treats them as opaque: never traversed, never
/// partially removed.
pub fn is_opaque_bundle(path: &Path) -> bool {
    const BUNDLE_EXTENSIONS: [&str; 14] = [
        "app",
        "photoslibrary",
        "photolibrary",
        "aplibrary",
        "fcpbundle",
        "sparsebundle",
        "logicx",
        "band",
        "rtfd",
        "framework",
        "bundle",
        "imovielibrary",
        "tvlibrary",
        "xcappdata",
    ];
    extension(path).is_some_and(|e| BUNDLE_EXTENSIONS.contains(&e.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn sibling_directories_are_not_contained() {
        // The bug that string-prefix checks always have.
        assert!(!is_within(&p("/home/alice2/x"), &p("/home/alice")));
        assert!(!is_within(&p("/home/alicent"), &p("/home/alice")));
        assert!(is_within(&p("/home/alice/x"), &p("/home/alice")));
    }

    #[test]
    fn a_path_contains_itself() {
        assert!(is_within(&p("/home/alice/.ssh"), &p("/home/alice/.ssh")));
        assert!(!is_strictly_within(
            &p("/home/alice/.ssh"),
            &p("/home/alice/.ssh")
        ));
    }

    #[test]
    fn dotdot_cannot_escape_a_protected_ancestor() {
        // Looks like it is inside Downloads; is actually the SSH directory.
        let sneaky = p("/home/alice/Downloads/../.ssh/id_ed25519");
        assert!(!is_within(&sneaky, &p("/home/alice/Downloads")));
        assert!(is_within(&sneaky, &p("/home/alice/.ssh")));
    }

    #[test]
    fn dotdot_never_walks_above_the_root() {
        assert_eq!(normalize(&p("/../../../etc")), p("/etc"));
        assert_eq!(normalize(&p("/a/../../b")), p("/b"));
    }

    #[test]
    fn curdir_noise_is_removed() {
        assert_eq!(normalize(&p("/a/./b/./c")), p("/a/b/c"));
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn case_does_not_defeat_a_rule_on_case_insensitive_filesystems() {
        assert!(is_within(
            &p("/Users/Alice/.SSH/id_rsa"),
            &p("/Users/alice/.ssh")
        ));
        assert!(has_component(
            &p("/Users/alice/Library/KEYCHAINS"),
            "keychains"
        ));
    }

    #[test]
    fn depth_below_counts_components() {
        assert_eq!(
            depth_below(&p("/home/alice/a/b"), &p("/home/alice")),
            Some(2)
        );
        assert_eq!(depth_below(&p("/home/alice"), &p("/home/alice")), Some(0));
        assert_eq!(depth_below(&p("/elsewhere"), &p("/home/alice")), None);
    }

    #[test]
    fn bundles_are_recognised_as_opaque() {
        assert!(is_opaque_bundle(&p("/Applications/Safari.app")));
        assert!(is_opaque_bundle(&p(
            "/Users/a/Pictures/Photos Library.photoslibrary"
        )));
        assert!(!is_opaque_bundle(&p("/Users/a/Downloads/thing.zip")));
    }

    #[test]
    fn extension_and_name_are_lowercased() {
        assert_eq!(extension(&p("/a/B/Thing.DMG")).as_deref(), Some("dmg"));
        assert_eq!(file_name_lower(&p("/a/Screenshot.PNG")), "screenshot.png");
    }

    #[test]
    fn a_symlinked_ancestor_below_the_base_is_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let real = base.join("real");
        std::fs::create_dir_all(real.join("inner")).unwrap();
        let link = base.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real, &link).unwrap();

        assert_eq!(
            first_link_below(&link.join("inner"), base).as_deref(),
            Some(link.as_path())
        );
        assert_eq!(first_link_below(&real.join("inner"), base), None);
    }

    #[test]
    fn links_above_the_base_are_not_our_problem() {
        // On macOS the temp directory itself lives under the /var symlink.
        // Scanning below it must not be treated as unsafe.
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp.path().join("a/b");
        std::fs::create_dir_all(&inner).unwrap();
        assert_eq!(first_link_below(&inner, tmp.path()), None);
    }

    #[test]
    fn a_path_outside_the_base_reports_no_link() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            first_link_below(Path::new("/elsewhere/x"), tmp.path()),
            None
        );
    }
}
